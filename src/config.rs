use anyhow::{Context, Result, bail};
use std::env;
use tokio::process::Command;
use wayclip_core::settings::Settings;

pub async fn handle_config(editor: Option<&str>) -> Result<()> {
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
        bail!("Editor process failed with status: {status}");
    }
    Ok(())
}
