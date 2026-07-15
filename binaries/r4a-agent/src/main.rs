use anyhow::{Context, Result};
use axum::{
    extract::{Path, Query, State as AxumState},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use bollard::container::{ListContainersOptions, LogsOptions, RestartContainerOptions, StopContainerOptions, StartContainerOptions};
use bollard::Docker;
use clap::{Parser, Subcommand};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use futures_util::StreamExt;
use r4a_core::{Identity, JoinRequest, JoinResponse, Manifest, MetricsReport, PeerInfo};
use r4a_worker::Reconciler;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use sysinfo::System;
use tracing::{info, warn, error};

const AGENT_API_PORT: u16 = 8082;

// Must match the public key in r4a-server (C-2).
// DEV/TEST key — replace before production deployment.
const RELEASE_SIGNING_PUBKEY: [u8; 32] = [
    0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7,
    0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
    0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25,
    0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
];

fn verify_release_signature(data: &[u8], sig_bytes: &[u8]) -> anyhow::Result<()> {
    if std::env::var("R4A_SKIP_SIGNATURE_VERIFY").as_deref() == Ok("1") {
        warn!("SECURITY: signature verification skipped (R4A_SKIP_SIGNATURE_VERIFY=1) — DO NOT USE IN PRODUCTION");
        return Ok(());
    }
    let key = VerifyingKey::from_bytes(&RELEASE_SIGNING_PUBKEY)
        .map_err(|e| anyhow::anyhow!("invalid signing public key: {e}"))?;
    let sig_arr: [u8; 64] = sig_bytes.try_into()
        .map_err(|_| anyhow::anyhow!("invalid signature length: expected 64 bytes, got {}", sig_bytes.len()))?;
    let sig = Signature::from_bytes(&sig_arr);
    key.verify(data, &sig)
        .map_err(|e| anyhow::anyhow!("signature verification failed: {e}"))?;
    Ok(())
}

fn state_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".r4a-agent")
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

fn load_identity(secret: Option<String>) -> Result<Identity> {
    let path = state_dir().join("identity.json");
    if path.exists() {
        let data = std::fs::read_to_string(&path)?;
        let mut id: Identity = serde_json::from_str(&data)?;
        if secret.is_some() && id.cluster_secret != secret {
            id.cluster_secret = secret;
            save_identity(&id)?;
        }
        info!("Loaded existing identity, public key: {}", id.public_key);
        return Ok(id);
    }
    info!("Generating new WireGuard keypair...");
    let kp = r4a_vpn::wireguard::generate_keypair()?;
    let id = Identity {
        private_key: kp.private,
        public_key: kp.public,
        cluster_secret: secret,
        admin_secret: None,
        agent_token: None,
    };
    save_identity(&id)?;
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
    Connect {
        #[arg(long)]
        master: String,
        #[arg(long, env = "R4A_SECRET")]
        secret: Option<String>,
        #[arg(long)]
        name: Option<String>,
        /// Публичный host:port этого агента (если есть белый IP или проброшен порт) —
        /// другие агенты будут подключаться к нему напрямую, минуя хаб.
        #[arg(long, env = "R4A_PUBLIC_ENDPOINT")]
        public_endpoint: Option<String>,
    },
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Subcommand)]
enum ServiceAction {
    Enable {
        #[arg(long)]
        master: String,
        #[arg(long, env = "R4A_SECRET")]
        secret: Option<String>,
        #[arg(long)]
        name: Option<String>,
    },
    Disable,
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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Cmd::Connect { master, secret, name, public_endpoint } => connect(&master, secret, name, public_endpoint).await,
        Cmd::Service { action } => handle_service(action),
    }
}

fn handle_service(action: ServiceAction) -> Result<()> {
    let manager = r4a_service::ServiceManager::detect()?;
    match action {
        ServiceAction::Enable { master, secret, name } => {
            // H-4: secret goes into a 0o600 env file, NOT the command line
            let mut exec = format!("/usr/local/bin/r4a-agent connect --master {}", master);
            if let Some(n) = &name {
                exec.push_str(&format!(" --name {}", n));
            }
            let env_pairs: Vec<(&str, &str)> = secret.as_deref()
                .map(|s| vec![("R4A_SECRET", s)])
                .unwrap_or_default();
            manager.enable("r4a-agent", "r4a Agent Node", &exec, &env_pairs)?;
        }
        ServiceAction::Disable => {
            manager.disable("r4a-agent")?;
        }
    }
    Ok(())
}

