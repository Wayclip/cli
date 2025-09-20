use crate::{copy_to_clipboard, handle_edit, handle_share, handle_view};
use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use colored::*;
use inquire::{Confirm, Select, Text};
use std::fmt;
use std::path::Path;
use wayclip_core::{
    api, delete_file, gather_unified_clips, models::UnifiedClipData, rename_all_entries,
    update_liked,
};

#[derive(Clone)]
struct ClipDisplay {
    name: String,
    display_string: String,
}

impl fmt::Display for ClipDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_string)
    }
}

fn generate_display_string(clip: &UnifiedClipData) -> String {
    let now = Utc::now();
    let clip_age = now.signed_duration_since(clip.created_at.with_timezone(&Utc));
    format!(
        "{} {} {}{}{}",
        if clip.local_path.is_some() {
            "⌨"
        } else {
            "  "
        },
        if clip.is_hosted { "☁" } else { "  " },
        if clip.local_data.as_ref().map_or(false, |d| d.liked) {
            "♥ ".red().to_string()
        } else {
            "".normal().to_string()
        },
        clip.name,
        if clip_age < chrono::Duration::hours(24) {
            " [NEW]".yellow().to_string()
        } else {
            "".normal().to_string()
        }
    )
}

fn sanitize_and_validate_filename_stem(new_name_input: &str) -> Result<String> {
    let trimmed = new_name_input.trim();
    if trimmed.is_empty() {
        bail!("New name cannot be empty.");
    }

    let sanitized = trimmed.replace(' ', "_");
    if sanitized
        .chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
    {
        bail!("New name contains invalid characters (< > : \" / \\ | ? *).");
    }
    Ok(sanitized.to_string())
}

