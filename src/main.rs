use crate::audio::handle_audio;
use crate::auth::{handle_2fa_setup, handle_2fa_status, handle_login, handle_logout};
use crate::autostart::{handle_autostart_off, handle_autostart_on};
use crate::clipboard::copy_to_clipboard;
use crate::config::handle_config;
use crate::delete::handle_delete;
use crate::edit::handle_edit;
use crate::like::handle_like;
use crate::list::handle_list;
use crate::manage::handle_manage;
use crate::model::{AutostartAction, Cli, Commands, DaemonCommand, TwoFactorCommand};
use crate::rename::handle_rename;
use crate::save::handle_save;
use crate::social::{handle_me, handle_share};
use crate::url::{handle_open, handle_url};
use crate::view::handle_view;
use anyhow::Result;
use clap::Parser;
use colored::*;
use std::process::ExitCode;
use wayclip_core::control::DaemonManager;

pub mod audio;
pub mod auth;
pub mod autostart;
pub mod clipboard;
pub mod config;
pub mod delete;
pub mod edit;
pub mod like;
pub mod list;
pub mod manage;
pub mod model;
pub mod rename;
pub mod save;
pub mod social;
pub mod unified_clip;
pub mod url;
pub mod validate;
pub mod view;

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(e) = run().await {
        eprintln!("{} {:#}", "✗ Error:".red().bold(), e);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    if cli.debug {
        println!("{}", "○ Debug mode is ON".yellow());
    }

    match &cli.command {
        Commands::Login { browser } => handle_login(browser).await?,
        Commands::Logout => handle_logout().await?,
        Commands::Me => handle_me().await?,
        Commands::TwoFactorAuth { action } => match action {
            TwoFactorCommand::Setup => handle_2fa_setup().await?,
            TwoFactorCommand::Status => handle_2fa_status().await?,
        },
        Commands::Share { name } => handle_share(name).await?,
        Commands::Save => handle_save().await?,
        Commands::List { .. } => handle_list(&cli.command).await?,
        Commands::Manage => handle_manage().await?,
        Commands::Config { editor } => handle_config(editor.as_deref()).await?,
        Commands::View { name, player } => handle_view(name, player.as_deref()).await?,
        Commands::Rename { name } => handle_rename(name).await?,
        Commands::Delete { name } => handle_delete(name).await?,
        Commands::Edit {
            name,
            start_time,
            end_time,
            disable_audio,
        } => handle_edit(name, start_time, end_time, disable_audio).await?,
        Commands::Like { name } => handle_like(name).await?,
        Commands::Url { name } => handle_url(name).await?,
        Commands::Open { name } => handle_open(name).await?,
        Commands::Daemon { action } => {
            let manager = DaemonManager::new();
            match action {
                DaemonCommand::Start => manager.start().await?,
                DaemonCommand::Stop => manager.stop().await?,
                DaemonCommand::Restart => manager.restart().await?,
                DaemonCommand::Logs => manager.logs().await?,
                DaemonCommand::Status => {
                    manager.status().await?;
                }
                DaemonCommand::Autostart { action } => match action {
                    AutostartAction::On => handle_autostart_on().await?,
                    AutostartAction::Off => handle_autostart_off().await?,
                },
            }
        }
        Commands::Audio => handle_audio().await?,
    }

    Ok(())
}
