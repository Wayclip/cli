use crate::unified_clip::find_unified_clip;
use crate::validate::{sanitize_and_validate_filename_stem, validate_ffmpeg_time};
use anyhow::{Context, Result, bail};
use colored::*;
use inquire::{Confirm, Select, Text};
use std::path::PathBuf;
use tokio::process::Command;

pub async fn handle_edit(
    name: &str,
    start_time_str: &str,
    end_time_str: &str,
    disable_audio: &bool,
) -> Result<()> {
    println!("○ Preparing to edit '{}'...", name.cyan());
    println!(
        "{}",
        "Note: This operation is performed locally and does not affect hosted clips.".yellow()
    );

    let start_time = validate_ffmpeg_time(start_time_str)?;
    let end_time = validate_ffmpeg_time(end_time_str)?;

    let clip = find_unified_clip(name).await?;
    let clip_path_str = clip
        .local_path
        .context(format!("Clip '{}' not found locally.", clip.name))?;
    let clip_path = PathBuf::from(&clip_path_str);

    let options = vec!["Create a new, edited copy", "Modify the original file"];
    let choice = Select::new("What would you like to do?", options).prompt()?;

    let (output_path, is_overwrite) = if choice == "Create a new, edited copy" {
        let new_name_suggestion = format!("{}_edited", clip.name);
        let new_name_input = Text::new("› Enter name for the new clip (without extension):")
            .with_initial_value(&new_name_suggestion)
            .prompt()?;
        let new_name_stem = sanitize_and_validate_filename_stem(&new_name_input)?;
        (
            clip_path.with_file_name(format!("{new_name_stem}.mp4")),
            false,
        )
    } else {
        let confirmed = Confirm::new("Modifying the original file cannot be undone. Are you sure?")
            .with_default(false)
            .prompt()?;
        if !confirmed {
            println!("{}", "○ Edit cancelled.".yellow());
            return Ok(());
        }
        (clip_path.clone(), true)
    };

    let temp_output_path = output_path.with_extension("tmp.mp4");

    println!("{}", "◌ Processing clip...".yellow());

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
