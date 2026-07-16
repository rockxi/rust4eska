use anyhow::{Context, Result};
use std::process::Command;

pub struct KeyPair {
    pub private: String,
    pub public: String,
}

pub fn validate_wg_pubkey(key: &str) -> Result<()> {
    if key.len() != 44 {
        anyhow::bail!(
            "WireGuard public key must be 44 characters, got {}",
            key.len()
        );
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
    {
        anyhow::bail!("WireGuard public key contains invalid characters");
    }
    Ok(())
}

pub fn validate_node_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        anyhow::bail!("Node name must be 1–64 characters");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        anyhow::bail!("Node name contains invalid characters (allowed: alphanumeric, -, _, .)");
    }
    Ok(())
}

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
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use rand::rngs::OsRng;
    use x25519_dalek::{PublicKey, StaticSecret};

    let secret = StaticSecret::random_from_rng(OsRng);
    let public = PublicKey::from(&secret);
    Ok(KeyPair {
        private: STANDARD.encode(secret.to_bytes()),
        public: STANDARD.encode(public.to_bytes()),
    })
}

pub fn setup_master(private_key: &str, vpn_ip: &str, listen_port: u16) -> Result<()> {
    setup_master_with_peers(private_key, vpn_ip, listen_port, &[])
}

pub fn setup_master_with_peers(
    private_key: &str,
    vpn_ip: &str,
    listen_port: u16,
    peers: &[(&str, &str)],
) -> Result<()> {
    for (pub_key, peer_ip) in peers {
        validate_wg_pubkey(pub_key)?;
        if peer_ip.contains('\n') || peer_ip.contains('\r') || peer_ip.contains('\0') {
            anyhow::bail!("peer VPN IP contains forbidden control characters");
        }
    }
    let mut conf = format!(
        "[Interface]\nPrivateKey = {private_key}\nAddress = {vpn_ip}/24\nListenPort = {listen_port}\n"
    );
    // Форвардинг агент↔агент через хаб: wg-quick сам ip_forward не включает.
    // `|| true` — в docker sysctl может быть запрещён (там форвардинг включается
    // через compose sysctls), а ошибка PostUp фатальна для wg-quick.
    if cfg!(target_os = "linux") {
        conf.push_str("PostUp = sysctl -w net.ipv4.ip_forward=1 || true\n");
    }
    for (pub_key, peer_ip) in peers {
        conf.push_str(&format!(
            "\n[Peer]\nPublicKey = {pub_key}\nAllowedIPs = {peer_ip}/32\nPersistentKeepalive = 25\n"
        ));
    }
    write_conf(&conf)?;
    bring_up()?;
    tracing::info!(
        "WireGuard master interface up at {vpn_ip} ({} peer(s))",
        peers.len()
    );
    Ok(())
}

pub fn setup_agent(
    private_key: &str,
    vpn_ip: &str,
    master_pub: &str,
    master_endpoint: &str,
) -> Result<()> {
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

/// Имя активного WG-интерфейса: на Linux всегда wg0, на macOS — utunN из state-файла.
pub fn iface_name() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(data) = std::fs::read_to_string(MACOS_WG_STATE) {
            if let Ok(state) = serde_json::from_str::<MacosWgState>(&data) {
                return state.iface;
            }
        }
    }
    "wg0".to_string()
}

fn wg_bin() -> String {
    if cfg!(target_os = "macos") {
        for p in ["/opt/homebrew/bin/wg", "/usr/local/bin/wg"] {
            if std::path::Path::new(p).exists() {
                return p.to_string();
            }
        }
    }
    "wg".to_string()
}

pub fn add_peer(pubkey: &str, vpn_ip: &str) -> Result<()> {
    validate_wg_pubkey(pubkey)?;
    if vpn_ip.contains('\n') || vpn_ip.contains('\r') || vpn_ip.contains('\0') {
        anyhow::bail!("VPN IP contains forbidden control characters");
    }
    let iface = iface_name();
    run(
        &wg_bin(),
        &[
            "set",
            &iface,
            "peer",
            pubkey,
            "allowed-ips",
            &format!("{vpn_ip}/32"),
            "persistent-keepalive",
            "25",
        ],
    )?;
    Ok(())
}

