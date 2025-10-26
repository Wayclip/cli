use crate::auth::{handle_2fa_setup, handle_login, handle_logout};
use crate::list::handle_list;
use crate::manage::handle_manage;
use anyhow::{Context, Result, bail};
use arboard::Clipboard;
use clap::{Parser, Subcommand};
use colored::*;
use inquire::{Confirm, Select, Text};
use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use uuid::Uuid;
use wayclip_core::control::DaemonManager;
use wayclip_core::{api, delete_file, gather_unified_clips, rename_all_entries, update_liked};
use wayclip_core::{models::UnifiedClipData, settings::Settings};
use which::which;

pub mod auth;
pub mod list;
pub mod manage;

#[derive(Parser)]
#[command(
    name = "wayclip-cli",
    version,
    about = "Capture and replay your screen instantly on Linux. Built for the modern desktop with Wayland and PipeWire."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    #[arg(long, hide = true)]
    debug: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    Daemon {
        #[command(subcommand)]
        action: DaemonCommand,
    },
    Save,
    List {
        #[arg(short = 't', long = "timestamp")]
        timestamp: bool,
        #[arg(short = 'l', long = "length")]
        length: bool,
        #[arg(short = 'r', long = "reverse")]
        reverse: bool,
        #[arg(short = 's', long = "size")]
        size: bool,
        #[arg(short = 'e', long = "extra")]
        extra: bool,
    },
    Manage,
    Config {
        #[arg(short = 'e', long = "editor")]
        editor: Option<String>,
    },
    View {
        name: String,
        #[arg(short = 'p', long = "player")]
        player: Option<String>,
    },
    Delete {
        name: String,
    },
    Rename {
        name: String,
    },
    Edit {
        name: String,
        start_time: String,
        end_time: String,
        #[arg(default_value_t = false)]
        disable_audio: bool,
    },
    Login,
    Logout,
    Me,
    #[command(name = "2fa")]
    TwoFactorAuth {
        #[command(subcommand)]
        action: TwoFactorCommand,
    },
    Share {
        #[arg(help = "Name of the clip to share")]
        name: String,
    },
    Like {
        #[arg(help = "Name of the local clip to like/unlike")]
        name: String,
    },
    Url {
        #[arg(help = "Name of the hosted clip to get the URL for")]
        name: String,
    },
    Open {
        #[arg(help = "Name of the hosted clip to open in a browser")]
        name: String,
    },
}

#[derive(Subcommand)]
pub enum AutostartAction {
    On,
    Off,
}

#[derive(Subcommand)]
pub enum DaemonCommand {
    Start,
    Stop,
    Restart,
    Status,
    Logs,
    Autostart {
        #[command(subcommand)]
        action: AutostartAction,
    },
}

#[derive(Subcommand)]
pub enum TwoFactorCommand {
    Setup,
    Status,
}

pub async fn copy_to_clipboard(text: &str) -> Result<()> {
    if env::var("WAYLAND_DISPLAY").is_ok() {
        if let Ok(mut process) = Command::new("wl-copy")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(mut stdin) = process.stdin.take() {
                if stdin.write_all(text.as_bytes()).await.is_ok() {
                    drop(stdin);
                    if process.wait().await.is_ok() {
                        return Ok(());
                    }
                }
            }
        }
    }

    if env::var("DISPLAY").is_ok() {
        if let Ok(mut process) = Command::new("xclip")
            .arg("-selection")
            .arg("clipboard")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(mut stdin) = process.stdin.take() {
                if stdin.write_all(text.as_bytes()).await.is_ok() {
                    drop(stdin);
                    if process.wait().await.is_ok() {
                        return Ok(());
                    }
                }
            }
        }
    }

    let text_owned = text.to_string();
    match tokio::task::spawn_blocking(move || -> Result<(), arboard::Error> {
        let mut clipboard = Clipboard::new()?;
        clipboard.set_text(text_owned)
    })
    .await
    {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => bail!(
            "Could not access clipboard.\n\
             - Please install 'wl-clipboard' (Wayland) or 'xclip' (X11).\n\
             - arboard error: {e:#}",
        ),
        Err(e) => bail!("Clipboard task failed: {e:#}"),
    }
}

