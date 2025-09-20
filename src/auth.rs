use anyhow::{Context, Result, bail};
use colored::*;
use inquire::{Confirm, Password, PasswordDisplayMode, Select, Text};
use serde_json::Value;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use wayclip_core::api;
use wayclip_core::settings::Settings;

const LOCAL_PORT: u16 = 54321;

enum AuthCallbackResult {
    Success(String),
    TwoFactor(String),
    Error(String),
}

fn parse_token_from_header(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .and_then(|header| header.to_str().ok())
        .and_then(|header_str| {
            header_str.split(';').find_map(|part| {
                let mut key_val = part.trim().splitn(2, '=');
                if key_val.next() == Some("token") {
                    key_val.next().map(String::from)
                } else {
                    None
                }
            })
        })
}

async fn handle_oauth_login(provider: &str) -> Result<()> {
    let settings = Settings::load().await?;
    let (tx, rx) = oneshot::channel::<AuthCallbackResult>();

    let server_handle = tokio::spawn(async move {
        let listener = match TcpListener::bind(format!("127.0.0.1:{LOCAL_PORT}")).await {
            Ok(l) => l,
            Err(_) => {
                let _ = tx.send(AuthCallbackResult::Error("port".to_string()));
                return;
            }
        };
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buffer = [0; 2048];
            if stream.read(&mut buffer).await.is_ok() {
                let request_str = String::from_utf8_lossy(&buffer[..]);
                let callback_result = parse_token_from_request(&request_str);

                let html_content = include_str!("../assets/success.html");
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
                    html_content.len(),
                    html_content
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;

                if let Some(result) = callback_result {
                    let _ = tx.send(result);
                }
            }
        }
    });

    let redirect_uri = format!("http://127.0.0.1:{LOCAL_PORT}/auth/callback");
    let login_url = format!(
        "{}/auth/{}?client=cli&redirect_uri={}",
        settings.api_url,
        provider,
        urlencoding::encode(&redirect_uri)
    );

    println!("{}", "○ Opening your browser to complete login...".cyan());
    if opener::open(&login_url).is_err() {
        println!("Could not open browser automatically.");
        println!("Please visit this URL to log in:\n{login_url}");
    }

    println!("{}", "◌ Waiting for authentication...".yellow());
    let result = tokio::time::timeout(Duration::from_secs(120), rx)
        .await
        .context("Login timed out. Please try again.")??;
    server_handle.abort();

    match result {
        AuthCallbackResult::Error(reason) if reason == "port" => {
            bail!(
                "Could not start local server on port {}. Is another process using it?",
                LOCAL_PORT
            );
        }
        AuthCallbackResult::Error(e) => {
            bail!("Login failed: {}", e);
        }
        AuthCallbackResult::Success(token) => {
            api::login(token).await?;
            println!("{}", "✔ Login successful!".green().bold());
        }
        AuthCallbackResult::TwoFactor(two_fa_token) => {
            return handle_2fa_authentication(&two_fa_token).await;
        }
    }

    Ok(())
}

