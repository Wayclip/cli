use crate::auth::{handle_login, handle_logout};
use crate::list::handle_list;
use crate::manage::handle_manage;
use anyhow::{Context, Result, bail};
use arboard::Clipboard;
use clap::{Parser, Subcommand};
use colored::*;
use inquire::{Confirm, Select, Text};
use std::env;
use std::path::Path;
use std::process::ExitCode;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use wayclip_core::control::DaemonManager;
use wayclip_core::{
    Collect, PullClipsArgs, api, delete_file, gather_clip_data, gather_unified_clips,
    rename_all_entries, update_liked,
};
use wayclip_core::{models::UnifiedClipData, settings::Settings};

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
pub enum DaemonCommand {
    Start,
    Stop,
    Restart,
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
             - arboard error: {:#}",
            e
        ),
        Err(e) => bail!("Clipboard task failed: {:#}", e),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(e) = run().await {
        eprintln!("{} {:#}", "Error:".red().bold(), e);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    if cli.debug {
        println!("{}", "Debug mode is ON".yellow());
    }

    match &cli.command {
        Commands::Login => handle_login().await?,
        Commands::Logout => handle_logout().await?,
        Commands::Me => handle_me().await?,
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
                DaemonCommand::Status => {
                    manager.status().await?;
                }
            }
        }
    }

    Ok(())
}

async fn find_unified_clip(name: &str) -> Result<UnifiedClipData> {
    let name_stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name);

    let all_clips = gather_unified_clips().await?;
    all_clips
        .into_iter()
        .find(|clip| clip.name == name_stem)
        .context(format!("Clip '{name_stem}' not found."))
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
                (usage_gb / limit_gb) * 100.0
            } else {
                0.0
            };

            println!("{}", "┌─ Your Profile ─────────".bold());
            println!("│ {} {}", "Username:".cyan(), profile.user.username);
            println!(
                "│ {} {}",
                "Tier:".cyan(),
                format!("{:?}", profile.user.tier).green()
            );
            println!("│ {} {}", "Hosted Clips:".cyan(), profile.clip_count);
            println!(
                "│ {} {:.2} GB / {:.2} GB ({:.1}%)",
                "Storage:".cyan(),
                usage_gb,
                limit_gb,
                percentage
            );
            println!("└────────────────────────");
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
    println!("{}", "○ Preparing to share...".cyan());

    let _ = api::get_current_user()
        .await
        .context("Could not get user profile. Are you logged in?")?;
    let settings = Settings::load().await?;
    let clips_path = Settings::home_path().join(&settings.save_path_from_home_string);

    let clip_filename = if clip_name.ends_with(".mp4") {
        clip_name.to_string()
    } else {
        format!("{clip_name}.mp4")
    };
    let clip_path = clips_path.join(&clip_filename);

    if !clip_path.exists() {
        bail!("Clip '{}' not found locally.", clip_name);
    }

    let profile = api::get_current_user().await?;
    let file_size = tokio::fs::metadata(&clip_path).await?.len() as i64;
    let available_storage = profile.storage_limit - profile.storage_used;

    if file_size > available_storage {
        bail!(
            "Upload rejected: File size ({:.2} MB) exceeds your available storage ({:.2} MB).",
            file_size as f64 / 1_048_576.0,
            available_storage as f64 / 1_048_576.0
        );
    }

    println!(
        "{}",
        "◌ Uploading clip... (this may take a moment)".yellow()
    );

    let client = api::get_api_client().await?;
    match api::share_clip(&client, &clip_path).await {
        Ok(url) => {
            println!("{}", "✔ Clip shared successfully!".green().bold());
            println!("  Public URL: {}", url.underline());

            match copy_to_clipboard(&url).await {
                Ok(_) => println!("{}", "✔ URL automatically copied to clipboard!".green()),
                Err(e) => {
                    println!(
                        "{}",
                        format!("✗ Could not copy URL to clipboard: {e:#}").yellow()
                    )
                }
            }
        }
        Err(api::ApiClientError::Unauthorized) => {
            bail!("You must be logged in to share clips. Please run `wayclip login`.");
        }
        Err(e) => {
            bail!("Failed to share clip: {}", e);
        }
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
    let settings = Settings::load().await?;
    let clips_path = Settings::home_path().join(&settings.save_path_from_home_string);
    let clip_filename = if name.ends_with(".mp4") {
        name.to_string()
    } else {
        format!("{name}.mp4")
    };
    let clip_file = clips_path.join(&clip_filename);

    if !clip_file.exists() {
        bail!("Clip '{}' not found locally.", clip_filename);
    }

    let player_name = player.unwrap_or("mpv");
    println!(
        "⏵ Launching '{}' with {}...",
        clip_filename.cyan(),
        player_name
    );
    let mut parts = player_name.split_whitespace();
    let mut command = Command::new(parts.next().unwrap_or("mpv"));
    command.args(parts);
    command.arg(clip_file);
    let mut child = command
        .spawn()
        .context(format!("Failed to launch media player '{player_name}'"))?;

    let _ = child.wait().await;
    Ok(())
}

