use anyhow::{Context, Result};

const HOSTS_FILE: &str = "/etc/hosts";
const MARKER: &str = "# r4a-managed";

/// Добавляет/обновляет записи в /etc/hosts. Идемпотентно. Атомарно.
pub fn set_hosts_entries(ips: &[&str], hostname: &str) -> Result<()> {
    let content = std::fs::read_to_string(HOSTS_FILE).context("read /etc/hosts")?;

    let filtered: String = content
        .lines()
        .filter(|l| !(l.contains(hostname) && l.contains(MARKER)))
        .map(|l| format!("{l}\n"))
        .collect();

    let mut ips_sorted = ips.to_vec();
    ips_sorted.sort();

    let mut new_entries = String::new();
    for ip in ips_sorted {
        new_entries.push_str(&format!("{ip} {hostname} {MARKER}\n"));
    }

    let new_content = format!("{filtered}{new_entries}");
    
    if content.lines().eq(new_content.lines()) {
        return Ok(());
    }

    std::fs::write(HOSTS_FILE, &new_content).context("write /etc/hosts")?;
    tracing::info!("Updated /etc/hosts: {} IPs for {}", ips.len(), hostname);
    Ok(())
}