async fn handle_password_login() -> Result<()> {
    let settings = Settings::load().await?;
    let email = Text::new("› Enter your email:")
        .prompt()?
        .trim()
        .to_string();
    if email.is_empty() {
        bail!("Email cannot be empty.");
    }

    let password = Password::new("› Enter your password:")
        .with_display_mode(PasswordDisplayMode::Masked)
        .prompt()?;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/auth/login", settings.api_url))
        .json(&serde_json::json!({
            "email": email,
            "password": password,
        }))
        .send()
        .await?;

    let status = response.status();
    let response_body_text = response.text().await?;

    if status.is_success() {
        let body: serde_json::Value = serde_json::from_str(&response_body_text)?;

        if body.get("2fa_required").and_then(|v| v.as_bool()).is_some() {
            if let Some(two_fa_token) = body.get("2fa_token").and_then(|v| v.as_str()) {
                return handle_2fa_authentication(two_fa_token).await;
            } else {
                bail!("2FA is required but no token was provided by the server.");
            }
        }

        let temp_response = reqwest::Response::from(
            http::Response::builder()
                .status(status)
                .body(response_body_text.clone())?,
        );

        let token = parse_token_from_header(&temp_response)
            .context("Login token not found in server response.")?;

        api::login(token).await?;
        println!("{}", "✔ Login successful!".green().bold());
        return Ok(());
    } else {
        let error_body: serde_json::Value =
            serde_json::from_str(&response_body_text).unwrap_or_default();
        let error_msg = error_body["message"]
            .as_str()
            .unwrap_or("Invalid credentials.");

        if error_body["error_code"].as_str() == Some("EMAIL_NOT_VERIFIED") {
            let resend = Confirm::new(
                "Your email is not verified. Would you like to resend the verification email?",
            )
            .with_default(true)
            .prompt()?;

            if resend {
                handle_resend_verification(&email).await?;
            }
            bail!("Please verify your email before logging in.");
        }

        bail!("Login failed: {}", error_msg);
    }
}

async fn handle_2fa_authentication(two_fa_token: &str) -> Result<()> {
    let settings = Settings::load().await?;

    println!("{}", "○ Two-Factor Authentication Required".yellow().bold());
    let code = Text::new("› Enter your 2FA code or a recovery code:")
        .prompt()?
        .trim()
        .to_string();
    if code.is_empty() {
        bail!("2FA code cannot be empty.");
    }

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/auth/2fa/authenticate", settings.api_url))
        .json(&serde_json::json!({
            "2fa_token": two_fa_token,
            "code": code,
        }))
        .send()
        .await?;

    if response.status().is_success() {
        let token =
            parse_token_from_header(&response).context("2FA token not found in response.")?;
        api::login(token).await?;
        println!("{}", "✔ 2FA authentication successful!".green().bold());
        Ok(())
    } else {
        let error_body: serde_json::Value = response.json().await.unwrap_or_default();
        let error_msg = error_body["message"]
            .as_str()
            .unwrap_or("Invalid 2FA code.");
        bail!("2FA authentication failed: {}", error_msg);
    }
}

async fn handle_register() -> Result<()> {
    let settings = Settings::load().await?;

    println!("{}", "○ Create a new account".cyan().bold());
    let username = Text::new("› Enter your username:")
        .prompt()?
        .trim()
        .to_string();
    if username.is_empty() {
        bail!("Username cannot be empty.");
    }

    let email = Text::new("› Enter your email:")
        .prompt()?
        .trim()
        .to_string();
    if email.is_empty() {
        bail!("Email cannot be empty.");
    }

    let password = Password::new("› Enter your password:")
        .with_display_mode(PasswordDisplayMode::Masked)
        .prompt()?;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/auth/register", settings.api_url))
        .json(&serde_json::json!({
            "username": username,
            "email": email,
            "password": password,
        }))
        .send()
        .await?;

    if response.status().is_success() {
        println!("{}", "✔ Registration successful!".green().bold());
        println!(
            "{}",
            "◌ Please check your email to verify your account before logging in.".yellow()
        );
        return Ok(());
    }

    let error_body: serde_json::Value = response.json().await.unwrap_or_default();
    let error_msg = error_body["message"]
        .as_str()
        .unwrap_or("Registration failed.");
    bail!("Registration failed: {}", error_msg);
}

async fn handle_resend_verification(email: &str) -> Result<()> {
    let settings = Settings::load().await?;

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/auth/resend-verification", settings.api_url))
        .json(&serde_json::json!({
            "email": email,
        }))
        .send()
        .await?;

    if response.status().is_success() {
        println!(
            "{}",
            "✔ Verification email sent (if account exists).".green()
        );
    } else {
        println!("{}", "✗ Could not send verification email.".yellow());
    }

    Ok(())
}