/// Добавить peer'а с явным endpoint (P2P-туннель агент↔агент).
pub fn add_peer_with_endpoint(pubkey: &str, vpn_ip: &str, endpoint: &str) -> Result<()> {
    validate_wg_pubkey(pubkey)?;
    validate_endpoint(endpoint)?;
    if vpn_ip.contains('\n') || vpn_ip.contains('\r') || vpn_ip.contains('\0') {
        anyhow::bail!("VPN IP contains forbidden control characters");
    }
    let iface = iface_name();
    run(
        &wg_bin(),
        &[
            "set",
            &iface,
            "peer",
            pubkey,
            "endpoint",
            endpoint,
            "allowed-ips",
            &format!("{vpn_ip}/32"),
            "persistent-keepalive",
            "25",
        ],
    )?;
    Ok(())
}

pub fn remove_peer(pubkey: &str) -> Result<()> {
    validate_wg_pubkey(pubkey)?;
    let iface = iface_name();
    run(&wg_bin(), &["set", &iface, "peer", pubkey, "remove"])?;
    Ok(())
}

/// pubkey → ip:port, наблюдаемый ядром WG (реальный адрес peer'а после NAT).
pub fn observed_endpoints() -> Result<std::collections::HashMap<String, String>> {
    let iface = iface_name();
    let out = run(&wg_bin(), &["show", &iface, "endpoints"])?;
    let mut map = std::collections::HashMap::new();
    for line in out.lines() {
        let mut parts = line.split_whitespace();
        if let (Some(pk), Some(ep)) = (parts.next(), parts.next()) {
            if ep != "(none)" {
                map.insert(pk.to_string(), ep.to_string());
            }
        }
    }
    Ok(map)
}

/// pubkey → unix-время последнего handshake (0 = ни одного).
pub fn latest_handshakes() -> Result<std::collections::HashMap<String, u64>> {
    let iface = iface_name();
    let out = run(&wg_bin(), &["show", &iface, "latest-handshakes"])?;
    let mut map = std::collections::HashMap::new();
    for line in out.lines() {
        let mut parts = line.split_whitespace();
        if let (Some(pk), Some(ts)) = (parts.next(), parts.next()) {
            if let Ok(ts) = ts.parse::<u64>() {
                map.insert(pk.to_string(), ts);
            }
        }
    }
    Ok(map)
}

/// Bring down the WireGuard interface (platform-aware).
pub fn bring_down() {
    #[cfg(target_os = "macos")]
    bring_down_macos();
    #[cfg(not(target_os = "macos"))]
    {
        let conf = wg_conf_path();
        let _ = Command::new("wg-quick").args(["down", conf]).output();
    }
}

// ── Config path ───────────────────────────────────────────────────────────────

pub fn wg_conf_path() -> &'static str {
    if cfg!(target_os = "macos") {
        "/tmp/r4a-wg/wg0.conf"
    } else {
        "/etc/wireguard/wg0.conf"
    }
}

pub fn wg_quick_bin() -> String {
    if cfg!(target_os = "macos") {
        if std::path::Path::new("/opt/homebrew/bin/wg-quick").exists() {
            "/opt/homebrew/bin/wg-quick".to_string()
        } else if std::path::Path::new("/usr/local/bin/wg-quick").exists() {
            "/usr/local/bin/wg-quick".to_string()
        } else {
            "wg-quick".to_string()
        }
    } else {
        "wg-quick".to_string()
    }
}

// ── Linux bring_up / bring_down ───────────────────────────────────────────────

fn bring_up() -> Result<()> {
    let conf = wg_conf_path();
    #[cfg(target_os = "macos")]
    return bring_up_macos(conf);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = Command::new("wg-quick").args(["down", conf]).output();
        run("wg-quick", &["up", conf])?;
        Ok(())
    }
}

// ── macOS WireGuard (wireguard-go + wg setconf + ifconfig + route) ────────────

#[cfg(target_os = "macos")]
const MACOS_WG_STATE: &str = "/tmp/r4a-wg-state.json";

#[cfg(target_os = "macos")]
#[derive(serde::Serialize, serde::Deserialize)]
struct MacosWgState {
    iface: String,
    pid: u32,
}

#[cfg(target_os = "macos")]
fn find_bin(candidates: &[&str]) -> Result<String> {
    for p in candidates {
        if std::path::Path::new(p).exists() {
            return Ok(p.to_string());
        }
    }
    anyhow::bail!("none of {:?} found", candidates)
}

#[cfg(target_os = "macos")]
fn find_free_utun() -> Result<String> {
    for i in 10..30u32 {
        let name = format!("utun{}", i);
        let in_use = Command::new("ifconfig")
            .arg(&name)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !in_use {
            return Ok(name);
        }
    }
    anyhow::bail!("no free utun interface (utun10-utun29 all in use)")
}

