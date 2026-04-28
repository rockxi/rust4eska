use anyhow::{anyhow, Result};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tracing::info;

pub enum ServiceManager {
    Systemd,
    Launchd,
}

impl ServiceManager {
    pub fn detect() -> Result<Self> {
        #[cfg(target_os = "linux")]
        return Ok(ServiceManager::Systemd);

        #[cfg(target_os = "macos")]
        return Ok(ServiceManager::Launchd);

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        Err(anyhow!("Unsupported OS for service management"))
    }

    pub fn enable(&self, name: &str, description: &str, exec: &str) -> Result<()> {
        match self {
            ServiceManager::Systemd => {
                let service_content = format!(
                    r#"[Unit]
Description={}
After=network.target

[Service]
Type=simple
ExecStart={}
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
"#,
                    description, exec
                );

                let path = PathBuf::from(format!("/etc/systemd/system/{}.service", name));
                fs::write(&path, service_content)?;
                info!("Wrote systemd service file to {}", path.display());

                run_command("systemctl", &["daemon-reload"])?;
                run_command("systemctl", &["enable", "--now", name])?;
                info!("Service {} enabled and started", name);
            }
            ServiceManager::Launchd => {
                let plist_content = format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
        {}
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/{}.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/{}.log</string>
</dict>
</plist>
"#,
                    name,
                    exec.split_whitespace()
                        .map(|s| format!("<string>{}</string>", s))
                        .collect::<Vec<_>>()
                        .join("\n        "),
                    name,
                    name
                );

                let path = PathBuf::from(format!("/Library/LaunchDaemons/{}.plist", name));
                fs::write(&path, plist_content)?;
                info!("Wrote launchd plist file to {}", path.display());

                run_command("launchctl", &["load", "-w", &path.to_string_lossy()])?;
                info!("Service {} loaded and started", name);
            }
        }
        Ok(())
    }

    pub fn disable(&self, name: &str) -> Result<()> {
        match self {
            ServiceManager::Systemd => {
                let _ = run_command("systemctl", &["disable", "--now", name]);
                let path = PathBuf::from(format!("/etc/systemd/system/{}.service", name));
                if path.exists() {
                    fs::remove_file(&path)?;
                }
                let _ = run_command("systemctl", &["daemon-reload"]);
                info!("Service {} disabled and removed", name);
            }
            ServiceManager::Launchd => {
                let path = PathBuf::from(format!("/Library/LaunchDaemons/{}.plist", name));
                if path.exists() {
                    let _ = run_command("launchctl", &["unload", "-w", &path.to_string_lossy()]);
                    fs::remove_file(&path)?;
                }
                info!("Service {} unloaded and removed", name);
            }
        }
        Ok(())
    }
}

fn run_command(cmd: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(cmd).args(args).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("Command {} failed with status {}", cmd, status))
    }
}