#[derive(Clone)]
struct AgentApiState {
    cluster_secret: String,
    node_name: String,
}

#[derive(Serialize)]
struct ContainerInfo {
    id: String,
    name: String,
    image: String,
    status: String,
    state: String,
}

#[derive(Deserialize)]
struct LogsQuery {
    tail: Option<u64>,
}

async fn agent_containers_handler(
    AxumState(state): AxumState<AgentApiState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<Vec<ContainerInfo>>, (StatusCode, String)> {
    if !check_secret(&headers, &state.cluster_secret) {
        return Err((StatusCode::UNAUTHORIZED, "Unauthorized".to_string()));
    }
    let docker = Docker::connect_with_local_defaults()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut filters = HashMap::new();
    filters.insert("label".to_string(), vec![format!("r4a.node={}", state.node_name)]);
    let opts = ListContainersOptions { all: true, filters, ..Default::default() };

    let containers = docker.list_containers(Some(opts)).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let result = containers.into_iter().map(|c| {
        let name = c.names
            .and_then(|ns| ns.into_iter().next())
            .unwrap_or_default()
            .trim_start_matches('/')
            .to_string();
        ContainerInfo {
            id: c.id.unwrap_or_default(),
            name,
            image: c.image.unwrap_or_default(),
            status: c.status.unwrap_or_default(),
            state: c.state.unwrap_or_default(),
        }
    }).collect();

    Ok(Json(result))
}

async fn agent_logs_handler(
    AxumState(state): AxumState<AgentApiState>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
    Query(q): Query<LogsQuery>,
) -> Result<Json<Vec<String>>, (StatusCode, String)> {
    if !check_secret(&headers, &state.cluster_secret) {
        return Err((StatusCode::UNAUTHORIZED, "Unauthorized".to_string()));
    }
    let docker = Docker::connect_with_local_defaults()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let tail = q.tail.unwrap_or(200);
    let opts = LogsOptions::<String> {
        stdout: true,
        stderr: true,
        tail: tail.to_string(),
        timestamps: false,
        ..Default::default()
    };

    let mut stream = docker.logs(&name, Some(opts));
    let mut lines = Vec::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(output) => lines.push(output.to_string()),
            Err(e) => lines.push(format!("[error] {}", e)),
        }
    }

    Ok(Json(lines))
}

async fn agent_restart_handler(
    AxumState(state): AxumState<AgentApiState>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !check_secret(&headers, &state.cluster_secret) {
        return Err((StatusCode::UNAUTHORIZED, "Unauthorized".to_string()));
    }
    let docker = Docker::connect_with_local_defaults()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    docker.restart_container(&name, Some(RestartContainerOptions { t: 5 })).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::OK)
}

async fn agent_stop_handler(
    AxumState(state): AxumState<AgentApiState>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !check_secret(&headers, &state.cluster_secret) {
        return Err((StatusCode::UNAUTHORIZED, "Unauthorized".to_string()));
    }
    let docker = Docker::connect_with_local_defaults()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    docker.stop_container(&name, Some(StopContainerOptions { t: 5 })).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::OK)
}

async fn agent_start_handler(
    AxumState(state): AxumState<AgentApiState>,
    headers: axum::http::HeaderMap,
    Path(name): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !check_secret(&headers, &state.cluster_secret) {
        return Err((StatusCode::UNAUTHORIZED, "Unauthorized".to_string()));
    }
    let docker = Docker::connect_with_local_defaults()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    docker.start_container(&name, None::<StartContainerOptions<String>>).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::OK)
}

