use crate::clipboard::copy_to_clipboard;
use crate::unified_clip::find_unified_clip;
use anyhow::{Context, Result, bail};
use colored::*;
use inquire::Confirm;
use std::path::Path;
use uuid::Uuid;
use wayclip_core::api;

pub async fn handle_me() -> Result<()> {
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
            bail!("Failed to fetch profile: {e}");
        }
    }
    Ok(())
}

pub async fn handle_share(clip_name: &str) -> Result<()> {
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
                .next_back()
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
        Err(e) => bail!("Failed to share clip: {e}"),
    }
    Ok(())
}
