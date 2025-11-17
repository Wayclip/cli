use anyhow::{Result, bail};
use arboard::Clipboard;
use std::env;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

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