fn check_secret(headers: &axum::http::HeaderMap, expected: &str) -> bool {
    // H-1: constant-time comparison to prevent timing attacks
    headers.get("X-R4A-Secret")
        .and_then(|v| v.to_str().ok())
        .map(|v| constant_time_eq::constant_time_eq(v.as_bytes(), expected.as_bytes()))
        .unwrap_or(false)
}

fn spawn_agent_api(cluster_secret: String, node_name: String, bind_ip: String) {
    tokio::spawn(async move {
        let state = AgentApiState { cluster_secret, node_name };
        let app = Router::new()
            .route("/containers", get(agent_containers_handler))
            .route("/containers/:name/logs", get(agent_logs_handler))
            .route("/containers/:name/restart", post(agent_restart_handler))
            .route("/containers/:name/stop", post(agent_stop_handler))
            .route("/containers/:name/start", post(agent_start_handler))
            .with_state(state);

        // C-2a: bind only to the VPN interface, not 0.0.0.0
        let addr = format!("{}:{}", bind_ip, AGENT_API_PORT);
        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => { error!("Agent API bind failed on {}: {}", addr, e); return; }
        };
        info!("Agent API listening on {}", addr);
        if let Err(e) = axum::serve(listener, app).await {
            error!("Agent API error: {}", e);
        }
    });
}

async fn connect(master_api: &str, secret: Option<String>, name: Option<String>, my_public_endpoint: Option<String>) -> Result<()> {
    let name = name.unwrap_or_else(|| {
        System::host_name().unwrap_or_else(|| "agent".to_string())
    });

    let identity = load_identity(secret).context("Failed to load or generate identity")?;
    let cluster_secret = identity.cluster_secret.clone().unwrap_or_default();

    info!("Joining master at {} as '{}'...", master_api, name);
    let client = reqwest::Client::new();
    let resp: JoinResponse = client
        .post(format!("{master_api}/api/join"))
        .header("X-R4A-Secret", &cluster_secret)
        .json(&JoinRequest { 
            pub_key: identity.public_key.clone(), 
            name: Some(name.clone()),
            role: Some("agent".to_string()),
            public_endpoint: my_public_endpoint.clone(),
        })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    info!("Assigned VPN IP : {}", resp.agent_vpn_ip);
    info!("Master endpoint : {}", resp.master_endpoint);

    let mut identity = identity;
    if let Some(token) = resp.agent_token.clone() {
        identity.agent_token = Some(token);
        let _ = save_identity(&identity);
        info!("Saved agent token to identity.json");
    }

    info!("Setting up WireGuard interface...");
    r4a_vpn::wireguard::setup_agent(
        &identity.private_key,
        &resp.agent_vpn_ip,
        &resp.master_pub_key,
        &resp.master_endpoint,
    )?;

    let master_ips: Vec<String> = resp.peers
        .values()
        .filter(|p| p.role == "master")
        .map(|p| p.ip.clone())
        .collect();

    let mut hosts_ips: Vec<&str> = master_ips.iter().map(|s| s.as_str()).collect();
    if hosts_ips.is_empty() {
        hosts_ips.push("10.42.0.1");
    }

    info!("Adding master.r4a.local ({}) to /etc/hosts...", hosts_ips.join(", "));
    r4a_vpn::dns::set_hosts_entries(&hosts_ips, "master.r4a.local")?;

    info!("Agent '{}' connected. VPN IP: {}", name, resp.agent_vpn_ip);

    spawn_agent_api(cluster_secret.clone(), name.clone(), resp.agent_vpn_ip.clone());

    // Telemetry: стримим логи r4a-контейнеров в ClickHouse (когда настроен на мастере)
    tokio::spawn(r4a_telemetry::collector::run_collector(
        name.clone(),
        "http://master.r4a.local:3501".to_string(),
        cluster_secret.clone(),
        state_dir().join("logs-state.json"),
    ));

    // Имена нод с установленным прямым P2P-туннелем: пишет p2p-цикл, читает metrics-репорт
    let p2p_direct: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let vpn_ip = resp.agent_vpn_ip.clone();
    let metrics_secret = cluster_secret.clone();
    let metrics_p2p_direct = p2p_direct.clone();
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
                p2p_direct: Some(metrics_p2p_direct.lock().unwrap().clone()),
            };

            let _ = client
                .post("http://master.r4a.local:3501/api/metrics")
                .header("X-R4A-Secret", &metrics_secret)
                .json(&report)
                .send()
                .await;
        }
    });

    let master_base = "http://master.r4a.local:3501".to_string();
    let update_client = client.clone();
    let update_vpn_ip = resp.agent_vpn_ip.clone();
    let update_secret = cluster_secret.clone();

    // Report initial checksum so the master knows our current version immediately
    if let Some(cs) = sha256_self() {
        let _ = report_update_status(&update_client, &master_base, &update_vpn_ip, "idle", &cs, &update_secret).await;
    }
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
            if let Err(e) = check_and_apply_update(&update_client, &master_base, &update_vpn_ip, &update_secret).await {
                warn!("Update check failed: {e}");
            }
        }
    });

    let reconcile_client = client.clone();
    let reconciler_node_name = name.clone();
    let reconcile_secret = cluster_secret.clone();
    let reconcile_token = identity.agent_token.clone().unwrap_or_default();
    tokio::spawn(async move {
        let reconciler = match Reconciler::new(reconciler_node_name.clone(), reconcile_token.clone()) {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to initialize Reconciler: {}", e);
                return;
            }
        };
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
            let url = format!("http://master.r4a.local:3501/api/manifests?node={}", reconciler_node_name);
            let mut req = reconcile_client.get(&url);
            if !reconcile_token.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", reconcile_token));
            } else {
                req = req.header("X-R4A-Secret", &reconcile_secret);
            }

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        let text = resp.text().await.unwrap_or_default();
                        match serde_json::from_str::<HashMap<String, Manifest>>(&text) {
                            Ok(manifests) => {
                                if let Err(e) = reconciler.reconcile(manifests).await {
                                    error!("Reconcile error: {}", e);
                                }
                            }
                            Err(e) => {
                                error!("Failed to parse manifests JSON from master: {}. Body snippet: {}", e, &text[..text.len().min(100)]);
                            }
                        }
                    } else {
                        warn!("Failed to fetch manifests: HTTP {}", status);
                    }
                }
                Err(e) => warn!("Failed to fetch manifests: {}", e),
            }
        }
    });

    // P2P: прямые туннели агент↔агент в обход хаба
    let p2p_secret = cluster_secret.clone();
    let p2p_my_pubkey = identity.public_key.clone();
    let p2p_master_pubkey = resp.master_pub_key.clone();
    let p2p_direct_out = p2p_direct.clone();
    tokio::spawn(async move {
        run_p2p_sync(p2p_secret, p2p_my_pubkey, p2p_master_pubkey, p2p_direct_out).await;
    });

    tokio::signal::ctrl_c().await?;
    Ok(())
}

