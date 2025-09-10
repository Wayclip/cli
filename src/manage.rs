use crate::{copy_to_clipboard, handle_edit, handle_share, handle_view};
use anyhow::{Context, Result};
use chrono::Utc;
use colored::*;
use inquire::{Confirm, Select, Text};
use std::fmt;
use std::path::Path;
use wayclip_core::{
    api, delete_file, gather_unified_clips, models::UnifiedClipData, rename_all_entries,
    update_liked,
};

struct ClipMenuItem {
    clip_data: UnifiedClipData,
}

impl fmt::Display for ClipMenuItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let clip = &self.clip_data;
        let now = Utc::now();
        let clip_age = now.signed_duration_since(clip.created_at.with_timezone(&Utc));
        write!(
            f,
            "{} {} {}{}{}",
            if clip.local_path.is_some() {
                "⌨"
            } else {
                "  "
            },
            if clip.is_hosted { "☁" } else { "  " },
            if clip.local_data.as_ref().map_or(false, |d| d.liked) {
                "♥ ".red()
            } else {
                "".normal()
            },
            clip.name,
            if clip_age < chrono::Duration::hours(24) {
                " [NEW]".yellow()
            } else {
                "".normal()
            }
        )
    }
}

async fn find_clip_by_name(name: &str) -> Result<Option<UnifiedClipData>> {
    let clips = gather_unified_clips().await?;
    Ok(clips.into_iter().find(|c| c.name == name))
}

