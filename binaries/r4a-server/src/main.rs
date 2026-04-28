use anyhow::Result;
use axum::{Json, Router, extract::State, http::StatusCode, routing::{any, get, post}};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use sysinfo::System;
use tracing::info;

const MASTER_VPN_IP: &str = "10.42.0.1";
const WG_PORT: u16 = 51820;
const API_PORT: u16 = 8080;

fn state_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".r4a-server")
}

#[derive(Serialize, Deserialize)]
struct Identity {
    private_key: String,
    public_key: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct PersistedPeer {
    pub_key: String,
    ip: String,
    name: String,
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
    let id = Identity { private_key: kp.private, public_key: kp.public };
    std::fs::create_dir_all(state_dir())?;
    std::fs::write(&path, serde_json::to_string_pretty(&id)?)?;
    info!("Saved identity to {}", path.display());
    Ok(id)
}

fn load_peers() -> Vec<PersistedPeer> {
    let path = state_dir().join("peers.json");
    if !path.exists() {
        return vec![];
    }
    match std::fs::read_to_string(&path).ok().and_then(|d| serde_json::from_str(&d).ok()) {
        Some(peers) => peers,
        None => vec![],
    }
}

fn save_peers(peers: &HashMap<String, PeerInfo>) {
    let path = state_dir().join("peers.json");
    let list: Vec<PersistedPeer> = peers.values().map(|p| PersistedPeer {
        pub_key: p.pub_key.clone(),
        ip: p.ip.clone(),
        name: p.name.clone(),
    }).collect();
    let _ = std::fs::write(&path, serde_json::to_string_pretty(&list).unwrap_or_default());
}

#[derive(Parser)]
#[command(name = "r4a-server", about = "r4a Master Node")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Инициализировать master-ноду: поднять WireGuard и встроенный HTTP ingress
    Init,
}

#[derive(Clone)]
struct PeerInfo {
    pub_key: String,
    ip: String,
    name: String,
    cpu_percent: Option<f32>,
    ram_used_mb: Option<u64>,
    ram_total_mb: Option<u64>,
    vram_used_mb: Option<u64>,
    vram_total_mb: Option<u64>,
}

#[derive(Clone)]
struct AppState {
    master_pub_key: String,
    peers: Arc<Mutex<HashMap<String, PeerInfo>>>,
    next_ip: Arc<Mutex<u8>>,
}

#[derive(Serialize)]
struct NodeInfo {
    ip: String,
    name: String,
    role: String,
    cpu_percent: Option<f32>,
    ram_used_mb: Option<u64>,
    ram_total_mb: Option<u64>,
    vram_used_mb: Option<u64>,
    vram_total_mb: Option<u64>,
}

#[derive(Deserialize)]
struct JoinRequest {
    pub_key: String,
    name: Option<String>,
}

#[derive(Serialize)]
struct JoinResponse {
    master_pub_key: String,
    agent_vpn_ip: String,
    master_endpoint: String,
}

#[derive(Deserialize)]
struct MetricsReport {
    agent_vpn_ip: String,
    cpu_percent: f32,
    ram_used_mb: u64,
    ram_total_mb: u64,
    vram_used_mb: Option<u64>,
    vram_total_mb: Option<u64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    match cli.command {
        Cmd::Init => init().await,
    }
}

