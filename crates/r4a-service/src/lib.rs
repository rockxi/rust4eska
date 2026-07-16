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

    /// Enable a service. `env_vars` are written to a 0o600 env-file and loaded
    /// via EnvironmentFile (systemd) or EnvironmentVariables (launchd), so they
    /// never appear in the process command line.
    pub fn enable(
        &self,
        name: &str,
        description: &str,
        exec: &str,
        env_vars: &[(&str, &str)],
    ) -> Result<()> {
        match self {
            ServiceManager::Systemd => {
                let env_file_line = if !env_vars.is_empty() {
                    let env_dir = PathBuf::from("/etc/r4a");
                    fs::create_dir_all(&env_dir)?;
                    let env_path = env_dir.join(format!("{}.env", name));
                    let content: String = env_vars
                        .iter()
                        .map(|(k, v)| format!("{}={}\n", k, v))
                        .collect();
                    fs::write(&env_path, &content)?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&env_path, fs::Permissions::from_mode(0o600))?;
                    }
                    info!("Wrote env file to {} (mode 600)", env_path.display());
                    format!("EnvironmentFile={}\n", env_path.display())
                } else {
                    String::new()
                };

                let service_content = format!(
                    "[Unit]\nDescription={}\nAfter=network.target\n\n[Service]\nType=simple\n{}ExecStart={}\nRestart=always\nRestartSec=5\n\n[Install]\nWantedBy=multi-user.target\n",
                    description, env_file_line, exec
                );

                let path = PathBuf::from(format!("/etc/systemd/system/{}.service", name));
                fs::write(&path, service_content)?;
                info!("Wrote systemd service file to {}", path.display());

                run_command("systemctl", &["daemon-reload"])?;
                run_command("systemctl", &["enable", "--now", name])?;
                info!("Service {} enabled and started", name);
            }
            ServiceManager::Launchd => {
                let env_block = if !env_vars.is_empty() {
                    let pairs: String = env_vars
                        .iter()
                        .map(|(k, v)| format!("    <key>{}</key>\n    <string>{}</string>", k, v))
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!(
                        "    <key>EnvironmentVariables</key>\n    <dict>\n{}\n    </dict>\n",
                        pairs
                    )
                } else {
                    String::new()
                };

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
{}    <key>RunAtLoad</key>
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
                    env_block,
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
                let env_path = PathBuf::from(format!("/etc/r4a/{}.env", name));
                if env_path.exists() {
                    let _ = fs::remove_file(&env_path);
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
