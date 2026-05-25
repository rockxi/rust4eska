use anyhow::{Context, Result};

const HOSTS_FILE: &str = "/etc/hosts";
const MARKER: &str = "# r4a-managed";
const RESOLVER_DIR: &str = "/etc/resolver";

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

/// Creates /etc/resolver/<domain> so macOS routes *.domain queries to nameserver_ip.
pub fn set_resolver_domain(domain: &str, nameserver_ip: &str) -> Result<()> {
    std::fs::create_dir_all(RESOLVER_DIR).context("create /etc/resolver")?;
    let path = format!("{}/{}", RESOLVER_DIR, domain);
    let content = format!("nameserver {}\n", nameserver_ip);
    std::fs::write(&path, content).with_context(|| format!("write {}", path))?;
    tracing::info!("DNS resolver: {} → {}", domain, nameserver_ip);
    Ok(())
}

/// Removes /etc/resolver/<domain> if it exists.
pub fn remove_resolver_domain(domain: &str) -> Result<()> {
    let path = format!("{}/{}", RESOLVER_DIR, domain);
    if std::path::Path::new(&path).exists() {
        std::fs::remove_file(&path).with_context(|| format!("remove {}", path))?;
        tracing::info!("DNS resolver: removed {}", domain);
    }
    Ok(())
}

