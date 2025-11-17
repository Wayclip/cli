use crate::unified_clip::find_unified_clip;
use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

pub async fn handle_view(name: &str, player: Option<&str>) -> Result<()> {
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

    bail!("Media player exited with an unexpected error status: {status}",);
}