#[cfg(target_os = "macos")]
fn wait_for_iface(name: &str) -> Result<()> {
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let up = Command::new("ifconfig")
            .arg(name)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if up {
            return Ok(());
        }
    }
    anyhow::bail!("interface {} did not appear within 5s", name)
}

#[cfg(target_os = "macos")]
fn bring_down_macos() {
    if let Ok(data) = std::fs::read_to_string(MACOS_WG_STATE) {
        if let Ok(state) = serde_json::from_str::<MacosWgState>(&data) {
            let _ = Command::new("kill")
                .args(["-9", &state.pid.to_string()])
                .output();
            let _ = Command::new("route")
                .args(["delete", "-net", "10.42.0.0/24"])
                .output();
            // Remove interface if still present
            let _ = Command::new("ifconfig")
                .args([&state.iface, "down"])
                .output();
        }
    }
    let _ = std::fs::remove_file(MACOS_WG_STATE);
}

#[cfg(target_os = "macos")]
fn bring_up_macos(conf_path: &str) -> Result<()> {
    // Tear down any existing r4a WireGuard session
    bring_down_macos();

    let wireguard_go = find_bin(&[
        "/opt/homebrew/bin/wireguard-go",
        "/usr/local/bin/wireguard-go",
    ])
    .context("wireguard-go not found — install: brew install wireguard-go")?;

    let wg_bin = find_bin(&["/opt/homebrew/bin/wg", "/usr/local/bin/wg"])
        .context("wg not found — install: brew install wireguard-tools")?;

    let iface = find_free_utun()?;

    // Start wireguard-go — it creates the utun interface and daemonizes
    let child = Command::new(&wireguard_go)
        .arg(&iface)
        .spawn()
        .with_context(|| format!("spawn wireguard-go {}", iface))?;

    let pid = child.id();

    // Persist state so bring_down can kill the process later
    let state = MacosWgState {
        iface: iface.clone(),
        pid,
    };
    std::fs::create_dir_all("/tmp/r4a-wg").ok();
    let _ = std::fs::write(
        MACOS_WG_STATE,
        serde_json::to_string(&state).unwrap_or_default(),
    );

    // Wait for the utun interface to appear
    wait_for_iface(&iface).with_context(|| format!("wireguard-go failed to create {}", iface))?;

    // `wg setconf` (unlike wg-quick) only understands PrivateKey/ListenPort/FwMark
    // and [Peer] fields — Address= is a wg-quick-only extension and must be
    // stripped before loading, or it fails with "Line unrecognized".
    let conf_text = std::fs::read_to_string(conf_path).context("read wg conf")?;
    let setconf_text: String = conf_text
        .lines()
        .filter(|l| !l.trim_start().starts_with("Address"))
        .collect::<Vec<_>>()
        .join("\n");
    let setconf_path = format!("{}.setconf", conf_path);
    std::fs::write(&setconf_path, &setconf_text).context("write stripped wg conf")?;

    run(&wg_bin, &["setconf", &iface, &setconf_path]).context("wg setconf failed")?;

    // Parse the Address= from the original conf file to assign to the interface
    let vpn_ip = conf_text
        .lines()
        .find(|l| l.trim_start().starts_with("Address"))
        .and_then(|l| l.splitn(2, '=').nth(1))
        .map(|s| s.trim().split('/').next().unwrap_or("").to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("Address= not found in wg conf"))?;

    // Assign the VPN IP to the interface (point-to-point)
    run("ifconfig", &[&iface, "inet", &vpn_ip, &vpn_ip]).context("ifconfig failed")?;

    // Add route for the entire VPN subnet through this interface
    let _ = Command::new("route")
        .args(["delete", "-net", "10.42.0.0/24"])
        .output();
    run("route", &["add", "-net", "10.42.0.0/24", &vpn_ip]).context("route add failed")?;

    tracing::info!("WireGuard up on {} ip={}", iface, vpn_ip);
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn write_conf(conf: &str) -> Result<()> {
    let path = wg_conf_path();
    let tmp_path = format!("{}.tmp", path);
    let dir = std::path::Path::new(path).parent().unwrap();

    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;

    std::fs::write(&tmp_path, conf).with_context(|| format!("write {}", tmp_path))?;

    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600));
    }

    std::fs::rename(&tmp_path, path).context("rename wg conf")?;
    Ok(())
}

fn run(cmd: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("exec {cmd}"))?;
    if !out.status.success() {
        anyhow::bail!("{cmd} failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
