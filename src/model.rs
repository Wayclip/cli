use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::fmt;

#[derive(Clone)]
pub struct ClipDisplay {
    pub name: String,
    pub display_string: String,
}

impl fmt::Display for ClipDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_string)
    }
}

#[derive(Parser)]
#[command(
    name = "wayclip-cli",
    version,
    about = "Capture and replay your screen instantly on Linux. Built for the modern desktop with Wayland and PipeWire."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
    #[arg(long, hide = true)]
    pub debug: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    Daemon {
        #[command(subcommand)]
        action: DaemonCommand,
    },
    Save,
    List {
        #[arg(short = 't', long = "timestamp")]
        timestamp: bool,
        #[arg(short = 'l', long = "length")]
        length: bool,
        #[arg(short = 'r', long = "reverse")]
        reverse: bool,
        #[arg(short = 's', long = "size")]
        size: bool,
        #[arg(short = 'e', long = "extra")]
        extra: bool,
    },
    Manage,
    Config {
        #[arg(short = 'e', long = "editor")]
        editor: Option<String>,
    },
    View {
        name: String,
        #[arg(short = 'p', long = "player")]
        player: Option<String>,
    },
    Delete {
        name: String,
    },
    Rename {
        name: String,
    },
    Edit {
        name: String,
        start_time: String,
        end_time: String,
        #[arg(default_value_t = false)]
        disable_audio: bool,
    },
    Login {
        #[arg(short = 'b', long = "browser")]
        browser: Option<String>,
    },
    Logout,
    Me,
    #[command(name = "2fa")]
    TwoFactorAuth {
        #[command(subcommand)]
        action: TwoFactorCommand,
    },
    Share {
        #[arg(help = "Name of the clip to share")]
        name: String,
    },
    Like {
        #[arg(help = "Name of the local clip to like/unlike")]
        name: String,
    },
    Url {
        #[arg(help = "Name of the hosted clip to get the URL for")]
        name: String,
    },
    Open {
        #[arg(help = "Name of the hosted clip to open in a browser")]
        name: String,
    },
    Audio,
}

#[derive(Subcommand)]
pub enum AutostartAction {
    On,
    Off,
}

#[derive(Subcommand)]
pub enum DaemonCommand {
    Start,
    Stop,
    Restart,
    Status,
    Logs,
    Autostart {
        #[command(subcommand)]
        action: AutostartAction,
    },
}

#[derive(Subcommand)]
pub enum TwoFactorCommand {
    Setup,
    Status,
}

pub const LOCAL_PORT: u16 = 54321;

pub enum AuthCallbackResult {
    Success(String),
    TwoFactor(String),
    Error(String),
}

#[derive(serde::Deserialize, Debug)]
pub struct PwNode {
    #[serde(default)]
    pub info: PwNodeInfo,
}

#[derive(serde::Deserialize, Debug, Default)]
pub struct PwNodeInfo {
    #[serde(default)]
    pub props: HashMap<String, serde_json::Value>,
}

#[derive(Clone)]
pub struct AudioDevice {
    pub name: String,
    pub description: String,
}
