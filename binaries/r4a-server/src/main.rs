use anyhow::Result;
use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, StatusCode},
    response::Response,
    routing::{any, get, post},
    Json, Router,
};
use clap::{Parser, Subcommand};
use r4a_core::{
    AgentUpdateStatus, AgentUpdateState, Identity, JoinRequest, JoinResponse,
    Manifest, MetricsReport, NodeInfo, PeerInfo,
};
use r4a_store::Store;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use sysinfo::System;
use tracing::{info, warn, error};

const WG_PORT: u16 = 51820;
const API_PORT: u16 = 8080;
const INGRESS_PORT: u16 = 8000;

fn state_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".r4a-server")
}

fn save_identity(id: &Identity) -> Result<()> {
    let path = state_dir().join("identity.json");
    let tmp_path = path.with_extension("json.tmp");
    
    std::fs::create_dir_all(state_dir())?;
    
    let data = serde_json::to_string_pretty(id)?;
    std::fs::write(&tmp_path, data)?;
    
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600));
    }
    
    std::fs::rename(&tmp_path, &path)?;
    Ok(())
}

fn load_identity() -> Result<Identity> {
    let path = state_dir().join("identity.json");
    let env_secret = std::env::var("R4A_SECRET").ok();
    
    if path.exists() {
        let data = std::fs::read_to_string(&path)?;
        let mut id: Identity = serde_json::from_str(&data)?;
        
        if let Some(secret) = env_secret {
            if id.cluster_secret.as_ref() != Some(&secret) {
                id.cluster_secret = Some(secret);
                save_identity(&id)?;
            }
        } else if id.cluster_secret.is_none() {
            use rand::{RngCore, thread_rng};
            let mut secret = [0u8; 32];
            thread_rng().fill_bytes(&mut secret);
            id.cluster_secret = Some(hex::encode(secret));
            save_identity(&id)?;
        }
        
        info!("Loaded existing identity, public key: {}", id.public_key);
        return Ok(id);
    }
    
    let kp = r4a_vpn::wireguard::generate_keypair()?;
    
    let secret = if let Some(s) = env_secret {
        s
    } else {
        use rand::{RngCore, thread_rng};
        let mut secret = [0u8; 32];
        thread_rng().fill_bytes(&mut secret);
        hex::encode(secret)
    };
    
    let id = Identity {
        private_key: kp.private,
        public_key: kp.public,
        cluster_secret: Some(secret),
    };
    save_identity(&id)?;
    Ok(id)
}

#[derive(Clone)]
struct AppState {
    master_pub_key: String,
    cluster_secret: String,
    my_vpn_ip: String,
    peers: Arc<Mutex<HashMap<String, PeerInfo>>>,
    next_ip: Arc<Mutex<u8>>,
    update_pending: Arc<Mutex<bool>>,
    agent_update_states: Arc<Mutex<HashMap<String, AgentUpdateState>>>,
    store: Store,
}

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

struct RequireSecret;

#[async_trait::async_trait]
impl FromRequestParts<AppState> for RequireSecret {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        if let Some(auth_header) = parts.headers.get("X-R4A-Secret") {
            if let Ok(auth_str) = auth_header.to_str() {
                if auth_str == state.cluster_secret {
                    return Ok(RequireSecret);
                }
            }
        }
        Err((StatusCode::UNAUTHORIZED, "Invalid or missing X-R4A-Secret header"))
    }
}

#[derive(Parser)]
#[command(name = "r4a-server", about = "r4a Master Node")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Init,
    JoinMaster {
        #[arg(long)]
        master: String,
        #[arg(long)]
        name: Option<String>,
    },
    PruneNodes,
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Subcommand)]
enum ServiceAction {
    Enable,
    Disable,
}

const AGENT_BINARY_PATH: &str = "/usr/local/bin/r4a-agent";

fn load_peers(store: &Store) -> HashMap<String, PeerInfo> {
    if let Ok(Some(data)) = store.get("core", b"peers") {
        if let Ok(peers) = serde_json::from_slice(&data) {
            return peers;
        }
    }
    HashMap::new()
}

