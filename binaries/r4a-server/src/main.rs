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
use tracing::{info, warn, error, debug};

const WG_PORT: u16 = 51820;
const API_PORT: u16 = 8080;
const INGRESS_PORT: u16 = 8000;

fn state_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".r4a-server")
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
#[command(name = "r4a-server", about = "r4a Master Node")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Инициализировать первую master-ноду
    Init,
    /// Присоединить эту ноду как дополнительный master
    JoinMaster {
        #[arg(long)]
        master: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// Очистить список всех нод
    PruneNodes,
    /// Управление системным сервисом (systemd/launchd)
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Subcommand)]
enum ServiceAction {
    /// Установить и запустить сервис
    Enable,
    /// Остановить и удалить сервис
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
    let json = serde_json::to_vec(peers).unwrap_or_default();
    let _ = store.put("core", b"peers", &json).await;

    let master_ips: Vec<String> = peers
        .values()
        .filter(|p| p.role == "master")
        .map(|p| p.ip.clone())
        .collect();
    store.set_masters(master_ips.clone());
    
    let ips_ref: Vec<&str> = master_ips.iter().map(|s| s.as_str()).collect();
    if !ips_ref.is_empty() {
        let _ = r4a_vpn::dns::set_hosts_entries(&ips_ref, "master.local");
    }
}

#[derive(Clone)]
struct AppState {
    master_pub_key: String,
    my_vpn_ip: String,
    peers: Arc<Mutex<HashMap<String, PeerInfo>>>,
    next_ip: Arc<Mutex<u8>>,
    update_pending: Arc<Mutex<bool>>,
    agent_update_states: Arc<Mutex<HashMap<String, AgentUpdateState>>>,
    store: Store,
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
    let _ = store.delete("core", b"peers").await;
    info!("Pruned all nodes from database. Master will need to be re-initialized.");
    Ok(())
}

