use anyhow::{Context, Result, bail};
use colored::*;
use tokio::process::Command;
use wayclip_core::control::DaemonManager;
use wayclip_core::settings::Settings;

pub async fn handle_save() -> Result<()> {
    let manager = DaemonManager::new();
    if !manager.is_running().await {
        bail!("Daemon is not running.  Start it with: wayclip daemon start");
    }

    let settings = Settings::load().await?;
    let mut trigger_command = Command::new(settings.trigger_path);
    let status = trigger_command
        .status()
        .await
        .context("Failed to execute the trigger process.")?;
    if status.success() {
        println!("{}", "✔ Trigger process finished successfully.".green());
    } else {
        bail!("Trigger process failed with status: {status}");
    }
    Ok(())
}