async fn save_peers(store: &Store, peers: &HashMap<String, PeerInfo>) {
    let json = match serde_json::to_vec(peers) {
        Ok(j) => j,
        Err(e) => {
            error!("Failed to serialize peers: {}", e);
            return;
        }
    };
    
    if let Err(e) = store.put("core", b"peers", &json).await {
        error!("Failed to save peers to store: {}", e);
    }

    let master_ips: Vec<String> = peers
        .values()
        .filter(|p| p.role == "master")
        .map(|p| p.ip.clone())
        .collect();
    store.set_masters(master_ips.clone());
    
    let ips_ref: Vec<&str> = master_ips.iter().map(|s| s.as_str()).collect();
    if !ips_ref.is_empty() {
        if let Err(e) = r4a_vpn::dns::set_hosts_entries(&ips_ref, "master.local") {
            warn!("Failed to update /etc/hosts: {}", e);
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    match cli.command {
        Cmd::Init => init("10.42.0.1", None).await,
        Cmd::JoinMaster { master, name } => join_master(&master, name).await,
        Cmd::PruneNodes => prune_nodes().await,
        Cmd::Service { action } => handle_service(action),
    }
}

async fn prune_nodes() -> Result<()> {
    let store = Store::open(state_dir().join("db"))?;
    store.delete("core", b"peers").await?;
    info!("Pruned all nodes from database.");
    Ok(())
}

fn handle_service(action: ServiceAction) -> Result<()> {
    let manager = r4a_service::ServiceManager::detect()?;
    match action {
        ServiceAction::Enable => {
            let exec = "/usr/local/bin/r4a-server init";
            manager.enable("r4a-server", "r4a Master Node", exec)?;
        }
        ServiceAction::Disable => {
            manager.disable("r4a-server")?;
        }
    }
    Ok(())
}

async fn join_master(first_master_url: &str, name: Option<String>) -> Result<()> {
    let identity = load_identity()?;
    let my_endpoint = format!("{}:{}", get_external_ip(), WG_PORT);
    let my_name = name.unwrap_or_else(|| {
        System::host_name().unwrap_or_else(|| "master-node".to_string())
    });

    info!("Joining existing master at {}...", first_master_url);
    let client = reqwest::Client::new();
    let req = JoinRequest {
        pub_key: identity.public_key.clone(),
        name: Some(my_name.clone()),
        role: Some("master".to_string()),
        public_endpoint: Some(my_endpoint.clone()),
    };

    let secret = identity.cluster_secret.clone().unwrap_or_default();

    let resp: JoinResponse = client
        .post(format!("{}/api/join", first_master_url))
        .header("X-R4A-Secret", &secret)
        .json(&req)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let my_vpn_ip = resp.agent_vpn_ip;
    info!("Assigned VPN IP as master: {}", my_vpn_ip);

    let store = Store::open(state_dir().join("db"))?;
    save_peers(&store, &resp.peers).await;

    let mut wg_peers = Vec::new();
    for peer in resp.peers.values() {
        if peer.pub_key != identity.public_key {
            wg_peers.push((peer.pub_key.as_str(), peer.ip.as_str()));
        }
    }
    
    r4a_vpn::wireguard::setup_master_with_peers(
        &identity.private_key,
        &my_vpn_ip,
        WG_PORT,
        &wg_peers,
    )?;
    
    let master_host = first_master_url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split(':')
        .next()
        .unwrap_or("");
        
    let initial_endpoint = format!("{}:{}", master_host, WG_PORT);
    
    let _ = std::process::Command::new("wg")
        .args(["set", "wg0", "peer", &resp.master_pub_key, "endpoint", &initial_endpoint])
        .status();

    start_server(identity, my_vpn_ip, store).await
}

async fn init(my_vpn_ip: &str, _store: Option<Store>) -> Result<()> {
    let identity = load_identity()?;
    let store = Store::open(state_dir().join("db"))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut peers = load_peers(&store);
    let my_endpoint = format!("{}:{}", get_external_ip(), WG_PORT);
    let my_name = System::host_name().unwrap_or_else(|| "master".to_string());

    if !peers.contains_key(&identity.public_key) {
        peers.insert(
            identity.public_key.clone(),
            PeerInfo {
                pub_key: identity.public_key.clone(),
                ip: my_vpn_ip.to_string(),
                name: my_name.clone(),
                role: "master".to_string(),
                public_endpoint: Some(my_endpoint.clone()),
                cpu_percent: None,
                ram_used_mb: None,
                ram_total_mb: None,
                vram_used_mb: None,
                vram_total_mb: None,
                last_seen: Some(now),
            },
        );
        save_peers(&store, &peers).await;
    }

    let saved_peers = load_peers(&store);
    save_peers(&store, &saved_peers).await;

    let mut wg_peers = Vec::new();
    for peer in saved_peers.values() {
        if peer.pub_key != identity.public_key {
            wg_peers.push((peer.pub_key.as_str(), peer.ip.as_str()));
        }
    }

    r4a_vpn::wireguard::setup_master_with_peers(
        &identity.private_key,
        my_vpn_ip,
        WG_PORT,
        &wg_peers,
    )?;

    for peer in saved_peers.values() {
        if peer.role == "master" && peer.pub_key != identity.public_key {
            if let Some(endpoint) = &peer.public_endpoint {
                let _ = std::process::Command::new("wg")
                    .args(["set", "wg0", "peer", &peer.pub_key, "endpoint", endpoint])
                    .status();
            }
        }
    }

    start_server(identity, my_vpn_ip.to_string(), store).await
}

async fn start_server(identity: Identity, my_vpn_ip: String, store: Store) -> Result<()> {
    let cluster_secret = identity.cluster_secret.clone().unwrap_or_default();
    let git_root = r4a_git_registry::default_git_root();
    let manifests_repo = git_root.join("manifests.git");
    r4a_git_registry::init_repo(&manifests_repo)?;

    let saved_peers = load_peers(&store);
    let next_ip = saved_peers
        .values()
        .filter_map(|p| p.ip.split('.').last()?.parse::<u8>().ok())
        .max()
        .map(|m| m + 1)
        .unwrap_or(2);

    let state = AppState {
        master_pub_key: identity.public_key.clone(),
        cluster_secret: cluster_secret.clone(),
        my_vpn_ip: my_vpn_ip.clone(),
        peers: Arc::new(Mutex::new(saved_peers)),
        next_ip: Arc::new(Mutex::new(next_ip)),
        update_pending: Arc::new(Mutex::new(false)),
        agent_update_states: Arc::new(Mutex::new(HashMap::new())),
        store: store.clone(),
    };
    
    store.set_secret(cluster_secret);

    let broadcast_state = state.clone();
    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap_or_default();
        let mut sys = System::new_all();
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            sys.refresh_all();
            let (vram_used_mb, vram_total_mb) = query_vram();
            
            let report = MetricsReport {
                agent_vpn_ip: broadcast_state.my_vpn_ip.clone(),
                cpu_percent: sys.global_cpu_usage(),
                ram_used_mb: sys.used_memory() / 1024 / 1024,
                ram_total_mb: sys.total_memory() / 1024 / 1024,
                vram_used_mb,
                vram_total_mb,
            };

            let masters: Vec<String> = {
                let peers = broadcast_state.peers.lock().unwrap();
                peers.values()
                    .filter(|p| p.role == "master" && p.ip != broadcast_state.my_vpn_ip)
                    .map(|p| p.ip.clone())
                    .collect()
            };

            for master_ip in masters {
                let _ = client
                    .post(format!("http://{master_ip}:8080/api/metrics"))
                    .header("X-R4A-Secret", &broadcast_state.cluster_secret)
                    .json(&report)
                    .send()
                    .await;
            }
        }
    });

    let hosts_state = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            let peers = hosts_state.peers.lock().unwrap().clone();
            let master_ips: Vec<String> = peers
                .values()
                .filter(|p| p.role == "master")
                .map(|p| p.ip.clone())
                .collect();
            
            let ips_ref: Vec<&str> = master_ips.iter().map(|s| s.as_str()).collect();
            if !ips_ref.is_empty() {
                let _ = r4a_vpn::dns::set_hosts_entries(&ips_ref, "master.local");
            }

            hosts_state.store.set_masters(master_ips);
        }
    });

    let manifest_state = state.clone();
    tokio::spawn(async move {
        let repo_path = r4a_git_registry::default_git_root().join("manifests.git");
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            match r4a_git_registry::list_files(&repo_path, "main", ".toml") {
                Ok(files) => {
                    let mut current_manifests = HashMap::new();
                    for file in files {
                        if let Ok(content) = r4a_git_registry::read_file(&repo_path, "main", &file) {
                            if let Ok(manifest) = toml::from_str::<Manifest>(&content) {
                                current_manifests.insert(manifest.app.name.clone(), manifest);
                            }
                        }
                    }
                    
                    if !current_manifests.is_empty() {
                        let json = serde_json::to_vec(&current_manifests).unwrap_or_default();
                        let _ = manifest_state.store.put("core", b"manifests", &json).await;
                    }
                }
                Err(_) => {}
            }
        }
    });

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/api/join", post(join_handler))
        .route("/api/nodes", get(nodes_handler))
        .route("/api/metrics", post(metrics_handler))
        .route("/api/manifests", get(manifests_handler))
        .route("/api/git/repos", get(git_repos_handler).post(git_create_repo_handler))
        .route("/api/agent-binary", get(agent_binary_handler))
        .route("/api/agent-checksum", get(agent_checksum_handler))
        .route("/api/update/test", post(update_test_handler))
        .route("/api/update/trigger", post(update_trigger_handler))
        .route("/api/update/poll", get(update_poll_handler))
        .route("/api/update/report", post(update_report_handler))
        .route("/api/update/status", get(update_status_handler))
        .nest_service(
            "/git",
            Router::new()
                .route("/*path", any(r4a_git_registry::handler::git_handler))
                .with_state(git_root),
        )
        .with_state(state)
        .merge(r4a_store::store_router(store.clone()));

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", API_PORT)).await?;
    info!("API listening on 0.0.0.0:{}", API_PORT);

    let pingora_store = store.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));

        let mut my_server = pingora::server::Server::new(None).expect("Failed to create Pingora server");
        my_server.bootstrap();
        
        let mut proxy = pingora::proxy::http_proxy_service(
            &my_server.configuration,
            r4a_ingress::IngressProxy { store: pingora_store }
        );
        
        let addr = format!("0.0.0.0:{}", INGRESS_PORT);
        proxy.add_tcp(&addr);
        
        my_server.add_service(proxy);
        my_server.run_forever();
    });

    axum::serve(listener, app).await?;
    Ok(())
}

