use crate::clipboard::copy_to_clipboard;
use crate::unified_clip::find_unified_clip;
use anyhow::{Context, Result, bail};
use colored::*;
use wayclip_core::settings::Settings;

pub async fn handle_url(name: &str) -> Result<()> {
    let clip = find_unified_clip(name).await?;
    let settings = Settings::load().await?;

    if let Some(id) = clip.hosted_id {
        let public_url = format!("{}/clip/{}", settings.api_url, id);
        println!("  {}", public_url.underline());
        match copy_to_clipboard(&public_url).await {
            Ok(_) => println!("{}", "✔ Public URL copied to clipboard!".green()),
            Err(e) => println!(
                "{}",
                format!("✗ Could not copy URL to clipboard: {e:#}").yellow()
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

pub async fn handle_open(name: &str) -> Result<()> {
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
