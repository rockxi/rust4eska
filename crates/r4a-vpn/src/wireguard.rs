use anyhow::{Context, Result};
use std::process::Command;

pub struct KeyPair {
    pub private: String,
    pub public: String,
}

/// Validates a WireGuard public key: must be exactly 44 base64 characters
/// (32 bytes encoded). Rejects newlines and non-base64 characters that would
/// allow injecting additional directives into the WireGuard config file.
pub fn validate_wg_pubkey(key: &str) -> Result<()> {
    if key.len() != 44 {
        anyhow::bail!("WireGuard public key must be 44 characters (base64-encoded 32 bytes), got {}", key.len());
    }
    if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=') {
        anyhow::bail!("WireGuard public key contains invalid characters");
    }
    Ok(())
}

/// Validates a node name: alphanumeric, hyphens, underscores, dots only.
/// Prevents newline injection into WireGuard config and systemd unit files.
pub fn validate_node_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        anyhow::bail!("Node name must be 1–64 characters");
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.') {
        anyhow::bail!("Node name contains invalid characters (allowed: alphanumeric, -, _, .)");
    }
    Ok(())
}

/// Validates a host:port endpoint string. Rejects newlines, carriage returns,
/// and null bytes that could inject config directives.
pub fn validate_endpoint(endpoint: &str) -> Result<()> {
    if endpoint.contains('\n') || endpoint.contains('\r') || endpoint.contains('\0') {
        anyhow::bail!("Endpoint contains forbidden control characters");
    }
    if !endpoint.contains(':') {
        anyhow::bail!("Endpoint must be in host:port format");
    }
    Ok(())
}

pub fn generate_keypair() -> Result<KeyPair> {
    let private = run("wg", &["genkey"])?;
    let public = run_with_stdin("wg", &["pubkey"], &private)?;
    Ok(KeyPair { private, public })
}

pub fn setup_master(private_key: &str, vpn_ip: &str, listen_port: u16) -> Result<()> {
    setup_master_with_peers(private_key, vpn_ip, listen_port, &[])
}

pub fn setup_master_with_peers(
    private_key: &str,
    vpn_ip: &str,
    listen_port: u16,
    peers: &[(&str, &str)], // (pub_key, vpn_ip)
) -> Result<()> {
    // Validate all peer fields before writing the config file (C-1)
    for (pub_key, peer_ip) in peers {
        validate_wg_pubkey(pub_key)?;
        if peer_ip.contains('\n') || peer_ip.contains('\r') || peer_ip.contains('\0') {
            anyhow::bail!("peer VPN IP contains forbidden control characters");
        }
    }

    let mut conf = format!(
        "[Interface]\nPrivateKey = {private_key}\nAddress = {vpn_ip}/24\nListenPort = {listen_port}\n"
    );
    for (pub_key, peer_ip) in peers {
        conf.push_str(&format!(
            "\n[Peer]\nPublicKey = {pub_key}\nAllowedIPs = {peer_ip}/32\nPersistentKeepalive = 25\n"
        ));
    }
    write_conf(&conf)?;
    bring_up()?;
    tracing::info!("WireGuard master interface up at {vpn_ip} ({} peer(s))", peers.len());
    Ok(())
}

pub fn setup_agent(
    private_key: &str,
    vpn_ip: &str,
    master_pub: &str,
    master_endpoint: &str,
) -> Result<()> {
    // Validate before writing the config file (C-1)
    validate_wg_pubkey(master_pub)?;
    validate_endpoint(master_endpoint)?;

    let conf = format!(
        "[Interface]\nPrivateKey = {private_key}\nAddress = {vpn_ip}/32\n\n\
         [Peer]\nPublicKey = {master_pub}\nEndpoint = {master_endpoint}\n\
         AllowedIPs = 10.42.0.0/24\nPersistentKeepalive = 25\n"
    );
    write_conf(&conf)?;
    bring_up()?;
    tracing::info!("WireGuard agent interface up, peer = {master_endpoint}");
    Ok(())
}

pub fn add_peer(pubkey: &str, vpn_ip: &str) -> Result<()> {
    validate_wg_pubkey(pubkey)?;
    if vpn_ip.contains('\n') || vpn_ip.contains('\r') || vpn_ip.contains('\0') {
        anyhow::bail!("VPN IP contains forbidden control characters");
    }
    run("wg", &["set", "wg0", "peer", pubkey, "allowed-ips", &format!("{vpn_ip}/32"), "persistent-keepalive", "25"])?;
    Ok(())
}

pub fn remove_peer(pubkey: &str) -> Result<()> {
    validate_wg_pubkey(pubkey)?;
    run("wg", &["set", "wg0", "peer", pubkey, "remove"])?;
    Ok(())
}

fn write_conf(conf: &str) -> Result<()> {
    let path = "/etc/wireguard/wg0.conf";
    let tmp_path = format!("{}.tmp", path);

    std::fs::create_dir_all("/etc/wireguard")
        .context("create /etc/wireguard/ (run as root?)")?;

    std::fs::write(&tmp_path, conf)
        .context("write /etc/wireguard/wg0.conf.tmp (run as root?)")?;
    
    run("chmod", &["600", &tmp_path])?;
    
    std::fs::rename(&tmp_path, path)
        .context("rename /etc/wireguard/wg0.conf.tmp to wg0.conf")?;
    
    Ok(())
}

fn bring_up() -> Result<()> {
    // down first to avoid "already exists" errors on re-runs
    let _ = Command::new("wg-quick").args(["down", "wg0"]).output();
    run("wg-quick", &["up", "wg0"])?;
    Ok(())
}

fn run(cmd: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("exec {cmd}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "{cmd} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn run_with_stdin(cmd: &str, args: &[&str], input: &str) -> Result<String> {
    use std::io::Write;
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn {cmd}"))?;
    child.stdin.as_mut().unwrap().write_all(input.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        anyhow::bail!("{cmd} failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