async fn index_handler() -> (StatusCode, &'static str) {
    (StatusCode::OK, "r4a master node OK\n")
}

async fn join_handler(
    State(state): State<AppState>,
    _auth: RequireSecret,
    Json(req): Json<JoinRequest>,
) -> Json<JoinResponse> {
    let mut peers = state.peers.lock().unwrap();

    let agent_ip = if let Some(existing) = peers.get(&req.pub_key) {
        existing.ip.clone()
    } else {
        let mut next = state.next_ip.lock().unwrap();
        let ip = format!("10.42.0.{}", *next);
        *next += 1;
        ip
    };

    let role = req.role.unwrap_or_else(|| "agent".to_string());
    let name = req.name.unwrap_or_else(|| format!("{}-{}", role, &agent_ip[agent_ip.rfind('.').unwrap_or(0)+1..]));

    let mut wg_cmd = std::process::Command::new("wg");
    wg_cmd.args(["set", "wg0", "peer", &req.pub_key, "allowed-ips", &format!("{agent_ip}/32"), "persistent-keepalive", "25"]);
    let _ = wg_cmd.status();

    peers.insert(
        req.pub_key.clone(),
        PeerInfo {
            pub_key: req.pub_key.clone(),
            ip: agent_ip.clone(),
            name,
            role,
            public_endpoint: req.public_endpoint.clone(),
            cpu_percent: None,
            ram_used_mb: None,
            ram_total_mb: None,
            vram_used_mb: None,
            vram_total_mb: None,
            last_seen: Some(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()),
        },
    );

    let cloned_peers = peers.clone();
    let cloned_store = state.store.clone();
    tokio::spawn(async move {
        save_peers(&cloned_store, &cloned_peers).await;
    });

    Json(JoinResponse {
        master_pub_key: state.master_pub_key.clone(),
        agent_vpn_ip: agent_ip,
        master_endpoint: format!("{}:{}", get_external_ip(), WG_PORT),
        peers: peers.clone(),
    })
}

