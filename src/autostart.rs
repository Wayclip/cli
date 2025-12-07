use anyhow::{Context, Result, bail};
use colored::*;
use inquire::Confirm;
use tokio::process::Command;
use wayclip_core::settings::Settings;
use which::which;

pub async fn handle_autostart_on() -> Result<()> {
    println!("○ Enabling autostart using systemd user service...");

    let daemon_path = which("wayclip-daemon")
        .context("Could not find 'wayclip-daemon' executable in your PATH. Please ensure it is installed correctly.")?;
    println!("  Daemon found at: {}", daemon_path.display());

    let service_content = format!(
        r#"[Unit]
Description=Wayclip Daemon
After=graphical.target pipewire.service pipewire-pulse.service
Wants=graphical.target
StartLimitBurst=5
StartLimitIntervalSec=60

[Service]
ExecStart={}
Restart=on-failure
RestartSec=5
Type=notify
TimeoutStartSec=90
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target
"#,
        daemon_path.to_str().unwrap()
    );

    let config_dir = Settings::config_path();

    tokio::fs::create_dir_all(&config_dir)
        .await
        .context(format!(
            "Failed to create systemd user directory at {}",
            config_dir.display()
        ))?;

    let service_path = config_dir.join("wayclip-daemon.service");

    if service_path.exists() {
        let overwrite = Confirm::new("Service file already exists. Overwrite?")
            .with_default(false)
            .prompt()?;
        if !overwrite {
            println!("{}", "○ Autostart setup cancelled.".yellow());
            return Ok(());
        }
    }

    tokio::fs::write(&service_path, service_content)
        .await
        .context(format!(
            "Failed to write service file to {}",
            service_path.display()
        ))?;
    println!("✔ Service file created at {}", service_path.display());

    println!("○ Reloading systemd user daemon...");
    let reload_status = Command::new("systemctl")
        .arg("--user")
        .arg("daemon-reload")
        .status()
        .await
        .context("Failed to execute 'systemctl --user daemon-reload'. Is systemd running?")?;

    if !reload_status.success() {
        bail!("'systemctl --user daemon-reload' failed. Please run it manually.");
    }

    println!("○ Enabling and starting the service...");
    let enable_output = Command::new("systemctl")
        .arg("--user")
        .arg("enable")
        .arg("--now")
        .arg("wayclip-daemon.service")
        .output()
        .await
        .context("Failed to execute 'systemctl --user enable --now'.")?;

    if !enable_output.status.success() {
        let stderr = String::from_utf8_lossy(&enable_output.stderr);
        bail!("'systemctl --user enable --now' failed. Please run it manually.\nError: {stderr}",);
    }

    println!("{}", "✔ Autostart enabled successfully!".green().bold());
    println!("  The Wayclip daemon will now start automatically when you log in.");
    println!(
        "  To disable it, run: {}",
        "wayclip daemon autostart off".italic()
    );

    Ok(())
}

pub async fn handle_autostart_off() -> Result<()> {
    println!("○ Disabling autostart using systemd user service...");

    let service_name = "wayclip-daemon.service";

    let config_dir = Settings::config_path();
    let service_path = config_dir.join(service_name);

    if !service_path.exists() {
        println!(
            "{}",
            "○ Autostart is already disabled (service file not found).".yellow()
        );
        return Ok(());
    }

    println!("○ Disabling and stopping the service...");
    let disable_output = Command::new("systemctl")
        .arg("--user")
        .arg("disable")
        .arg("--now")
        .arg(service_name)
        .output()
        .await
        .context("Failed to execute 'systemctl --user disable --now'.")?;

    if !disable_output.status.success() {
        let stderr = String::from_utf8_lossy(&disable_output.stderr);
        if !stderr.contains("unit file does not exist")
            && !stderr.contains("No such file or directory")
        {
            bail!("'systemctl --user disable --now' failed.\nError: {stderr}",);
        }
    }

    println!("{}", "✔ Autostart disabled successfully!".green().bold());

    let delete_file = Confirm::new("Do you want to remove the systemd service file?")
        .with_default(false)
        .prompt()?;

    if delete_file {
        tokio::fs::remove_file(&service_path)
            .await
            .context(format!(
                "Failed to remove service file at {}",
                service_path.display()
            ))?;
        println!("✔ Service file removed.");
        Command::new("systemctl")
            .arg("--user")
            .arg("daemon-reload")
            .status()
            .await?;
    }

    Ok(())
}