async fn handle_rename(name: &str) -> Result<()> {
    let name_stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name);

    let clips = gather_clip_data(
        Collect::All,
        PullClipsArgs {
            page: 1,
            page_size: 999,
            search_query: Some(name_stem.to_string()),
        },
    )
    .await?
    .clips;

    let clip_to_rename = clips
        .first()
        .context(format!("Clip '{name_stem}' not found."))?;

    let new_name_stem = Text::new("Enter new name (without extension):")
        .with_initial_value(&clip_to_rename.name)
        .prompt()?;

    if new_name_stem.is_empty() || new_name_stem == clip_to_rename.name {
        println!("{}", "Rename cancelled.".yellow());
        return Ok(());
    }

    let extension = Path::new(&clip_to_rename.path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("mp4");
    let new_full_name = format!("{new_name_stem}.{extension}");

    match rename_all_entries(&clip_to_rename.path, &new_full_name).await {
        Ok(_) => println!("{}", format!("✔ Renamed to '{new_full_name}'").green()),
        Err(e) => bail!("Failed to rename: {}", e),
    }
    Ok(())
}

async fn handle_delete(name: &str) -> Result<()> {
    let name_stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name);

    let clips = gather_clip_data(
        Collect::All,
        PullClipsArgs {
            page: 1,
            page_size: 999,
            search_query: Some(name_stem.to_string()),
        },
    )
    .await?
    .clips;

    let clip_to_delete = clips
        .first()
        .context(format!("Clip '{name_stem}' not found."))?;

    let hosted_clips = api::get_hosted_clips_index().await.unwrap_or_default();
    let clip_filename = Path::new(&clip_to_delete.path)
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    let hosted_info = hosted_clips.iter().find(|c| c.file_name == clip_filename);

    println!("Preparing to delete '{}'.", name.cyan());

    if let Some(hosted) = hosted_info {
        let confirmed = Confirm::new("This clip is hosted on the server. Delete the server copy?")
            .with_default(true)
            .prompt()?;
        if confirmed {
            let client = api::get_api_client().await?;
            api::delete_clip(&client, hosted.id).await?;
            println!("{}", "✔ Server copy deleted.".green());
        }
    }

    let confirmed_local = Confirm::new("Delete the local file? This cannot be undone.")
        .with_default(false)
        .prompt()?;
    if confirmed_local {
        delete_file(&clip_to_delete.path)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        println!("{}", "✔ Local file deleted.".green());
    }

    Ok(())
}

async fn handle_edit(
    name: &str,
    start_time: &str,
    end_time: &str,
    disable_audio: &bool,
) -> Result<()> {
    println!("{} '{}'...", "Preparing to edit".cyan(), name);
    println!(
        "{}",
        "Note: This operation is performed locally and does not affect hosted clips.".yellow()
    );

    let settings = Settings::load().await?;
    let clips_path = Settings::home_path().join(&settings.save_path_from_home_string);

    let clip_filename = if name.ends_with(".mp4") {
        name.to_string()
    } else {
        format!("{name}.mp4")
    };
    let clip_path = clips_path.join(&clip_filename);

    if !clip_path.exists() {
        bail!("Clip '{}' not found locally.", name);
    }

    let options = vec!["Create a new, edited copy", "Modify the original file"];
    let choice = Select::new("What would you like to do?", options).prompt()?;

    let (output_path, is_overwrite) = if choice == "Create a new, edited copy" {
        let file_stem = Path::new(name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("edited_clip");
        let new_name_suggestion = format!("{file_stem}_edited");
        let new_name = Text::new("Enter name for the new clip (without extension):")
            .with_initial_value(&new_name_suggestion)
            .prompt()?;
        (clip_path.with_file_name(format!("{new_name}.mp4")), false)
    } else {
        let confirmed = Confirm::new("Modifying the original file cannot be undone. Are you sure?")
            .with_default(false)
            .prompt()?;
        if !confirmed {
            println!("{}", "Edit cancelled.".yellow());
            return Ok(());
        }
        (clip_path.clone(), true)
    };

    let temp_output_path = output_path.with_extension("tmp.mp4");

    println!("{}", "Processing clip...".yellow());

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