async fn metrics_handler(
    State(state): State<AppState>,
    _auth: RequireSecret,
    Json(report): Json<MetricsReport>,
) -> StatusCode {
    let mut peers = state.peers.lock().unwrap();
    if let Some(peer) = peers.values_mut().find(|p| p.ip == report.agent_vpn_ip) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        peer.cpu_percent = Some(report.cpu_percent);
        peer.ram_used_mb = Some(report.ram_used_mb);
        peer.ram_total_mb = Some(report.ram_total_mb);
        peer.vram_used_mb = report.vram_used_mb;
        peer.vram_total_mb = report.vram_total_mb;
        peer.last_seen = Some(now);
    }
    StatusCode::OK
}

async fn nodes_handler(State(state): State<AppState>, _auth: RequireSecret) -> Json<Vec<NodeInfo>> {
    let mut sys = System::new_all();
    sys.refresh_all();
    let master_name = System::host_name().unwrap_or_else(|| "master".to_string());
    let (master_vram_used, master_vram_total) = query_vram();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut nodes = vec![NodeInfo {
        ip: state.my_vpn_ip.clone(),
        name: master_name,
        role: "master".to_string(),
        cpu_percent: Some(sys.global_cpu_usage()),
        ram_used_mb: Some(sys.used_memory() / 1024 / 1024),
        ram_total_mb: Some(sys.total_memory() / 1024 / 1024),
        vram_used_mb: master_vram_used,
        vram_total_mb: master_vram_total,
        last_seen: Some(now),
    }];

    for peer in state.peers.lock().unwrap().values() {
        if peer.ip == state.my_vpn_ip { continue; } 
        if let Some(ls) = peer.last_seen {
            if now - ls > 600 { continue; }
        } else { continue; }

        nodes.push(NodeInfo {
            ip: peer.ip.clone(),
            name: peer.name.clone(),
            role: peer.role.clone(),
            cpu_percent: peer.cpu_percent,
            ram_used_mb: peer.ram_used_mb,
            ram_total_mb: peer.ram_total_mb,
            vram_used_mb: peer.vram_used_mb,
            vram_total_mb: peer.vram_total_mb,
            last_seen: peer.last_seen,
        });
    }

    Json(nodes)
}