pub async fn handle_login() -> Result<()> {
    let options = vec![
        "GitHub",
        "Google",
        "Discord",
        "Email/Password",
        "Register new account",
    ];
    let choice = Select::new("How would you like to proceed?", options).prompt()?;

    match choice {
        "GitHub" | "Google" | "Discord" => {
            handle_oauth_login(&choice.to_lowercase()).await?;
        }
        "Email/Password" => {
            handle_password_login().await?;
        }
        "Register new account" => {
            handle_register().await?;
        }
        _ => unreachable!(),
    }

    Ok(())
}

pub async fn handle_logout() -> Result<()> {
    api::logout().await?;
    println!("{}", "✔ You have been logged out.".green());
    Ok(())
}

pub async fn handle_2fa_setup() -> Result<()> {
    let settings = Settings::load().await?;

    println!("{}", "◌ Contacting the server to set up 2FA...".yellow());
    let client = api::get_api_client().await?;
    let response = client
        .post(format!("{}/auth/2fa/setup", settings.api_url))
        .send()
        .await?;

    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        bail!(
            "Failed to initialize 2FA setup. Are you logged in? Server response: {}",
            error_text
        );
    }

    let setup_data: Value = response.json().await?;
    let secret = setup_data["secret"]
        .as_str()
        .context("No secret key received from the server. The API response may have changed.")?;

    println!("{}", "\n○ Two-Factor Authentication Setup".cyan().bold());
    println!("1. Open an authenticator app (like Google Authenticator, Authy, or 1Password).");
    println!("2. Choose to add a new account via manual entry or secret key.");
    println!("3. Enter the following secret key when prompted:");
    println!("\n   {}", secret.green().bold());
    println!("\n4. After adding the account, your app will generate a 6-digit code.");

    let code = Text::new("› Enter the code from your app to verify and complete the setup:")
        .prompt()?
        .trim()
        .to_string();

    if code.is_empty() {
        bail!("Verification code cannot be empty. Setup cancelled.");
    }

    println!("{}", "◌ Verifying code with the server...".yellow());
    let verify_response = client
        .post(format!("{}/auth/2fa/verify", settings.api_url))
        .json(&serde_json::json!({
            "secret": secret,
            "code": code,
        }))
        .send()
        .await?;

    if verify_response.status().is_success() {
        let verify_data: Value = verify_response.json().await?;
        println!("{}", "✔ 2FA enabled successfully!".green().bold());

        if let Some(recovery_codes) = verify_data["recovery_codes"].as_array() {
            println!(
                "\n{}",
                "IMPORTANT: Save these recovery codes in a safe place:"
                    .yellow()
                    .bold()
            );
            println!("These can be used to access your account if you lose your 2FA device.");
            for (i, code) in recovery_codes.iter().enumerate() {
                if let Some(code_str) = code.as_str() {
                    println!("  {}. {}", i + 1, code_str.cyan());
                }
            }
        }
    } else {
        let error_body: Value = verify_response.json().await.unwrap_or_default();
        let error_msg = error_body["message"]
            .as_str()
            .unwrap_or("Verification failed.");
        bail!("2FA setup failed: {error_msg}");
    }

    Ok(())
}

fn parse_token_from_request(request: &str) -> Option<AuthCallbackResult> {
    let first_line = request.lines().next()?;
    if !first_line.contains("/auth/callback") {
        return None;
    }
    let path_and_query = first_line.split_whitespace().nth(1)?;
    let query_string = path_and_query.split('?').nth(1)?;

    for param in query_string.split('&') {
        if let Some(token) = param.strip_prefix("token=") {
            return Some(AuthCallbackResult::Success(token.to_string()));
        }
        if let Some(two_fa_token) = param.strip_prefix("2fa_token=") {
            return Some(AuthCallbackResult::TwoFactor(two_fa_token.to_string()));
        }
    }
    None
}