fn handle_service(action: ServiceAction) -> Result<()> {
    let manager = r4a_service::ServiceManager::detect()?;
    match action {
        ServiceAction::Enable => {
            let exec = format!("/usr/local/bin/r4a-server init");
            manager.enable("r4a-server", "r4a Master Node", &exec)?;
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

    let resp: JoinResponse = client
        .post(format!("{}/api/join", first_master_url))
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
    
    info!("Setting up WireGuard on {} with {} peers...", my_vpn_ip, wg_peers.len());
    r4a_vpn::wireguard::setup_master_with_peers(
        &identity.private_key,
        &my_vpn_ip,
        WG_PORT,
        &wg_peers,
    )?;
    
    // Parse the host from first_master_url
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
    info!("Loaded {} peers from store", saved_peers.len());
    save_peers(&store, &saved_peers).await;

    let mut wg_peers = Vec::new();
    for peer in saved_peers.values() {
        if peer.pub_key != identity.public_key {
            wg_peers.push((peer.pub_key.as_str(), peer.ip.as_str()));
        }
    }

    info!("Setting up WireGuard on {} with {} peers...", my_vpn_ip, wg_peers.len());
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
        my_vpn_ip: my_vpn_ip.clone(),
        peers: Arc::new(Mutex::new(saved_peers)),
        next_ip: Arc::new(Mutex::new(next_ip)),
        update_pending: Arc::new(Mutex::new(false)),
        agent_update_states: Arc::new(Mutex::new(HashMap::new())),
        store: store.clone(),
    };

    let broadcast_state = state.clone();
    tokio::spawn(async move {
        use std::io::Write;
        let mut log_file = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/metrics_debug.log").ok();
        
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

            if let Some(f) = &mut log_file {
                let _ = writeln!(f, "Broadcasting metrics to masters: {:?}", masters);
            }

            for master_ip in masters {
                let res = client
                    .post(format!("http://{master_ip}:8080/api/metrics"))
                    .json(&report)
                    .send()
                    .await;
                if let Some(f) = &mut log_file {
                    let _ = writeln!(f, "Sent to {}: {:?}", master_ip, res.is_ok());
                }
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
        info!("Starting manifest parsing loop for {}", repo_path.display());
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            match r4a_git_registry::list_files(&repo_path, "main", ".toml") {
                Ok(files) => {
                    let mut current_manifests = HashMap::new();
                    for file in files {
                        match r4a_git_registry::read_file(&repo_path, "main", &file) {
                            Ok(content) => {
                                match toml::from_str::<Manifest>(&content) {
                                    Ok(manifest) => {
                                        info!("Parsed manifest: {} for node: {}", manifest.app.name, manifest.app.node_selector);
                                        current_manifests.insert(manifest.app.name.clone(), manifest);
                                    }
                                    Err(e) => warn!("Failed to parse manifest file {}: {}", file, e),
                                }
                            }
                            Err(e) => warn!("Failed to read manifest file {}: {}", file, e),
                        }
                    }
                    
                    if !current_manifests.is_empty() {
                        let json = serde_json::to_vec(&current_manifests).unwrap_or_default();
                        if let Err(e) = manifest_state.store.put("core", b"manifests", &json).await {
                            error!("Failed to save manifests to store: {}", e);
                        }
                    }
                }
                Err(e) => {
                    debug!("Could not list manifest files: {}", e);
                }
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

    info!("Public key: {}", identity.public_key);
    info!("My VPN IP: {}", my_vpn_ip);
    info!("Waiting for agents and other masters...");

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
        info!("Pingora Ingress starting on {}", addr);
        my_server.run_forever();
    });


    axum::serve(listener, app.clone()).await?;
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

    let agent_ip = if let Some(existing) = peers.get(&req.pub_key) {
        info!("Node re-joined: name={} ip={}", existing.name, existing.ip);
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
    // DO NOT force endpoint here. WireGuard will dynamically learn the correct NAT endpoint 
    // when the peer sends its first handshake. Forcing it to `req.public_endpoint` breaks NAT.
    let _ = wg_cmd.status();

    info!("Node joined: role={} name={} pub_key={}... ip={}", role, name, &req.pub_key[..8], agent_ip);

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

async fn nodes_handler(State(state): State<AppState>) -> Json<Vec<NodeInfo>> {
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
            if now - ls > 600 {
                continue;
            }
        } else {
            continue;
        }

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

#[derive(Serialize)]
struct RepoInfo {
    name: String,
    clone_url: String,
}

async fn git_repos_handler(State(state): State<AppState>) -> Json<Vec<RepoInfo>> {
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
    info!("Created git repository: {}", repo_name);
    
    Ok(Json(RepoInfo { name: repo_name, clone_url }))
}

fn sha256_file(path: &str) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    Some(format!("{:x}", hasher.finalize()))
}

async fn agent_binary_handler() -> Response {
    match tokio::fs::read(AGENT_BINARY_PATH).await {
        Ok(data) => Response::builder()
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(header::CONTENT_DISPOSITION, "attachment; filename=\"r4a-agent\"")
            .body(Body::from(data))
            .unwrap(),
        Err(e) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from(format!("binary not found: {e}")))
            .unwrap(),
    }
}

#[derive(Serialize)]
struct ChecksumResponse {
    checksum: String,
}

async fn agent_checksum_handler() -> Result<Json<ChecksumResponse>, (StatusCode, String)> {
    match sha256_file(AGENT_BINARY_PATH) {
        Some(checksum) => Ok(Json(ChecksumResponse { checksum })),
        None => Err((StatusCode::NOT_FOUND, "binary not found".to_string())),
    }
}

#[derive(Serialize)]
struct TestResponse {
    ok: bool,
    checksum: Option<String>,
    message: String,
}

async fn update_test_handler() -> Json<TestResponse> {
    match sha256_file(AGENT_BINARY_PATH) {
        Some(checksum) => Json(TestResponse {
            ok: true,
            checksum: Some(checksum),
            message: "binary OK".to_string(),
        }),
        None => Json(TestResponse {
            ok: false,
            checksum: None,
            message: format!("binary not found at {AGENT_BINARY_PATH}"),
        }),
    }
}

async fn update_trigger_handler(State(state): State<AppState>) -> StatusCode {
    *state.update_pending.lock().unwrap() = true;
    info!("Update triggered for all agents");
    StatusCode::OK
}

#[derive(Serialize)]
struct UpdatePollResponse {
    update_pending: bool,
    checksum: Option<String>,
}

async fn update_poll_handler(State(state): State<AppState>) -> Json<UpdatePollResponse> {
    let update_pending = *state.update_pending.lock().unwrap();
    let checksum = if update_pending { sha256_file(AGENT_BINARY_PATH) } else { None };
    Json(UpdatePollResponse { update_pending, checksum })
}

#[derive(Deserialize)]
struct UpdateReport {
    agent_vpn_ip: String,
    checksum: String,
    status: String,
}

async fn update_report_handler(
    State(state): State<AppState>,
    Json(report): Json<UpdateReport>,
) -> StatusCode {
    let update_status = match report.status.as_str() {
        "updated" => AgentUpdateStatus::Updated,
        "updating" => AgentUpdateStatus::Updating,
        "failed" => AgentUpdateStatus::Failed("reported failure".to_string()),
        _ => AgentUpdateStatus::Unknown,
    };
    info!("Update report from {}: status={}", report.agent_vpn_ip, report.status);
    state.agent_update_states.lock().unwrap().insert(
        report.agent_vpn_ip,
        AgentUpdateState { status: update_status, checksum: Some(report.checksum) },
    );
    StatusCode::OK
}

#[derive(Serialize)]
struct UpdateStatusResponse {
    master_checksum: Option<String>,
    update_pending: bool,
    agents: HashMap<String, AgentUpdateStateDto>,
}

#[derive(Serialize)]
struct AgentUpdateStateDto {
    status: String,
    checksum: Option<String>,
}

async fn update_status_handler(State(state): State<AppState>) -> Json<UpdateStatusResponse> {
    let master_checksum = sha256_file(AGENT_BINARY_PATH);
    let update_pending = *state.update_pending.lock().unwrap();
    let states = state.agent_update_states.lock().unwrap();
    let agents = states.iter().map(|(ip, s)| {
        let status_str = match &s.status {
            AgentUpdateStatus::Unknown => "unknown",
            AgentUpdateStatus::Pending => "pending",
            AgentUpdateStatus::Updating => "updating",
            AgentUpdateStatus::Updated => "updated",
            AgentUpdateStatus::Failed(_) => "failed",
        }.to_string();
        (ip.clone(), AgentUpdateStateDto { status: status_str, checksum: s.checksum.clone() })
    }).collect();
    Json(UpdateStatusResponse { master_checksum, update_pending, agents })
}

fn get_external_ip() -> String {
    let out = std::process::Command::new("ip")
        .args(["-4", "addr", "show"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    
    let mut best_ip = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("inet ") && !line.contains("127.") && !line.contains("10.42.") {
            if let Some(ip) = line.split_whitespace().nth(1) {
                if let Some(ip) = ip.split('/').next() {
                    let ip_str = ip.to_string();
                    if ip_str.starts_with("100.") {
                        return ip_str; // Максимальный приоритет - Tailscale
                    } else if ip_str.starts_with("192.168.") {
                        best_ip = ip_str;
                    } else if best_ip.is_empty() && !ip_str.starts_with("10.") {
                        best_ip = ip_str;
                    }
                }
            }
        }
    }
    if !best_ip.is_empty() {
        return best_ip;
    }
    "127.0.0.1".to_string()
}

#[cfg(test)]
mod tests {
    use r4a_core::Manifest;

    #[test]
    fn test_manifest_parsing() {
        let content = r#"
[app]
name = "test-app"
node_selector = "home"

[container]
image = "alpine"
restart = "always"
command = ["sleep", "1000"]
"#;
        let manifest: Manifest = toml::from_str(content).unwrap();
        assert_eq!(manifest.app.name, "test-app");
        assert_eq!(manifest.container.unwrap().image, "alpine");
    }
}