// ── P2P: прямые WG-туннели между агентами ─────────────────────────────────────
//
// Механика: мастер видит реальный ip:port каждого агента после NAT (wg show
// endpoints) и раздаёт их через /api/peers. Оба агента добавляют друг друга
// peer'ом с AllowedIPs <ip>/32 — маршрут специфичнее хабового /24, трафик идёт
// напрямую; keepalive с обеих сторон пробивает NAT (full-cone/restricted).
// Если handshake не появился — peer удаляется (маршрут откатывается на хаб)
// и ретрай с экспоненциальным backoff. Symmetric NAT так не пробить — там
// остаётся релей через мастера.

struct P2pPeerState {
    endpoint: String,
    added_at: u64,
    established: bool,
    failures: u32,
    backoff_until: u64,
}

const P2P_SYNC_INTERVAL_SECS: u64 = 30;
// Сколько ждём первый handshake после добавления peer'а
const P2P_GRACE_SECS: u64 = 60;
// Handshake старше этого = туннель мёртв (штатно wg делает handshake каждые ~2 мин)
const P2P_STALE_SECS: u64 = 180;

fn p2p_backoff_secs(failures: u32) -> u64 {
    match failures {
        0 | 1 => 60,
        2 => 300,
        _ => 1800,
    }
}

