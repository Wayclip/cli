use anyhow::{Context, Result, bail};
use colored::*;
use tokio::process::Command;
use wayclip_core::settings::Settings;

pub async fn handle_save() -> Result<()> {
    let settings = Settings::load().await?;
    let mut trigger_command = Command::new(settings.trigger_path);
    let status = trigger_command
        .status()
        .await
        .context("Failed to execute the trigger process. Is the daemon running?")?;
    if status.success() {
        println!("{}", "✔ Trigger process finished successfully.".green());
    } else {
        bail!("Trigger process failed with status: {status}");
    }
    Ok(())
}