#[derive(Deserialize)]
struct ManifestsQuery {
    node: Option<String>,
}

async fn manifests_handler(
    State(state): State<AppState>,
    _auth: RequireSecret,
    Query(query): Query<ManifestsQuery>,
) -> Json<HashMap<String, Manifest>> {
    if let Ok(Some(data)) = state.store.get("core", b"manifests") {
        if let Ok(manifests) = serde_json::from_slice::<HashMap<String, Manifest>>(&data) {
            if let Some(node_name) = query.node {
                let filtered = manifests
                    .into_iter()
                    .filter(|(_, m)| m.app.node_selector == node_name || m.app.node_selector == "all")
                    .collect();
                return Json(filtered);
            }
            return Json(manifests);
        }
    }
    Json(HashMap::new())
}

fn query_vram() -> (Option<u64>, Option<u64>) {
    let out = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.used,memory.total", "--format=csv,noheader,nounits"])
        .output()
        .ok();
    if let Some(o) = out {
        let text = String::from_utf8_lossy(&o.stdout);
        if let Some(line) = text.lines().next() {
            let mut parts = line.split(',');
            let used = parts.next().and_then(|s| s.trim().parse().ok());
            let total = parts.next().and_then(|s| s.trim().parse().ok());
            return (used, total);
        }
    }
    (None, None)
}

#[derive(Serialize)]
struct RepoInfo {
    name: String,
    clone_url: String,
}

async fn git_repos_handler(State(state): State<AppState>, _auth: RequireSecret) -> Json<Vec<RepoInfo>> {
    let git_root = r4a_git_registry::default_git_root();
    let mut repos = vec![];
    if let Ok(entries) = std::fs::read_dir(&git_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("HEAD").exists() {
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                let clone_url = format!("http://{}:{}/git/{}", state.my_vpn_ip, API_PORT, name);
                repos.push(RepoInfo { name, clone_url });
            }
        }
    }
    repos.sort_by(|a, b| a.name.cmp(&b.name));
    Json(repos)
}

#[derive(Deserialize)]
struct CreateRepoRequest {
    name: String,
}

async fn git_create_repo_handler(
    State(state): State<AppState>,
    _auth: RequireSecret,
    Json(req): Json<CreateRepoRequest>,
) -> Result<Json<RepoInfo>, (StatusCode, String)> {
    let name = req.name.trim().to_string();
    if name.is_empty() || name.contains('/') || name.contains("..") {
        return Err((StatusCode::BAD_REQUEST, "invalid repo name".to_string()));
    }
    let repo_name = if name.ends_with(".git") { name.clone() } else { format!("{}.git", name) };
    let path = r4a_git_registry::default_git_root().join(&repo_name);
    if path.exists() {
        return Err((StatusCode::CONFLICT, format!("repository '{}' already exists", repo_name)));
    }
    r4a_git_registry::init_repo(&path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let clone_url = format!("http://{}:{}/git/{}", state.my_vpn_ip, API_PORT, repo_name);
    Ok(Json(RepoInfo { name: repo_name, clone_url }))
}

fn sha256_file(path: &str) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    Some(format!("{:x}", hasher.finalize()))
}

