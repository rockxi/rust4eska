use anyhow::{Context, Result};

const HOSTS_FILE: &str = "/etc/hosts";
const MARKER: &str = "# r4a-managed";

/// Добавляет/обновляет запись в /etc/hosts. Идемпотентно.
pub fn set_hosts_entry(ip: &str, hostname: &str) -> Result<()> {
    let content = std::fs::read_to_string(HOSTS_FILE).context("read /etc/hosts")?;
    let entry = format!("{ip} {hostname} {MARKER}");

    // Удаляем старую запись с этим hostname (если есть)
    let filtered: String = content
        .lines()
        .filter(|l| !(l.contains(hostname) && l.contains(MARKER)))
        .map(|l| format!("{l}\n"))
        .collect();

    let new_content = format!("{filtered}{entry}\n");
    std::fs::write(HOSTS_FILE, new_content).context("write /etc/hosts")?;
    tracing::info!("Set {hostname} -> {ip} in /etc/hosts");
    Ok(())
}