pub async fn handle_manage() -> Result<()> {
    let settings = wayclip_core::settings::Settings::load().await?;

    println!("\n{}", "◌ Loading clips...".yellow());
    let mut all_clips: Vec<UnifiedClipData> = gather_unified_clips().await?;

    'main_loop: loop {
        if all_clips.is_empty() {
            println!("{}", "○ No clips found.".yellow());
            return Ok(());
        }

        let sort_options = vec![
            "Date (Newest First)",
            "Name (A-Z)",
            "Liked First",
            "Hosted First",
            "[Refresh List]",
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
                    .then_with(|| b.created_at.cmp(&a.created_at))
            }),
            "Hosted First" => all_clips.sort_by(|a, b| {
                b.is_hosted
                    .cmp(&a.is_hosted)
                    .then_with(|| b.created_at.cmp(&a.created_at))
            }),
            "[Refresh List]" => {
                println!("{}", "◌ Refreshing clips...".yellow());
                all_clips = gather_unified_clips().await?;
                continue 'main_loop;
            }
            _ => break 'main_loop,
        }

        let display_items: Vec<_> = all_clips
            .iter()
            .map(|clip| ClipDisplay {
                name: clip.name.clone(),
                display_string: generate_display_string(clip),
            })
            .collect();

        let selected_display_item = match Select::new("Select a clip to manage:", display_items)
            .with_page_size(15)
            .prompt()
        {
            Ok(item) => item,
            Err(_) => continue 'main_loop,
        };

        let selected_idx = all_clips
            .iter()
            .position(|c| c.name == selected_display_item.name)
            .context("Could not find selected clip in memory. Please refresh.")?;

        'action_loop: loop {
            let mut break_to_main_menu = false;

            let clip = &mut all_clips[selected_idx];

            let mut options = Vec::new();
            if clip.is_hosted {
                options.push("⌂ Open URL");
                options.push("☐ Copy URL");
            }
            if clip.local_path.is_some() {
                options.push("▷ View Local File");
                options.push("✎ Rename");
                options.push("✎ Edit");
                options.push("⎘ Copy Name");
                if clip.local_data.as_ref().map_or(false, |d| d.liked) {
                    options.push("♡ Unlike");
                } else {
                    options.push("♥ Like");
                }
            }
            if !clip.is_hosted && clip.local_path.is_some() {
                options.push("↗ Share");
            }
            if clip.is_hosted {
                options.push("⌫ Delete Server Copy");
            }
            if clip.local_path.is_some() {
                options.push("⌫ Delete Local File");
            }
            options.push("← Back to Clip List");

            let action = match Select::new(&format!("Action for '{}':", clip.name.cyan()), options)
                .prompt()
            {
                Ok(choice) => choice,
                Err(_) => break 'action_loop,
            };

            match action {
                "← Back to Clip List" => break 'action_loop,

                "▷ View Local File" => {
                    if let Err(e) = handle_view(&clip.full_filename, None).await {
                        println!("{} {}", "✗ Error viewing clip:".red(), e);
                    }
                }

                "♥ Like" | "♡ Unlike" => {
                    let new_status = !clip.local_data.as_ref().map_or(false, |d| d.liked);
                    match update_liked(&clip.full_filename, new_status).await {
                        Ok(_) => {
                            if let Some(local_data) = clip.local_data.as_mut() {
                                local_data.liked = new_status;
                            }
                            println!("{}", "✔ Liked status updated!".green());
                        }
                        Err(e) => {
                            println!("{}", format!("✗ Failed to update liked status: {e}").red())
                        }
                    }
                }

                "⌫ Delete Server Copy" | "⌫ Delete Local File" => {
                    let is_server = action == "⌫ Delete Server Copy";
                    let confirmed = Confirm::new(if is_server {
                        "Delete server copy?"
                    } else {
                        "Delete local file?"
                    })
                    .with_help_message("This cannot be undone.")
                    .with_default(false)
                    .prompt()?;

                    if confirmed {
                        let result: Result<()> = if is_server {
                            let client = api::get_api_client().await?;
                            api::delete_clip(&client, clip.hosted_id.unwrap())
                                .await
                                .map_err(|e| anyhow!(e))
                        } else {
                            delete_file(clip.local_path.as_ref().unwrap())
                                .await
                                .map_err(|e| anyhow!(e))
                        };

                        match result {
                            Ok(_) => {
                                println!("{}", "✔ Operation successful.".green());
                                all_clips.remove(selected_idx);
                                break_to_main_menu = true;
                            }
                            Err(e) => println!("✗ Operation failed: {}", e.to_string().red()),
                        }
                    }
                }

                "⌂ Open URL" => {
                    let url = format!("{}/clip/{}", settings.api_url, clip.hosted_id.unwrap());
                    if opener::open(&url).is_ok() {
                        println!("○ Opening URL in browser: {}", url.cyan());
                    } else {
                        println!("✗ Failed to open URL.");
                    }
                }

                "☐ Copy URL" => {
                    let public_url =
                        format!("{}/clip/{}", settings.api_url, clip.hosted_id.unwrap());
                    match copy_to_clipboard(&public_url).await {
                        Ok(_) => println!("{}", "✔ Public URL copied!".green()),
                        Err(e) => println!("{}", format!("✗ Failed to copy URL: {e}").red()),
                    }
                }

                "⎘ Copy Name" => match copy_to_clipboard(&clip.name).await {
                    Ok(_) => println!("{}", "✔ Name copied!".green()),
                    Err(e) => println!("{}", format!("✗ Failed to copy name: {e}").red()),
                },

                "↗ Share" => {
                    if let Err(e) = handle_share(&clip.name).await {
                        println!("{} {}", "✗ Share failed:".red(), e);
                    } else {
                        println!("{}", "◌ Refreshing clip state...".yellow());
                        if let Some(updated_clip) = gather_unified_clips()
                            .await?
                            .into_iter()
                            .find(|c| c.name == clip.name)
                        {
                            *clip = updated_clip;
                        }
                        println!("{}", "✔ Clip is now hosted.".green());
                    }
                }

                "✎ Rename" => {
                    let local_path_str = clip.local_path.as_ref().context("No local path")?.clone();
                    let new_name_input = Text::new("› Enter new name:")
                        .with_initial_value(&clip.name)
                        .prompt()?;

                    match sanitize_and_validate_filename_stem(&new_name_input) {
                        Ok(new_stem) if new_stem != clip.name => {
                            let ext = Path::new(&local_path_str)
                                .extension()
                                .and_then(|s| s.to_str())
                                .unwrap_or("mp4");
                            let new_full = format!("{new_stem}.{ext}");
                            match rename_all_entries(&local_path_str, &new_full).await {
                                Ok(_) => {
                                    println!("✔ Renamed to '{}'", new_full.green());
                                    println!("{}", "◌ Refreshing clip list...".yellow());
                                    all_clips = gather_unified_clips().await?;
                                    break_to_main_menu = true;
                                }
                                Err(e) => println!("✗ Failed to rename: {}", e.to_string().red()),
                            }
                        }
                        Ok(_) => println!("{}", "○ Rename cancelled.".yellow()),
                        Err(e) => println!("✗ Invalid name: {}", e.to_string().red()),
                    }
                }

                "✎ Edit" => {
                    let start_time =
                        Text::new("› Enter start time (e.g., 5.5 or 00:01:30):").prompt()?;
                    let end_time =
                        Text::new("› Enter end time (e.g., 10 or 00:02:00):").prompt()?;
                    let disable_audio = Confirm::new("Disable audio?")
                        .with_default(false)
                        .prompt()?;

                    if let Err(e) =
                        handle_edit(&clip.full_filename, &start_time, &end_time, &disable_audio)
                            .await
                    {
                        println!("{} {}", "✗ Edit failed:".red(), e);
                    } else {
                        println!("{}", "◌ Refreshing clip list...".yellow());
                        all_clips = gather_unified_clips().await?;
                        break_to_main_menu = true;
                    }
                }
                _ => {}
            }

            if break_to_main_menu {
                break 'action_loop;
            }
        }
    }
    Ok(())
}