async fn agent_binary_handler(_auth: RequireSecret) -> Response {
    match tokio::fs::read(AGENT_BINARY_PATH).await {
        Ok(data) => Response::builder()
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(data))
            .unwrap(),
        Err(e) => Response::builder().status(StatusCode::NOT_FOUND).body(Body::from(e.to_string())).unwrap(),
    }
}

async fn agent_checksum_handler(_auth: RequireSecret) -> Result<Json<ChecksumResponse>, (StatusCode, String)> {
    match sha256_file(AGENT_BINARY_PATH) {
        Some(checksum) => Ok(Json(ChecksumResponse { checksum })),
        None => Err((StatusCode::NOT_FOUND, "binary not found".to_string())),
    }
}

#[derive(Serialize)]
struct ChecksumResponse { checksum: String }

#[derive(Serialize)]
struct TestResponse { ok: bool, checksum: Option<String>, message: String }

async fn update_test_handler(_auth: RequireSecret) -> Json<TestResponse> {
    match sha256_file(AGENT_BINARY_PATH) {
        Some(checksum) => Json(TestResponse { ok: true, checksum: Some(checksum), message: "binary OK".to_string() }),
        None => Json(TestResponse { ok: false, checksum: None, message: "not found".to_string() }),
    }
}

async fn update_trigger_handler(State(state): State<AppState>, _auth: RequireSecret) -> StatusCode {
    *state.update_pending.lock().unwrap() = true;
    StatusCode::OK
}

#[derive(Serialize)]
struct UpdatePollResponse { update_pending: bool, checksum: Option<String> }

async fn update_poll_handler(State(state): State<AppState>, _auth: RequireSecret) -> Json<UpdatePollResponse> {
    let update_pending = *state.update_pending.lock().unwrap();
    let checksum = if update_pending { sha256_file(AGENT_BINARY_PATH) } else { None };
    Json(UpdatePollResponse { update_pending, checksum })
}

#[derive(Deserialize)]
struct UpdateReport { agent_vpn_ip: String, checksum: String, status: String }

async fn update_report_handler(State(state): State<AppState>, _auth: RequireSecret, Json(report): Json<UpdateReport>) -> StatusCode {
    let update_status = match report.status.as_str() {
        "updated" => AgentUpdateStatus::Updated,
        "updating" => AgentUpdateStatus::Updating,
        "failed" => AgentUpdateStatus::Failed("failed".to_string()),
        _ => AgentUpdateStatus::Unknown,
    };
    state.agent_update_states.lock().unwrap().insert(
        report.agent_vpn_ip,
        AgentUpdateState { status: update_status, checksum: Some(report.checksum) },
    );
    StatusCode::OK
}

#[derive(Serialize)]
struct UpdateStatusResponse { master_checksum: Option<String>, update_pending: bool, agents: HashMap<String, AgentUpdateStateDto> }

#[derive(Serialize)]
struct AgentUpdateStateDto { status: String, checksum: Option<String> }

async fn update_status_handler(State(state): State<AppState>, _auth: RequireSecret) -> Json<UpdateStatusResponse> {
    let master_checksum = sha256_file(AGENT_BINARY_PATH);
    let update_pending = *state.update_pending.lock().unwrap();
    let states = state.agent_update_states.lock().unwrap();
    let agents = states.iter().map(|(ip, s)| {
        let status_str = match &s.status {
            AgentUpdateStatus::Updated => "updated",
            AgentUpdateStatus::Updating => "updating",
            AgentUpdateStatus::Failed(_) => "failed",
            _ => "unknown",
        }.to_string();
        (ip.clone(), AgentUpdateStateDto { status: status_str, checksum: s.checksum.clone() })
    }).collect();
    Json(UpdateStatusResponse { master_checksum, update_pending, agents })
}

fn get_external_ip() -> String {
    let out = std::process::Command::new("ip").args(["-4", "addr", "show"]).output();
    let mut fallback = "127.0.0.1".to_string();
    
    if let Ok(o) = out {
        let text = String::from_utf8_lossy(&o.stdout);
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with("inet ") && !line.contains("127.") && !line.contains("10.42.") {
                if let Some(ip_with_mask) = line.split_whitespace().nth(1) {
                    if let Some(ip) = ip_with_mask.split('/').next() {
                        if ip.starts_with("100.") {
                            return ip.to_string();
                        }
                        fallback = ip.to_string();
                    }
                }
            }
        }
    }
    fallback
}
