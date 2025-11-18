use crate::model::{AudioDevice, PwNode};
use anyhow::{Context, Result, bail};
use colored::*;
use inquire::Select;
use regex::Regex;
use tokio::process::Command;
use wayclip_core::settings::Settings;

pub async fn handle_audio() -> Result<()> {
    println!("○ Gathering audio device information...");

    let pw_dump_output = Command::new("pw-dump")
        .arg("Node")
        .output()
        .await
        .context("Failed to execute 'pw-dump'. Is PipeWire installed and running?")?;

    if !pw_dump_output.status.success() {
        let stderr = String::from_utf8_lossy(&pw_dump_output.stderr);
        bail!(
            "'pw-dump' failed with status: {}\n{}",
            pw_dump_output.status,
            stderr
        );
    }

    let all_nodes: Vec<PwNode> = serde_json::from_slice(&pw_dump_output.stdout)
        .context("Failed to parse JSON from 'pw-dump'.")?;

    let mut sources = Vec::new();
    let mut sinks = Vec::new();

    for node in all_nodes {
        let media_class = node.info.props.get("media.class").and_then(|v| v.as_str());
        let name = node.info.props.get("node.name").and_then(|v| v.as_str());
        let description = node
            .info
            .props
            .get("node.description")
            .and_then(|v| v.as_str());

        if let (Some(media_class), Some(name), Some(description)) = (media_class, name, description)
        {
            let device = AudioDevice {
                name: name.to_string(),
                description: description.to_string(),
            };
            if media_class.contains("Audio/Source") {
                sources.push(device);
            } else if media_class.contains("Audio/Sink") {
                sinks.push(device);
            }
        }
    }

    let wpctl_output = Command::new("wpctl")
        .arg("status")
        .output()
        .await
        .context("Failed to execute 'wpctl'.")?;

    let wpctl_stdout = String::from_utf8_lossy(&wpctl_output.stdout);
    let default_re = Regex::new(r"│\s+\*\s+\d+\.\s+(.*?)\s+\[vol:").unwrap();
    let mut default_sink_desc = None;
    let mut default_source_desc = None;
    let mut in_sinks = false;
    let mut in_sources = false;

    for line in wpctl_stdout.lines() {
        if line.contains("Sinks:") {
            in_sinks = true;
            in_sources = false;
            continue;
        }
        if line.contains("Sources:") {
            in_sources = true;
            in_sinks = false;
            continue;
        }
        if line.contains("Filters:") || line.contains("Streams:") {
            in_sinks = false;
            in_sources = false;
            continue;
        }

        if let Some(caps) = default_re.captures(line) {
            let desc = caps.get(1).unwrap().as_str().trim().to_string();
            if in_sinks {
                default_sink_desc = Some(desc.clone());
            }
            if in_sources {
                default_source_desc = Some(desc);
            }
        }
    }

    let mut settings = Settings::load().await?;

    if !sources.is_empty() {
        let default_source_name = default_source_desc
            .as_ref()
            .and_then(|desc| sources.iter().find(|s| &s.description == desc))
            .map(|s| s.name.clone())
            .unwrap_or_else(|| sources.first().map_or(String::new(), |s| s.name.clone()));

        let mut source_options = vec!["Use System Default".to_string()];
        source_options.extend(sources.iter().map(|s| s.description.clone()));

        let source_choice =
            Select::new("🎤 Select your microphone (audio source):", source_options).prompt()?;

        if source_choice == "Use System Default" {
            let default_device = default_source_desc
                .as_ref()
                .and_then(|desc| sources.iter().find(|s| &s.description == desc));
            settings.mic_node_name = default_device.map_or(default_source_name, |d| d.name.clone());
        } else {
            let selected_source = sources
                .iter()
                .find(|s| s.description == source_choice)
                .unwrap();
            settings.mic_node_name = selected_source.name.clone();
        }
    } else {
        println!("{}", "⚠ No audio sources found.".yellow());
    }

    if !sinks.is_empty() {
        let default_sink_name = default_sink_desc
            .as_ref()
            .and_then(|desc| sinks.iter().find(|s| &s.description == desc))
            .map(|s| s.name.clone())
            .unwrap_or_else(|| sinks.first().map_or(String::new(), |s| s.name.clone()));

        let mut sink_options = vec!["Use System Default".to_string()];
        sink_options.extend(sinks.iter().map(|s| s.description.clone()));

        let sink_choice = Select::new(
            "🎧 Select your background audio device (audio sink):",
            sink_options,
        )
        .prompt()?;

        if sink_choice == "Use System Default" {
            let default_device = default_sink_desc
                .as_ref()
                .and_then(|desc| sinks.iter().find(|s| &s.description == desc));
            settings.bg_node_name = default_device.map_or(default_sink_name, |d| d.name.clone());
        } else {
            let selected_sink = sinks.iter().find(|s| s.description == sink_choice).unwrap();
            settings.bg_node_name = selected_sink.name.clone();
        }
    } else {
        println!("{}", "⚠ No audio sinks found.".yellow());
    }

    settings.save().await?;
    println!(
        "\n{}",
        "✔ Audio settings updated successfully!".green().bold()
    );
    println!("  Mic set to: {}", settings.mic_node_name.cyan());
    println!("  Audio set to: {}", settings.bg_node_name.cyan());

    Ok(())
}