async fn init() -> Result<()> {
    // Загружаем или создаём keypair (персистентный)
    let identity = load_identity()?;

    // Загружаем сохранённых пиров
    let saved_peers = load_peers();
    if !saved_peers.is_empty() {
        info!("Restoring {} peer(s) from disk", saved_peers.len());
    }

    // Поднимаем WireGuard с уже известными пирами
    info!("Setting up WireGuard ({}:{})...", MASTER_VPN_IP, WG_PORT);
    r4a_vpn::wireguard::setup_master_with_peers(
        &identity.private_key,
        MASTER_VPN_IP,
        WG_PORT,
        &saved_peers.iter().map(|p| (p.pub_key.as_str(), p.ip.as_str())).collect::<Vec<_>>(),
    )?;

    // Инициализация git-хранилища манифестов
    let git_root = r4a_git_registry::default_git_root();
    let manifests_repo = git_root.join("manifests.git");
    r4a_git_registry::init_repo(&manifests_repo)?;

    let master_endpoint = get_external_ip();
    info!("Master external IP: {}", master_endpoint);

    // Восстанавливаем состояние из сохранённых пиров
    let peers_map: HashMap<String, PeerInfo> = saved_peers.iter().map(|p| {
        (p.pub_key.clone(), PeerInfo {
            pub_key: p.pub_key.clone(),
            ip: p.ip.clone(),
            name: p.name.clone(),
            cpu_percent: None,
            ram_used_mb: None,
            ram_total_mb: None,
            vram_used_mb: None,
            vram_total_mb: None,
        })
    }).collect();

    // next_ip = max(существующие IP) + 1
    let next_ip = saved_peers.iter()
        .filter_map(|p| p.ip.split('.').last()?.parse::<u8>().ok())
        .max()
        .map(|m| m + 1)
        .unwrap_or(2);

    let state = AppState {
        master_pub_key: identity.public_key.clone(),
        peers: Arc::new(Mutex::new(peers_map)),
        next_ip: Arc::new(Mutex::new(next_ip)),
    };

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/api/join", post(join_handler))
        .route("/api/nodes", get(nodes_handler))
        .route("/api/metrics", post(metrics_handler))
        .nest_service(
            "/git",
            Router::new()
                .route("/*path", any(r4a_git_registry::handler::git_handler))
                .with_state(git_root),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(
        format!("0.0.0.0:{}", API_PORT)
    ).await?;
    info!("API listening on 0.0.0.0:{}", API_PORT);
    info!("Public key: {}", identity.public_key);
    info!("Waiting for agents...");

    axum::serve(listener, app).await?;

    Ok(())
}

async fn index_handler() -> (StatusCode, &'static str) {
    (StatusCode::OK, "r4a master node OK\n")
}

async fn join_handler(
    State(state): State<AppState>,
    Json(req): Json<JoinRequest>,
) -> Json<JoinResponse> {
    let mut peers = state.peers.lock().unwrap();

    // Если агент уже зарегистрирован (повторный join после рестарта агента)
    if let Some(existing) = peers.get(&req.pub_key) {
        let agent_ip = existing.ip.clone();
        info!("Agent re-joined: name={} ip={}", existing.name, agent_ip);
        return Json(JoinResponse {
            master_pub_key: state.master_pub_key.clone(),
            agent_vpn_ip: agent_ip,
            master_endpoint: format!("{}:{}", get_external_ip(), WG_PORT),
        });
    }

    let agent_ip = {
        let mut next = state.next_ip.lock().unwrap();
        let ip = format!("10.42.0.{}", *next);
        *next += 1;
        ip
    };

    let _ = std::process::Command::new("wg")
        .args(["set", "wg0", "peer", &req.pub_key, "allowed-ips", &format!("{agent_ip}/32")])
        .status();

    let name = req.name.unwrap_or_else(|| format!("agent-{}", &agent_ip[agent_ip.rfind('.').unwrap_or(0)+1..]));
    info!("Agent joined: name={} pub_key={}... ip={}", name, &req.pub_key[..8], agent_ip);

    peers.insert(req.pub_key.clone(), PeerInfo {
        pub_key: req.pub_key.clone(),
        ip: agent_ip.clone(),
        name,
        cpu_percent: None,
        ram_used_mb: None,
        ram_total_mb: None,
        vram_used_mb: None,
        vram_total_mb: None,
    });

    save_peers(&peers);

    Json(JoinResponse {
        master_pub_key: state.master_pub_key.clone(),
        agent_vpn_ip: agent_ip,
        master_endpoint: format!("{}:{}", get_external_ip(), WG_PORT),
    })
}

async fn metrics_handler(
    State(state): State<AppState>,
    Json(report): Json<MetricsReport>,
) -> StatusCode {
    let mut peers = state.peers.lock().unwrap();
    if let Some(peer) = peers.values_mut().find(|p| p.ip == report.agent_vpn_ip) {
        peer.cpu_percent = Some(report.cpu_percent);
        peer.ram_used_mb = Some(report.ram_used_mb);
        peer.ram_total_mb = Some(report.ram_total_mb);
        peer.vram_used_mb = report.vram_used_mb;
        peer.vram_total_mb = report.vram_total_mb;
    }
    StatusCode::OK
}

async fn nodes_handler(State(state): State<AppState>) -> Json<Vec<NodeInfo>> {
    let mut sys = System::new_all();
    sys.refresh_all();

    let master_name = System::host_name().unwrap_or_else(|| "master".to_string());
    let (master_vram_used, master_vram_total) = query_vram();

    let mut nodes = vec![NodeInfo {
        ip: MASTER_VPN_IP.to_string(),
        name: master_name,
        role: "master".to_string(),
        cpu_percent: Some(sys.global_cpu_usage()),
        ram_used_mb: Some(sys.used_memory() / 1024 / 1024),
        ram_total_mb: Some(sys.total_memory() / 1024 / 1024),
        vram_used_mb: master_vram_used,
        vram_total_mb: master_vram_total,
    }];

    for peer in state.peers.lock().unwrap().values() {
        nodes.push(NodeInfo {
            ip: peer.ip.clone(),
            name: peer.name.clone(),
            role: "agent".to_string(),
            cpu_percent: peer.cpu_percent,
            ram_used_mb: peer.ram_used_mb,
            ram_total_mb: peer.ram_total_mb,
            vram_used_mb: peer.vram_used_mb,
            vram_total_mb: peer.vram_total_mb,
        });
    }

    Json(nodes)
}

fn query_vram() -> (Option<u64>, Option<u64>) {
    let inner = || -> Option<(u64, u64)> {
        let out = std::process::Command::new("nvidia-smi")
            .args(["--query-gpu=memory.used,memory.total", "--format=csv,noheader,nounits"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let line = text.lines().next()?.to_string();
        let mut parts = line.split(',');
        let used: u64 = parts.next()?.trim().parse().ok()?;
        let total: u64 = parts.next()?.trim().parse().ok()?;
        Some((used, total))
    };
    match inner() {
        Some((u, t)) => (Some(u), Some(t)),
        None => (None, None),
    }
}

fn get_external_ip() -> String {
    let out = std::process::Command::new("ip")
        .args(["-4", "addr", "show"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("inet ") && !line.contains("127.") && !line.contains("10.42.") {
            if let Some(ip) = line.split_whitespace().nth(1) {
                if let Some(ip) = ip.split('/').next() {
                    if ip.starts_with("192.168.") {
                        return ip.to_string();
                    }
                }
            }
        }
    }
    "127.0.0.1".to_string()
}
