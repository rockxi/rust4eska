use anyhow::{Context, Result};
use std::process::Command;

pub struct KeyPair {
    pub private: String,
    pub public: String,
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
    let mut conf = format!(
        "[Interface]\nPrivateKey = {private_key}\nAddress = {vpn_ip}/24\nListenPort = {listen_port}\n"
    );
    for (pub_key, peer_ip) in peers {
        conf.push_str(&format!(
            "\n[Peer]\nPublicKey = {pub_key}\nAllowedIPs = {peer_ip}/32\n"
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

fn write_conf(conf: &str) -> Result<()> {
    std::fs::write("/etc/wireguard/wg0.conf", conf)
        .context("write /etc/wireguard/wg0.conf (run as root?)")?;
    run("chmod", &["600", "/etc/wireguard/wg0.conf"])?;
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
