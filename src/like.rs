use crate::unified_clip::find_unified_clip;
use anyhow::{Result, bail};
use colored::*;
use wayclip_core::update_liked;

pub async fn handle_like(name: &str) -> Result<()> {
    let clip = find_unified_clip(name).await?;

    if let (Some(local_data), Some(_)) = (&clip.local_data, &clip.local_path) {
        let new_liked_status = !local_data.liked;
        match update_liked(&clip.full_filename, new_liked_status).await {
            Ok(_) => {
                let status = if new_liked_status { "Liked" } else { "Unliked" };
                println!("✔ Clip '{}' has been {}.", clip.name.cyan(), status.green());
            }
            Err(e) => bail!("Failed to update liked status: {e}"),
        }
    } else {
        bail!(
            "Clip '{}' does not exist locally and cannot be liked/unliked.",
            clip.name
        );
    }
    Ok(())
}