async fn handle_autostart_on() -> Result<()> {
    println!("○ Enabling autostart using systemd user service...");

    let daemon_path = which("wayclip-daemon")
        .context("Could not find 'wayclip-daemon' executable in your PATH. Please ensure it is installed correctly.")?;
    println!("  Daemon found at: {}", daemon_path.display());

    let service_content = format!(
        r#"[Unit]
Description=Wayclip Daemon
After=graphical.target pipewire.service pipewire-pulse.service
Wants=graphical.target

[Service]
ExecStart={}
Restart=always
RestartSec=3
Type=notify
TimeoutStartSec=90

[Install]
WantedBy=default.target
"#,
        daemon_path.to_str().unwrap()
    );

    let config_dir = Settings::config_path();

    tokio::fs::create_dir_all(&config_dir)
        .await
        .context(format!(
            "Failed to create systemd user directory at {}",
            config_dir.display()
        ))?;

    let service_path = config_dir.join("wayclip-daemon.service");

    if service_path.exists() {
        let overwrite = Confirm::new("Service file already exists. Overwrite?")
            .with_default(false)
            .prompt()?;
        if !overwrite {
            println!("{}", "○ Autostart setup cancelled.".yellow());
            return Ok(());
        }
    }

    tokio::fs::write(&service_path, service_content)
        .await
        .context(format!(
            "Failed to write service file to {}",
            service_path.display()
        ))?;
    println!("✔ Service file created at {}", service_path.display());

    println!("○ Reloading systemd user daemon...");
    let reload_status = Command::new("systemctl")
        .arg("--user")
        .arg("daemon-reload")
        .status()
        .await
        .context("Failed to execute 'systemctl --user daemon-reload'. Is systemd running?")?;

    if !reload_status.success() {
        bail!("'systemctl --user daemon-reload' failed. Please run it manually.");
    }

    println!("○ Enabling and starting the service...");
    let enable_output = Command::new("systemctl")
        .arg("--user")
        .arg("enable")
        .arg("--now")
        .arg("wayclip-daemon.service")
        .output()
        .await
        .context("Failed to execute 'systemctl --user enable --now'.")?;

    if !enable_output.status.success() {
        let stderr = String::from_utf8_lossy(&enable_output.stderr);
        bail!("'systemctl --user enable --now' failed. Please run it manually.\nError: {stderr}",);
    }

    println!("{}", "✔ Autostart enabled successfully!".green().bold());
    println!("  The Wayclip daemon will now start automatically when you log in.");
    println!(
        "  To disable it, run: {}",
        "wayclip daemon autostart off".italic()
    );

    Ok(())
}

