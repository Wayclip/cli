use crate::unified_clip::find_unified_clip;
use anyhow::Result;
use colored::*;
use inquire::Confirm;
use wayclip_core::{api, delete_file};

pub async fn handle_delete(name: &str) -> Result<()> {
    let clip_to_delete = find_unified_clip(name).await?;

    println!("○ Preparing to delete '{}'.", name.cyan());

    if let Some(hosted_id) = clip_to_delete.hosted_id {
        let confirmed = Confirm::new("This clip is hosted on the server. Delete the server copy?")
            .with_default(true)
            .prompt()?;
        if confirmed {
            let client = api::get_api_client().await?;
            api::delete_clip(&client, hosted_id).await?;
            println!("{}", "✔ Server copy deleted.".green());
        }
    }

    if let Some(local_path_str) = &clip_to_delete.local_path {
        let confirmed_local = Confirm::new("Delete the local file? This cannot be undone.")
            .with_default(false)
            .prompt()?;
        if confirmed_local {
            delete_file(local_path_str)
                .await
                .map_err(|e| anyhow::anyhow!(e))?;
            println!("{}", "✔ Local file deleted.".green());
        }
    }

    if clip_to_delete.local_path.is_none() && clip_to_delete.hosted_id.is_none() {
        println!(
            "{}",
            "○ Clip metadata found, but no local or hosted file to delete.".yellow()
        );
    }

    Ok(())
}
