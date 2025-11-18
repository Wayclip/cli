use crate::unified_clip::find_unified_clip;
use crate::validate::sanitize_and_validate_filename_stem;
use anyhow::{Context, Result, bail};
use colored::*;
use inquire::Text;
use std::path::PathBuf;
use wayclip_core::rename_all_entries;

pub async fn handle_rename(name: &str) -> Result<()> {
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
        Err(e) => bail!("Failed to rename: {e}"),
    }
    Ok(())
}