async fn run_p2p_sync(
    cluster_secret: String,
    my_pubkey: String,
    master_pubkey: String,
    p2p_direct_out: Arc<Mutex<Vec<String>>>,
) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();
    let mut states: HashMap<String, P2pPeerState> = HashMap::new();

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(P2P_SYNC_INTERVAL_SECS)).await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let peers: HashMap<String, PeerInfo> = match client
            .get("http://master.r4a.local:3501/api/peers")
            .header("X-R4A-Secret", &cluster_secret)
            .send()
            .await
            .and_then(|r| r.error_for_status())
        {
            Ok(r) => match r.json().await {
                Ok(p) => p,
                Err(e) => {
                    warn!("p2p: bad /api/peers response: {}", e);
                    continue;
                }
            },
            Err(e) => {
                warn!("p2p: /api/peers failed: {}", e);
                continue;
            }
        };

        let handshakes = r4a_vpn::wireguard::latest_handshakes().unwrap_or_default();

        // Кандидаты: другие агенты с известным endpoint (явный public_endpoint
        // приоритетнее наблюдаемого мастером)
        for peer in peers.values() {
            if peer.pub_key == my_pubkey || peer.pub_key == master_pubkey || peer.role != "agent" {
                continue;
            }
            let endpoint = match peer.public_endpoint.as_ref().or(peer.observed_endpoint.as_ref()) {
                Some(ep) => ep.clone(),
                None => continue,
            };

            match states.get_mut(&peer.pub_key) {
                None => {
                    if try_add_p2p_peer(peer, &endpoint) {
                        states.insert(peer.pub_key.clone(), P2pPeerState {
                            endpoint, added_at: now, established: false, failures: 0, backoff_until: 0,
                        });
                    }
                }
                Some(st) if st.backoff_until > 0 => {
                    // В backoff: пробуем снова, когда время пришло или у peer'а новый endpoint
                    if now >= st.backoff_until || st.endpoint != endpoint {
                        if try_add_p2p_peer(peer, &endpoint) {
                            st.endpoint = endpoint;
                            st.added_at = now;
                            st.established = false;
                            st.backoff_until = 0;
                        }
                    }
                }
                Some(st) => {
                    // Активный p2p-peer: следим за handshake
                    let hs = handshakes.get(&peer.pub_key).copied().unwrap_or(0);
                    let alive = hs > 0 && now.saturating_sub(hs) < P2P_STALE_SECS;
                    if alive {
                        if !st.established {
                            st.established = true;
                            st.failures = 0;
                            info!("p2p established with '{}' via {}", peer.name, st.endpoint);
                        }
                        // Endpoint у peer'а сменился (переехал NAT) — переподключаемся
                        if st.endpoint != endpoint && peer.public_endpoint.is_some() {
                            if try_add_p2p_peer(peer, &endpoint) {
                                st.endpoint = endpoint;
                                st.added_at = now;
                                st.established = false;
                            }
                        }
                    } else if now.saturating_sub(st.added_at) > P2P_GRACE_SECS {
                        // Туннель не пробился или умер → откат на релей через хаб
                        st.failures += 1;
                        st.backoff_until = now + p2p_backoff_secs(st.failures);
                        st.established = false;
                        warn!(
                            "p2p with '{}' failed ({} attempt(s)), falling back to hub relay, retry in {}s",
                            peer.name, st.failures, p2p_backoff_secs(st.failures)
                        );
                        if let Err(e) = r4a_vpn::wireguard::remove_peer(&peer.pub_key) {
                            warn!("p2p: remove_peer failed: {}", e);
                        }
                    }
                }
            }
        }

        // Ноды, исчезнувшие из кластера — убираем их p2p-peer'ов
        let gone: Vec<String> = states.keys().filter(|pk| !peers.contains_key(*pk)).cloned().collect();
        for pk in gone {
            let st = states.remove(&pk);
            if st.map(|s| s.backoff_until == 0).unwrap_or(false) {
                let _ = r4a_vpn::wireguard::remove_peer(&pk);
            }
        }

        // Публикуем список established-пиров для metrics-репорта
        let mut established: Vec<String> = states.iter()
            .filter(|(_, st)| st.established)
            .filter_map(|(pk, _)| peers.get(pk).map(|p| p.name.clone()))
            .collect();
        established.sort();
        *p2p_direct_out.lock().unwrap() = established;
    }
}