pub async fn handle_manage() -> Result<()> {
    let settings = wayclip_core::settings::Settings::load().await?;

    'main_loop: loop {
        println!();
        let mut all_clips = gather_unified_clips().await?;

        if all_clips.is_empty() {
            println!("{}", "No clips found.".yellow());
            break;
        }

        let sort_options = vec![
            "Date (Newest First)",
            "Name (A-Z)",
            "Liked First",
            "Hosted First",
            "[Quit]",
        ];
        let sort_choice = match Select::new("Filter / Sort clips:", sort_options).prompt() {
            Ok(choice) => choice,
            Err(_) => break 'main_loop,
        };

        match sort_choice {
            "Date (Newest First)" => all_clips.sort_by(|a, b| b.created_at.cmp(&a.created_at)),
            "Name (A-Z)" => {
                all_clips.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            }
            "Liked First" => all_clips.sort_by(|a, b| {
                b.local_data
                    .as_ref()
                    .map_or(false, |d| d.liked)
                    .cmp(&a.local_data.as_ref().map_or(false, |d| d.liked))
                    .then(b.created_at.cmp(&a.created_at))
            }),
            "Hosted First" => all_clips.sort_by(|a, b| {
                b.is_hosted
                    .cmp(&a.is_hosted)
                    .then(b.created_at.cmp(&a.created_at))
            }),
            _ => break 'main_loop,
        }

        let clip_menu_items: Vec<ClipMenuItem> = all_clips
            .into_iter()
            .map(|clip| ClipMenuItem { clip_data: clip })
            .collect();

        let selected_item = match Select::new("Select a clip to manage:", clip_menu_items)
            .with_page_size(15)
            .prompt()
        {
            Ok(item) => item,
            Err(_) => continue 'main_loop,
        };

        let mut selected_clip = selected_item.clip_data;

        'action_loop: loop {
            println!();

            let mut options = Vec::new();
            if selected_clip.is_hosted {
                options.push("⌂ Open URL");
                options.push("☐ Copy URL");
            }
            if selected_clip.local_path.is_some() {
                options.push("▷ View Local File");
                if !selected_clip.is_hosted {
                    options.push("✎ Rename");
                }
                options.push("✎ Edit");
                options.push("⎘ Copy Name");
                if selected_clip.local_data.as_ref().map_or(false, |d| d.liked) {
                    options.push("♡ Unlike");
                } else {
                    options.push("♥ Like");
                }
            }
            if !selected_clip.is_hosted && selected_clip.local_path.is_some() {
                options.push("↗ Share");
            }
            if selected_clip.is_hosted {
                options.push("⌫ Delete Server Copy");
            }
            if selected_clip.local_path.is_some() {
                options.push("⌫ Delete Local File");
            }
            options.push("← Back to Clip List");

            let action = match Select::new(
                &format!("Action for '{}':", selected_clip.name.cyan()),
                options,
            )
            .prompt()
            {
                Ok(choice) => choice,
                Err(_) => break 'action_loop,
            };

            match action {
                "▷ View Local File" => {
                    if let Err(e) = handle_view(&selected_clip.full_filename, None).await {
                        println!("{} {}", "Error viewing clip:".red(), e);
                    }
                }
                "← Back to Clip List" => {
                    break 'action_loop;
                }
                "✎ Rename" => {
                    if selected_clip.is_hosted {
                        println!(
                            "{}",
                            "Cannot rename a clip that has been shared/hosted.".red()
                        );
                    } else {
                        let local_path = selected_clip
                            .local_path
                            .as_ref()
                            .context("No local path for rename")?;
                        let new_name_stem = Text::new("Enter new name (without extension):")
                            .with_initial_value(&selected_clip.name)
                            .prompt()?;
                        if !new_name_stem.is_empty() && new_name_stem != selected_clip.name {
                            let extension = Path::new(local_path)
                                .extension()
                                .and_then(|s| s.to_str())
                                .unwrap_or("mp4");
                            let new_full_name = format!("{new_name_stem}.{extension}");
                            match rename_all_entries(local_path, &new_full_name).await {
                                Ok(_) => println!("✔ Renamed to '{}'", new_full_name.green()),
                                Err(e) => println!("✗ Failed to rename: {}", e.to_string().red()),
                            }
                            break 'action_loop;
                        } else {
                            println!("{}", "Rename cancelled.".yellow());
                        }
                    }
                }
                "⌫ Delete Server Copy" | "⌫ Delete Local File" => {
                    let confirm_msg = if action == "⌫ Delete Server Copy" {
                        "Delete server copy?"
                    } else {
                        "Delete local file?"
                    };
                    let confirmed = Confirm::new(confirm_msg)
                        .with_help_message("This cannot be undone.")
                        .with_default(false)
                        .prompt()?;

                    if confirmed {
                        let result: Result<()> = if action == "⌫ Delete Server Copy" {
                            api::delete_clip(
                                &api::get_api_client().await?,
                                selected_clip.hosted_id.unwrap(),
                            )
                            .await
                            .map_err(|e| anyhow::anyhow!(e))
                        } else {
                            delete_file(selected_clip.local_path.as_ref().unwrap())
                                .await
                                .map_err(|e| anyhow::anyhow!(e))
                        };

                        match result {
                            Ok(_) => println!(
                                "{}",
                                "✔ Operation successful. Returning to main list.".green()
                            ),
                            Err(e) => println!("✗ Operation failed: {}", e.to_string().red()),
                        }
                        break 'action_loop;
                    }
                }
                "⌂ Open URL" => {
                    opener::open(format!(
                        "{}/clip/{}",
                        settings.api_url,
                        selected_clip.hosted_id.unwrap()
                    ))?;
                    println!("Opening URL in browser...");
                }
                "✎ Edit" => {
                    let start_time =
                        Text::new("Enter start time (e.g., 00:00:05 or 5):").prompt()?;
                    let end_time = Text::new("Enter end time (e.g., 00:00:10 or 10):").prompt()?;
                    let disable_audio = Confirm::new("Disable audio?")
                        .with_default(false)
                        .prompt()?;
                    if let Err(e) = handle_edit(
                        &selected_clip.full_filename,
                        &start_time,
                        &end_time,
                        &disable_audio,
                    )
                    .await
                    {
                        println!("{} {}", "✗ Edit failed:".red(), e);
                    }
                }
                "⎘ Copy Name" => match copy_to_clipboard(&selected_clip.name).await {
                    Ok(_) => println!("{}", "✔ Name copied to clipboard!".green()),
                    Err(e) => println!("{}", format!("✗ Failed to copy name: {e}").red()),
                },
                "♥ Like" | "♡ Unlike" => {
                    let new_liked_status =
                        !selected_clip.local_data.as_ref().map_or(false, |d| d.liked);
                    match update_liked(&selected_clip.full_filename, new_liked_status).await {
                        Ok(_) => println!("{}", "✔ Liked status updated!".green()),
                        Err(e) => {
                            println!("{}", format!("✗ Failed to update liked status: {e}").red())
                        }
                    }
                }
                "↗ Share" => {
                    if let Err(e) = handle_share(&selected_clip.name).await {
                        println!("{} {}", "✗ Share failed:".red(), e);
                    }
                }
                "☐ Copy URL" => {
                    if let Some(id) = selected_clip.hosted_id {
                        let public_url = format!("{}/clip/{}", settings.api_url, id);
                        match copy_to_clipboard(&public_url).await {
                            Ok(_) => println!(
                                "✔ Public URL copied to clipboard: {}",
                                public_url.underline().green()
                            ),
                            Err(e) => println!("{}", format!("✗ Failed to copy URL: {e}").red()),
                        }
                    }
                }
                _ => {}
            }

            if let Some(updated_clip) = find_clip_by_name(&selected_clip.name).await? {
                selected_clip = updated_clip;
            } else {
                println!(
                    "{}",
                    "\nClip data has changed, returning to main list.".yellow()
                );
                break 'action_loop;
            }
        }
    }

    Ok(())
}
