use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use sysinfo::System;
use tracing::{info, warn};

fn state_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".r4a-agent")
}

#[derive(Serialize, Deserialize)]
struct Identity {
    private_key: String,
    public_key: String,
}

fn load_identity() -> Result<Identity> {
    let path = state_dir().join("identity.json");
    if path.exists() {
        let data = std::fs::read_to_string(&path)?;
        let id: Identity = serde_json::from_str(&data)?;
        info!("Loaded existing identity, public key: {}", id.public_key);
        return Ok(id);
    }
    info!("Generating new WireGuard keypair...");
    let kp = r4a_vpn::wireguard::generate_keypair()?;
    let id = Identity {
        private_key: kp.private,
        public_key: kp.public,
    };
    std::fs::create_dir_all(state_dir())?;
    std::fs::write(&path, serde_json::to_string_pretty(&id)?)?;
    info!("Saved identity to {}", path.display());
    Ok(id)
}

#[derive(Parser)]
#[command(name = "r4a-agent", about = "r4a Agent Node")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Подключиться к master-ноде
    Connect {
        #[arg(long)]
        master: String,
        /// Имя ноды (по умолчанию — hostname)
        #[arg(long)]
        name: Option<String>,
    },
    /// Управление системным сервисом (systemd/launchd)
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Subcommand)]
enum ServiceAction {
    /// Установить и запустить сервис
    Enable {
        #[arg(long)]
        master: String,
    },
    /// Остановить и удалить сервис
    Disable,
}

#[derive(Serialize)]
struct JoinRequest {
    pub_key: String,
    name: String,
    role: String,
}

#[derive(Deserialize, Clone)]
#[allow(dead_code)]
struct PeerInfo {
    pub_key: String,
    ip: String,
    name: String,
    role: String,
    public_endpoint: Option<String>,
}

#[derive(Serialize)]
struct MetricsReport {
    agent_vpn_ip: String,
    cpu_percent: f32,
    ram_used_mb: u64,
    ram_total_mb: u64,
    vram_used_mb: Option<u64>,
    vram_total_mb: Option<u64>,
}

fn query_vram() -> (Option<u64>, Option<u64>) {
    let out = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.used,memory.total", "--format=csv,noheader,nounits"])
        .output()
        .ok();
    let out = match out {
        Some(o) if o.status.success() => o,
        _ => return (None, None),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let line = match text.lines().next() {
        Some(l) => l,
        None => return (None, None),
    };
    let mut parts = line.split(',');
    let used: Option<u64> = parts.next().and_then(|s| s.trim().parse().ok());
    let total: Option<u64> = parts.next().and_then(|s| s.trim().parse().ok());
    (used, total)
}

#[derive(Deserialize)]
struct JoinResponse {
    master_pub_key: String,
    agent_vpn_ip: String,
    master_endpoint: String,
    peers: HashMap<String, PeerInfo>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Cmd::Connect { master, name } => connect(&master, name).await,
        Cmd::Service { action } => handle_service(action),
    }
}

fn handle_service(action: ServiceAction) -> Result<()> {
    let manager = r4a_service::ServiceManager::detect()?;
    match action {
        ServiceAction::Enable { master } => {
            let exec = format!("/usr/local/bin/r4a-agent connect --master {}", master);
            manager.enable("r4a-agent", "r4a Agent Node", &exec)?;
        }
        ServiceAction::Disable => {
            manager.disable("r4a-agent")?;
        }
    }
    Ok(())
}