fn try_add_p2p_peer(peer: &PeerInfo, endpoint: &str) -> bool {
    match r4a_vpn::wireguard::add_peer_with_endpoint(&peer.pub_key, &peer.ip, endpoint) {
        Ok(()) => {
            info!("p2p: trying direct tunnel to '{}' ({}) via {}", peer.name, peer.ip, endpoint);
            true
        }
        Err(e) => {
            warn!("p2p: add peer '{}' failed: {}", peer.name, e);
            false
        }
    }
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

async fn check_and_apply_update(client: &reqwest::Client, master: &str, vpn_ip: &str, secret: &str) -> Result<()> {
    let poll: UpdatePollResponse = client
        .get(format!("{master}/api/update/poll"))
        .header("X-R4A-Secret", secret)
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
        // Already on latest — report so master can clear update_pending
        let _ = report_update_status(client, master, vpn_ip, "updated", &self_checksum, secret).await;
        return Ok(());
    }

    info!("Update available (master={} self={}), downloading...", &master_checksum[..8], &self_checksum[..8]);

    let _ = report_update_status(client, master, vpn_ip, "updating", &self_checksum, secret).await;

    // C-2: download binary and its Ed25519 signature in parallel
    let bytes = client
        .get(format!("{master}/api/agent-binary"))
        .header("X-R4A-Secret", secret)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    let sig_response = client
        .get(format!("{master}/api/agent-binary-sig"))
        .header("X-R4A-Secret", secret)
        .send()
        .await?;

    let skip_verify = std::env::var("R4A_SKIP_SIGNATURE_VERIFY").as_deref() == Ok("1");

    if sig_response.status().is_success() {
        let sig_bytes = sig_response.bytes().await?;
        verify_release_signature(&bytes, &sig_bytes)
            .map_err(|e| {
                let _ = tokio::runtime::Handle::current()
                    .block_on(report_update_status(client, master, vpn_ip, "failed", &master_checksum, secret));
                e
            })?;
        info!("Binary signature verified successfully");
    } else if skip_verify {
        warn!("SECURITY: no signature available from master (R4A_SKIP_SIGNATURE_VERIFY=1) — skipping verification");
    } else {
        let _ = report_update_status(client, master, vpn_ip, "failed", &master_checksum, secret).await;
        anyhow::bail!("master has no signature for agent binary (HTTP {}): refusing to apply unsigned update", sig_response.status());
    }

    // Verify SHA256 checksum against what the master advertised
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let downloaded_checksum = format!("{:x}", hasher.finalize());
    if downloaded_checksum != master_checksum {
        let _ = report_update_status(client, master, vpn_ip, "failed", &downloaded_checksum, secret).await;
        anyhow::bail!("checksum mismatch: expected {master_checksum} got {downloaded_checksum}");
    }

    // Write to a unique temp path to avoid symlink attacks (C-2/M-4)
    let tmp_path = format!("/tmp/r4a-agent-{}.new", std::process::id());
    let target_path = "/usr/local/bin/r4a-agent";
    std::fs::write(&tmp_path, &bytes)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))?;
    }

    std::fs::rename(&tmp_path, target_path)?;
    info!("Updated to checksum {}, restarting...", &master_checksum[..8]);

    let _ = report_update_status(client, master, vpn_ip, "updated", &master_checksum, secret).await;

    std::process::exit(0);
}

async fn report_update_status(
    client: &reqwest::Client,
    master: &str,
    vpn_ip: &str,
    status: &str,
    checksum: &str,
    secret: &str,
) -> Result<()> {
    #[derive(Serialize)]
    struct Report<'a> { agent_vpn_ip: &'a str, checksum: &'a str, status: &'a str }
    client
        .post(format!("{master}/api/update/report"))
        .header("X-R4A-Secret", secret)
        .json(&Report { agent_vpn_ip: vpn_ip, checksum, status })
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}
