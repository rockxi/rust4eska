use anyhow::Result;
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use sysinfo::System;
use tracing::info;

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
}

#[derive(Serialize)]
struct JoinRequest {
    pub_key: String,
    name: String,
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
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Cmd::Connect { master, name } => connect(&master, name).await,
    }
}

async fn connect(master_api: &str, name: Option<String>) -> Result<()> {
    let name = name.unwrap_or_else(|| {
        System::host_name().unwrap_or_else(|| "agent".to_string())
    });

    info!("Generating WireGuard keypair...");
    let kp = r4a_vpn::wireguard::generate_keypair()?;

    info!("Joining master at {} as '{}'...", master_api, name);
    let client = reqwest::Client::new();
    let resp: JoinResponse = client
        .post(format!("{master_api}/api/join"))
        .json(&JoinRequest { pub_key: kp.public.clone(), name: name.clone() })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    info!("Assigned VPN IP : {}", resp.agent_vpn_ip);
    info!("Master endpoint : {}", resp.master_endpoint);

    info!("Setting up WireGuard interface...");
    r4a_vpn::wireguard::setup_agent(
        &kp.private,
        &resp.agent_vpn_ip,
        &resp.master_pub_key,
        &resp.master_endpoint,
    )?;

    info!("Adding master.local to /etc/hosts...");
    r4a_vpn::dns::set_hosts_entry("10.42.0.1", "master.local")?;

    info!("Agent '{}' connected. VPN IP: {}", name, resp.agent_vpn_ip);

    let vpn_ip = resp.agent_vpn_ip.clone();
    tokio::spawn(async move {
        let client = reqwest::Client::new();
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
            let _ = client
                .post("http://10.42.0.1:8080/api/metrics")
                .json(&report)
                .send()
                .await;
        }
    });

    tokio::signal::ctrl_c().await?;
    Ok(())
}