async fn connect(master_api: &str, name: Option<String>) -> Result<()> {
    let name = name.unwrap_or_else(|| {
        System::host_name().unwrap_or_else(|| "agent".to_string())
    });

    let identity = load_identity().context("Failed to load or generate identity")?;

    info!("Joining master at {} as '{}'...", master_api, name);
    let client = reqwest::Client::new();
    let resp: JoinResponse = client
        .post(format!("{master_api}/api/join"))
        .json(&JoinRequest { 
            pub_key: identity.public_key.clone(), 
            name: name.clone(),
            role: "agent".to_string(),
        })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    info!("Assigned VPN IP : {}", resp.agent_vpn_ip);
    info!("Master endpoint : {}", resp.master_endpoint);

    info!("Setting up WireGuard interface...");
    r4a_vpn::wireguard::setup_agent(
        &identity.private_key,
        &resp.agent_vpn_ip,
        &resp.master_pub_key,
        &resp.master_endpoint,
    )?;

    // Выбираем всех мастеров из списка пиров
    let master_ips: Vec<String> = resp.peers
        .values()
        .filter(|p| p.role == "master")
        .map(|p| p.ip.clone())
        .collect();

    let mut hosts_ips: Vec<&str> = master_ips.iter().map(|s| s.as_str()).collect();
    if hosts_ips.is_empty() {
        hosts_ips.push("10.42.0.1"); // Фолбэк на случай если что-то не так
    }

    info!("Adding master.local ({}) to /etc/hosts...", hosts_ips.join(", "));
    r4a_vpn::dns::set_hosts_entries(&hosts_ips, "master.local")?;

    info!("Agent '{}' connected. VPN IP: {}", name, resp.agent_vpn_ip);

    let vpn_ip = resp.agent_vpn_ip.clone();
    tokio::spawn(async move {
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(3)).build().unwrap_or_default();
        let mut sys = System::new_all();
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            sys.refresh_all();
            let (vram_used_mb, vram_total_mb) = query_vram();
            let report = MetricsReport {
                agent_vpn_ip: vpn_ip.clone(),
                cpu_percent: sys.global_cpu_usage(),
                ram_used_mb: sys.used_memory() / 1024 / 1024,
                ram_total_mb: sys.total_memory() / 1024 / 1024,
                vram_used_mb,
                vram_total_mb,
            };
            
            // Если есть несколько мастеров, мы могли бы делать fallback.
            // Но пока что master.local отрезолвится в первый живой (в зависимости от поведения reqwest).
            let _ = client
                .post("http://master.local:8080/api/metrics")
                .json(&report)
                .send()
                .await;
        }
    });

    let master_base = "http://master.local:8080".to_string();
    let update_client = client.clone();
    let update_vpn_ip = resp.agent_vpn_ip.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
            if let Err(e) = check_and_apply_update(&update_client, &master_base, &update_vpn_ip).await {
                warn!("Update check failed: {e}");
            }
        }
    });

    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[derive(Deserialize)]
struct UpdatePollResponse {
    update_pending: bool,
    checksum: Option<String>,
}

fn sha256_self() -> Option<String> {
    let path = std::env::current_exe().ok()?;
    let data = std::fs::read(&path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    Some(format!("{:x}", hasher.finalize()))
}

async fn check_and_apply_update(client: &reqwest::Client, master: &str, vpn_ip: &str) -> Result<()> {
    let poll: UpdatePollResponse = client
        .get(format!("{master}/api/update/poll"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    if !poll.update_pending {
        return Ok(());
    }

    let master_checksum = match poll.checksum {
        Some(c) => c,
        None => return Ok(()),
    };

    let self_checksum = sha256_self().unwrap_or_default();
    if self_checksum == master_checksum {
        return Ok(());
    }

    info!("Update available (master={} self={}), downloading...", &master_checksum[..8], &self_checksum[..8]);

    let _ = report_update_status(client, master, vpn_ip, "updating", &self_checksum).await;

    let bytes = client
        .get(format!("{master}/api/agent-binary"))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let downloaded_checksum = format!("{:x}", hasher.finalize());
    if downloaded_checksum != master_checksum {
        let _ = report_update_status(client, master, vpn_ip, "failed", &downloaded_checksum).await;
        anyhow::bail!("checksum mismatch: expected {master_checksum} got {downloaded_checksum}");
    }

    let tmp_path = "/tmp/r4a-agent.new";
    let target_path = "/usr/local/bin/r4a-agent";
    std::fs::write(tmp_path, &bytes)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp_path, std::fs::Permissions::from_mode(0o755))?;
    }

    std::fs::rename(tmp_path, target_path)?;
    info!("Updated to checksum {}, restarting...", &master_checksum[..8]);

    let _ = report_update_status(client, master, vpn_ip, "updated", &master_checksum).await;

    std::process::exit(0);
}

async fn report_update_status(
    client: &reqwest::Client,
    master: &str,
    vpn_ip: &str,
    status: &str,
    checksum: &str,
) -> Result<()> {
    #[derive(Serialize)]
    struct Report<'a> { agent_vpn_ip: &'a str, checksum: &'a str, status: &'a str }
    client
        .post(format!("{master}/api/update/report"))
        .json(&Report { agent_vpn_ip: vpn_ip, checksum, status })
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}
