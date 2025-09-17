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

async fn handle_oauth_login(provider: &str) -> Result<()> {
    let settings = Settings::load().await?;
    let (tx, rx) = oneshot::channel::<String>();

    let server_handle = tokio::spawn(async move {
        let listener = match TcpListener::bind(format!("127.0.0.1:{LOCAL_PORT}")).await {
            Ok(l) => l,
            Err(_) => {
                let _ = tx.send("error:port".to_string());
                return;
            }
        };
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buffer = [0; 2048];
            if stream.read(&mut buffer).await.is_ok() {
                let request_str = String::from_utf8_lossy(&buffer[..]);
                if let Some(token) = parse_token_from_request(&request_str) {
                    let _ = tx.send(token);
                    let html_content = include_str!("../assets/success.html");
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
                        html_content.len(),
                        html_content
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
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
    let token = tokio::time::timeout(Duration::from_secs(120), rx)
        .await
        .context("Login timed out. Please try again.")??;
    server_handle.abort();

    if token == "error:port" {
        bail!(
            "Could not start local server on port {}. Is another process using it?",
            LOCAL_PORT
        );
    }
    if token.is_empty() {
        bail!("Local server failed to start or did not receive token. Cannot complete login.");
    }
    api::login(token).await?;
    println!("{}", "✔ Login successful!".green().bold());

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

    if response.status().is_success() {
        let body: serde_json::Value = response.json().await?;

        if let Some(true) = body.get("2fa_required").and_then(|v| v.as_bool()) {
            if let Some(two_fa_token) = body.get("2fa_token").and_then(|v| v.as_str()) {
                return handle_2fa_authentication(two_fa_token).await;
            } else {
                bail!("2FA is required but no token was provided.");
            }
        }

        if let Some(token) = body["token"].as_str() {
            api::login(token.to_string()).await?;
            println!("{}", "✔ Login successful!".green().bold());
            return Ok(());
        }

        bail!("Received an unexpected response from the server.");
    } else {
        let error_msg = response
            .text()
            .await
            .unwrap_or_else(|_| "Invalid credentials.".to_string());

        if error_msg.contains("verify your email") {
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
    let code = Text::new("› Enter your 2FA code from your authenticator app:")
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
        let body: serde_json::Value = response.json().await?;
        if let Some(token) = body["token"].as_str() {
            api::login(token.to_string()).await?;
            println!("{}", "✔ 2FA authentication successful!".green().bold());
            Ok(())
        } else {
            bail!("2FA authentication failed: Received an unexpected response from the server.");
        }
    } else {
        let error_msg = response
            .text()
            .await
            .unwrap_or_else(|_| "Invalid 2FA code.".to_string());
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

    let error_msg = response
        .text()
        .await
        .unwrap_or_else(|_| "Registration failed.".to_string());
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

    let client = api::get_api_client().await?;
    let response = client
        .post(format!("{}/auth/2fa/setup", settings.api_url))
        .send()
        .await?;

    if !response.status().is_success() {
        bail!("Failed to initialize 2FA setup. Are you logged in?");
    }

    let setup_data: Value = response.json().await?;
    let secret = setup_data["secret"]
        .as_str()
        .context("No secret received")?;
    let qr_code_base64 = setup_data["qr_code_base64"]
        .as_str()
        .context("No QR code received")?;

    println!("{}", "○ Two-Factor Authentication Setup".cyan().bold());
    println!("1. Install a 2FA app like Google Authenticator or Authy");
    println!("2. Scan the QR code below OR manually enter this secret:");
    println!("   {}", secret.green());
    println!("\n3. QR Code (if your terminal supports it):");
    println!("{qr_code_base64}");

    println!("\n4. After adding to your app, enter a code to verify:");
    let code = Text::new("› Enter verification code from your 2FA app:")
        .prompt()?
        .trim()
        .to_string();

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
            for (i, code) in recovery_codes.iter().enumerate() {
                if let Some(code_str) = code.as_str() {
                    println!("{}. {}", i + 1, code_str.cyan());
                }
            }
            println!(
                "\nThese codes can be used to access your account if you lose your 2FA device."
            );
        }
    } else {
        let error_msg = verify_response
            .text()
            .await
            .unwrap_or_else(|_| "Verification failed.".to_string());
        bail!("2FA setup failed: {}", error_msg);
    }

    Ok(())
}

fn parse_token_from_request(request: &str) -> Option<String> {
    let first_line = request.lines().next()?;
    if !first_line.contains("/auth/callback") {
        return None;
    }
    let path_and_query = first_line.split_whitespace().nth(1)?;
    let query_string = path_and_query.split('?').nth(1)?;
    let token_param = query_string.split('&').find(|p| p.starts_with("token="))?;
    token_param.strip_prefix("token=").map(String::from)
}