async fn handle_autostart_off() -> Result<()> {
    println!("○ Disabling autostart using systemd user service...");

    let service_name = "wayclip-daemon.service";

    let config_dir = Settings::config_path();
    let service_path = config_dir.join(service_name);

    if !service_path.exists() {
        println!(
            "{}",
            "○ Autostart is already disabled (service file not found).".yellow()
        );
        return Ok(());
    }

    println!("○ Disabling and stopping the service...");
    let disable_output = Command::new("systemctl")
        .arg("--user")
        .arg("disable")
        .arg("--now")
        .arg(service_name)
        .output()
        .await
        .context("Failed to execute 'systemctl --user disable --now'.")?;

    if !disable_output.status.success() {
        let stderr = String::from_utf8_lossy(&disable_output.stderr);
        if !stderr.contains("unit file does not exist")
            && !stderr.contains("No such file or directory")
        {
            bail!("'systemctl --user disable --now' failed.\nError: {stderr}",);
        }
    }

    println!("{}", "✔ Autostart disabled successfully!".green().bold());

    let delete_file = Confirm::new("Do you want to remove the systemd service file?")
        .with_default(false)
        .prompt()?;

    if delete_file {
        tokio::fs::remove_file(&service_path)
            .await
            .context(format!(
                "Failed to remove service file at {}",
                service_path.display()
            ))?;
        println!("✔ Service file removed.");
        Command::new("systemctl")
            .arg("--user")
            .arg("daemon-reload")
            .status()
            .await?;
    }

    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(e) = run().await {
        eprintln!("{} {:#}", "✗ Error:".red().bold(), e);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    if cli.debug {
        println!("{}", "○ Debug mode is ON".yellow());
    }

    match &cli.command {
        Commands::Login => handle_login().await?,
        Commands::Logout => handle_logout().await?,
        Commands::Me => handle_me().await?,
        Commands::TwoFactorAuth { action } => match action {
            TwoFactorCommand::Setup => handle_2fa_setup().await?,
            TwoFactorCommand::Status => handle_2fa_status().await?,
        },
        Commands::Share { name } => handle_share(name).await?,
        Commands::Save => handle_save().await?,
        Commands::List { .. } => handle_list(&cli.command).await?,
        Commands::Manage => handle_manage().await?,
        Commands::Config { editor } => handle_config(editor.as_deref()).await?,
        Commands::View { name, player } => handle_view(name, player.as_deref()).await?,
        Commands::Rename { name } => handle_rename(name).await?,
        Commands::Delete { name } => handle_delete(name).await?,
        Commands::Edit {
            name,
            start_time,
            end_time,
            disable_audio,
        } => handle_edit(name, start_time, end_time, disable_audio).await?,
        Commands::Like { name } => handle_like(name).await?,
        Commands::Url { name } => handle_url(name).await?,
        Commands::Open { name } => handle_open(name).await?,
        Commands::Daemon { action } => {
            let manager = DaemonManager::new();
            match action {
                DaemonCommand::Start => manager.start().await?,
                DaemonCommand::Stop => manager.stop().await?,
                DaemonCommand::Restart => manager.restart().await?,
                DaemonCommand::Logs => manager.logs().await?,
                DaemonCommand::Status => {
                    manager.status().await?;
                }
                DaemonCommand::Autostart { action } => match action {
                    AutostartAction::On => handle_autostart_on().await?,
                    AutostartAction::Off => handle_autostart_off().await?,
                },
            }
        }
    }

    Ok(())
}

async fn find_unified_clip(name_input: &str) -> Result<UnifiedClipData> {
    let trimmed_name = name_input.trim();

    let name_stem = if trimmed_name.to_lowercase().ends_with(".mp4") {
        &trimmed_name[..trimmed_name.len() - 4]
    } else {
        trimmed_name
    };

    if name_stem.is_empty() {
        bail!("Clip name cannot be empty.");
    }

    let all_clips = gather_unified_clips().await?;
    all_clips
        .into_iter()
        .find(|clip| clip.name.eq_ignore_ascii_case(name_stem))
        .context(format!("Clip '{}' not found.", name_stem))
}

fn sanitize_and_validate_filename_stem(new_name_input: &str) -> Result<String> {
    let trimmed = new_name_input.trim();
    if trimmed.is_empty() {
        bail!("New name cannot be empty.");
    }
    let sanitized = trimmed.replace(' ', "_");

    if sanitized
        .chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
    {
        bail!("New name contains invalid characters (< > : \" / \\ | ? *).");
    }
    Ok(sanitized)
}

fn validate_ffmpeg_time(time_str: &str) -> Result<String> {
    let trimmed = time_str.trim();
    if trimmed.parse::<f64>().is_ok() {
        return Ok(trimmed.to_string());
    }
    let parts: Vec<&str> = trimmed.split(':').collect();
    if parts.len() > 3 || parts.is_empty() {
        bail!(
            "Invalid time format '{}'. Use seconds (e.g., 5.5) or HH:MM:SS format.",
            time_str
        );
    }
    if parts
        .iter()
        .all(|p| !p.is_empty() && p.parse::<f64>().is_ok())
    {
        Ok(trimmed.to_string())
    } else {
        bail!(
            "Invalid time format '{}'. Use seconds (e.g., 5.5) or HH:MM:SS format.",
            time_str
        );
    }
}

async fn handle_like(name: &str) -> Result<()> {
    let clip = find_unified_clip(name).await?;

    if let (Some(local_data), Some(_)) = (&clip.local_data, &clip.local_path) {
        let new_liked_status = !local_data.liked;
        match update_liked(&clip.full_filename, new_liked_status).await {
            Ok(_) => {
                let status = if new_liked_status { "Liked" } else { "Unliked" };
                println!("✔ Clip '{}' has been {}.", clip.name.cyan(), status.green());
            }
            Err(e) => bail!("Failed to update liked status: {}", e),
        }
    } else {
        bail!(
            "Clip '{}' does not exist locally and cannot be liked/unliked.",
            clip.name
        );
    }
    Ok(())
}

async fn handle_url(name: &str) -> Result<()> {
    let clip = find_unified_clip(name).await?;
    let settings = Settings::load().await?;

    if let Some(id) = clip.hosted_id {
        let public_url = format!("{}/clip/{}", settings.api_url, id);
        println!("  {}", public_url.underline());
        match copy_to_clipboard(&public_url).await {
            Ok(_) => println!("{}", "✔ Public URL copied to clipboard!".green()),
            Err(e) => println!(
                "{}",
                format!("✗ Could not copy URL to clipboard: {:#}", e).yellow()
            ),
        }
    } else {
        bail!(
            "'{}' is not a hosted clip and does not have a public URL.",
            clip.name
        );
    }
    Ok(())
}

async fn handle_open(name: &str) -> Result<()> {
    let clip = find_unified_clip(name).await?;
    let settings = Settings::load().await?;

    if let Some(id) = clip.hosted_id {
        let public_url = format!("{}/clip/{}", settings.api_url, id);
        println!("○ Opening URL in browser: {}", public_url.cyan());
        opener::open(&public_url).context("Failed to open URL in browser.")?;
    } else {
        bail!(
            "'{}' is not a hosted clip and does not have a public URL.",
            clip.name
        );
    }
    Ok(())
}

async fn handle_me() -> Result<()> {
    match api::get_current_user().await {
        Ok(profile) => {
            let usage_gb = profile.storage_used as f64 / 1_073_741_824.0;
            let limit_gb = profile.storage_limit as f64 / 1_073_741_824.0;
            let percentage = if profile.storage_limit > 0 {
                (profile.storage_used as f64 / profile.storage_limit as f64) * 100.0
            } else {
                0.0
            };

            println!("{}", "┌─ Your Profile & Status ─────────".bold());
            println!("│ {} {}", "Username:".cyan(), profile.user.username);
            if let Some(email) = &profile.user.email {
                let verified = if profile.user.email_verified_at.is_some() {
                    "✔ Verified".green()
                } else {
                    "⚠ Not verified".yellow()
                };
                println!("│ {} {} ({})", "Email:".cyan(), email, verified);
            }
            println!(
                "│ {} {}",
                "Tier:".cyan(),
                format!("{:?}", profile.user.tier).green()
            );

            let two_fa_status = if profile.user.two_factor_enabled {
                "Enabled ✔".green()
            } else {
                "Disabled".yellow()
            };
            println!("│ {} {}", "2FA:".cyan(), two_fa_status);

            println!("├─ Usage ───────────────────────");
            println!("│ {} {}", "Hosted Clips:".cyan(), profile.clip_count);
            println!(
                "│ {} {:.2} GB / {} GB ({:.1}%)",
                "Storage:".cyan(),
                usage_gb,
                limit_gb,
                percentage
            );

            println!("├─ Activity ────────────────────");
            if let (Some(time), Some(ip)) =
                (profile.user.last_login_at, &profile.user.last_login_ip)
            {
                println!(
                    "│ {} {}",
                    "Last Login:".cyan(),
                    time.format("%Y-%m-%d %H:%M:%S UTC")
                );
                println!("│ {} {}", "From IP:".cyan(), ip);
            } else {
                println!("│ {}", "No login activity recorded.".cyan());
            }

            println!("└─────────────────────────────────");
        }
        Err(api::ApiClientError::Unauthorized) => {
            bail!("You are not logged in. Please run `wayclip login` first.");
        }
        Err(e) => {
            bail!("Failed to fetch profile: {}", e);
        }
    }
    Ok(())
}

async fn handle_2fa_status() -> Result<()> {
    match api::get_current_user().await {
        Ok(profile) => {
            if profile.user.two_factor_enabled {
                println!(
                    "{}",
                    "✔ Two-Factor Authentication is ENABLED".green().bold()
                );
                println!("Your account is protected with 2FA.");
            } else {
                println!(
                    "{}",
                    "⚠ Two-Factor Authentication is DISABLED".yellow().bold()
                );
                println!("Run `wayclip 2fa setup` to enable 2FA for better security.");
            }
        }
        Err(api::ApiClientError::Unauthorized) => {
            bail!("You are not logged in. Please run `wayclip login` first.");
        }
        Err(e) => {
            bail!("Failed to fetch profile: {}", e);
        }
    }
    Ok(())
}

async fn handle_share(clip_name: &str) -> Result<()> {
    let _ = api::get_current_user()
        .await
        .context("You must be logged in to share clips.")?;

    let clip = find_unified_clip(clip_name).await?;
    let clip_path_str = clip
        .local_path
        .context(format!("Clip '{}' not found locally.", clip.name))?;
    let clip_path = Path::new(&clip_path_str);

    let confirmed = Confirm::new("Are you sure you want to share this clip?")
        .with_default(true)
        .prompt()?;

    if !confirmed {
        println!("{}", "○ Share cancelled.".yellow());
        return Ok(());
    }

    println!("{}", "◌ Initializing upload...".yellow());
    let client = api::get_api_client().await?;
    match api::share_clip(&client, clip_path).await {
        Ok(url) => {
            println!("{}", "✔ Clip shared successfully!".green().bold());
            println!("  Public URL: {}", url.underline());

            let clip_id_str = url
                .split('/')
                .last()
                .context("Could not parse clip ID from URL")?;
            let clip_id = Uuid::parse_str(clip_id_str)?;

            let full_filename = clip_path
                .file_name()
                .and_then(|s| s.to_str())
                .context("Invalid filename")?;
            wayclip_core::update_hosted_id(full_filename, clip_id)
                .await
                .context("Failed to save hosted ID to local data file")?;

            match copy_to_clipboard(&url).await {
                Ok(_) => println!("{}", "✔ URL automatically copied to clipboard!".green()),
                Err(e) => println!(
                    "{}",
                    format!("✗ Could not copy URL to clipboard: {e:#}").yellow()
                ),
            }
        }
        Err(e) => bail!("Failed to share clip: {}", e),
    }
    Ok(())
}

async fn handle_save() -> Result<()> {
    let settings = Settings::load().await?;
    let mut trigger_command = Command::new(settings.trigger_path);
    let status = trigger_command
        .status()
        .await
        .context("Failed to execute the trigger process. Is the daemon running?")?;
    if status.success() {
        println!("{}", "✔ Trigger process finished successfully.".green());
    } else {
        bail!("Trigger process failed with status: {}", status);
    }
    Ok(())
}

async fn handle_config(editor: Option<&str>) -> Result<()> {
    let editor_name = editor
        .map(String::from)
        .or_else(|| env::var("VISUAL").ok())
        .or_else(|| env::var("EDITOR").ok());
    let mut command = match editor_name {
        Some(editor) => {
            println!("Using editor: {}", &editor);
            let mut parts = editor.split_whitespace();
            let mut cmd = Command::new(parts.next().unwrap_or("nano"));
            cmd.args(parts);
            cmd
        }
        None => {
            println!("VISUAL and EDITOR not set, falling back to nano.");
            Command::new("nano")
        }
    };
    command.arg(
        Settings::config_path()
            .join("wayclip")
            .join("settings.json"),
    );
    let status = command.status().await.context("Failed to open editor")?;
    if !status.success() {
        bail!("Editor process failed with status: {}", status);
    }
    Ok(())
}

async fn handle_view(name: &str, player: Option<&str>) -> Result<()> {
    let clip = find_unified_clip(name).await?;
    let clip_file_str = clip
        .local_path
        .context(format!("Clip '{}' not found locally.", clip.name))?;
    let clip_file = Path::new(&clip_file_str);

    let player_name = player.unwrap_or("mpv").to_string();
    let mut parts = player_name.split_whitespace();
    let player_cmd = parts.next().unwrap_or("mpv");
    let player_args = parts;

    let mut command = Command::new(player_cmd);
    command.args(player_args);
    command.arg(clip_file);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let status = command
        .status()
        .await
        .context(format!("Failed to launch media player '{player_name}'"))?;

    if status.success() {
        return Ok(());
    }

    if let Some(code) = status.code() {
        if code == 3 || code == 4 {
            return Ok(());
        }
    }

    bail!(
        "Media player exited with an unexpected error status: {}",
        status
    );
}

async fn handle_rename(name: &str) -> Result<()> {
    let clip_to_rename = find_unified_clip(name).await?;

    let clip_path_str = clip_to_rename
        .local_path
        .context("Cannot rename a clip that does not exist locally.")?;
    let clip_path = PathBuf::from(&clip_path_str);

    let new_name_input = Text::new("› Enter new name (without extension):")
        .with_initial_value(&clip_to_rename.name)
        .prompt()?;

    let new_name_stem = sanitize_and_validate_filename_stem(&new_name_input)?;

    if new_name_stem == clip_to_rename.name {
        println!("{}", "○ Rename cancelled (name is the same).".yellow());
        return Ok(());
    }

    let extension = clip_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("mp4");
    let new_full_name = format!("{new_name_stem}.{extension}");

    match rename_all_entries(&clip_path_str, &new_full_name).await {
        Ok(_) => println!("{}", format!("✔ Renamed to '{new_full_name}'").green()),
        Err(e) => bail!("Failed to rename: {}", e),
    }
    Ok(())
}

async fn handle_delete(name: &str) -> Result<()> {
    let clip_to_delete = find_unified_clip(name).await?;

    println!("○ Preparing to delete '{}'.", name.cyan());

    if let Some(hosted_id) = clip_to_delete.hosted_id {
        let confirmed = Confirm::new("This clip is hosted on the server. Delete the server copy?")
            .with_default(true)
            .prompt()?;
        if confirmed {
            let client = api::get_api_client().await?;
            api::delete_clip(&client, hosted_id).await?;
            println!("{}", "✔ Server copy deleted.".green());
        }
    }

    if let Some(local_path_str) = &clip_to_delete.local_path {
        let confirmed_local = Confirm::new("Delete the local file? This cannot be undone.")
            .with_default(false)
            .prompt()?;
        if confirmed_local {
            delete_file(local_path_str)
                .await
                .map_err(|e| anyhow::anyhow!(e))?;
            println!("{}", "✔ Local file deleted.".green());
        }
    }

    if clip_to_delete.local_path.is_none() && clip_to_delete.hosted_id.is_none() {
        println!(
            "{}",
            "○ Clip metadata found, but no local or hosted file to delete.".yellow()
        );
    }

    Ok(())
}

async fn handle_edit(
    name: &str,
    start_time_str: &str,
    end_time_str: &str,
    disable_audio: &bool,
) -> Result<()> {
    println!("○ Preparing to edit '{}'...", name.cyan());
    println!(
        "{}",
        "Note: This operation is performed locally and does not affect hosted clips.".yellow()
    );

    let start_time = validate_ffmpeg_time(start_time_str)?;
    let end_time = validate_ffmpeg_time(end_time_str)?;

    let clip = find_unified_clip(name).await?;
    let clip_path_str = clip
        .local_path
        .context(format!("Clip '{}' not found locally.", clip.name))?;
    let clip_path = PathBuf::from(&clip_path_str);

    let options = vec!["Create a new, edited copy", "Modify the original file"];
    let choice = Select::new("What would you like to do?", options).prompt()?;

    let (output_path, is_overwrite) = if choice == "Create a new, edited copy" {
        let new_name_suggestion = format!("{}_edited", clip.name);
        let new_name_input = Text::new("› Enter name for the new clip (without extension):")
            .with_initial_value(&new_name_suggestion)
            .prompt()?;
        let new_name_stem = sanitize_and_validate_filename_stem(&new_name_input)?;
        (
            clip_path.with_file_name(format!("{new_name_stem}.mp4")),
            false,
        )
    } else {
        let confirmed = Confirm::new("Modifying the original file cannot be undone. Are you sure?")
            .with_default(false)
            .prompt()?;
        if !confirmed {
            println!("{}", "○ Edit cancelled.".yellow());
            return Ok(());
        }
        (clip_path.clone(), true)
    };

    let temp_output_path = output_path.with_extension("tmp.mp4");

    println!("{}", "◌ Processing clip...".yellow());

    let mut command = Command::new("ffmpeg");
    command
        .arg("-i")
        .arg(&clip_path)
        .arg("-ss")
        .arg(start_time)
        .arg("-to")
        .arg(end_time)
        .arg("-c:v")
        .arg("copy");

    if *disable_audio {
        command.arg("-an");
    } else {
        command.arg("-c:a").arg("copy");
    }

    command.arg(&temp_output_path);

    let output = command
        .output()
        .await
        .context("Failed to execute ffmpeg. Is it installed and in your PATH?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("ffmpeg failed with status: {}\n{}", output.status, stderr);
    }

    if is_overwrite {
        tokio::fs::rename(&temp_output_path, &clip_path)
            .await
            .context("Failed to replace original file")?;
        println!("{}", "✔ Original clip successfully modified.".green());
    } else {
        tokio::fs::rename(&temp_output_path, &output_path)
            .await
            .context("Failed to save new clip")?;
        println!(
            "{}",
            format!(
                "✔ New clip saved as '{}'",
                output_path.file_name().unwrap().to_str().unwrap()
            )
            .green()
        );
    }

    Ok(())
}
