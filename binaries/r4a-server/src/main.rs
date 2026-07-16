use anyhow::Result;
use argon2::{password_hash::SaltString, Argon2, PasswordHasher};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, get, post},
    Json, Router,
};
use bollard::{
    container::{
        ListContainersOptions, LogsOptions, RestartContainerOptions, StartContainerOptions,
        StopContainerOptions,
    },
    Docker,
};
use constant_time_eq::constant_time_eq;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use futures_util::StreamExt;
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyUsagePurpose, SanType,
};

use clap::{Parser, Subcommand};
use r4a_core::{
    models::{
        AgentUpdateState, AgentUpdateStatus, Binding, ConnectRequest, ConnectResponse, Connection,
        Identity, JoinRequest, JoinResponse, MetricsReport, NodeInfo, PeerInfo, Policy, Resource,
        Rule, Token, User, VaultConfig, VaultSecret, Verb,
    },
    Manifest,
};
use tower_http::cors::{AllowOrigin, CorsLayer};

use r4a_store::Store;
use rand::{thread_rng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use sysinfo::System;
use tracing::{error, info, warn};

const WG_PORT: u16 = 51820;
const API_PORT: u16 = 3501;
const INGRESS_PORT: u16 = 3500;
const HTTPS_PORT: u16 = 443;

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

fn generate_secret() -> String {
    let mut secret = [0u8; 32];
    thread_rng().fill_bytes(&mut secret);
    hex::encode(secret)
}

fn load_identity() -> Result<Identity> {
    let path = state_dir().join("identity.json");
    let env_secret = std::env::var("R4A_SECRET").ok();
    let env_admin_secret = std::env::var("R4A_ADMIN_SECRET").ok();
    let env_node_name = std::env::var("R4A_NODE_NAME").ok();

    if path.exists() {
        let data = std::fs::read_to_string(&path)?;
        let mut id: Identity = serde_json::from_str(&data)?;

        if let Some(secret) = env_secret {
            if id.cluster_secret.as_ref() != Some(&secret) {
                id.cluster_secret = Some(secret);
                save_identity(&id)?;
            }
        } else if id.cluster_secret.is_none() {
            id.cluster_secret = Some(generate_secret());
            save_identity(&id)?;
        }

        if let Some(secret) = env_admin_secret {
            if id.admin_secret.as_ref() != Some(&secret) {
                id.admin_secret = Some(secret);
                save_identity(&id)?;
            }
        } else if id.admin_secret.is_none() {
            let secret = generate_secret();
            info!(
                "Generated admin secret (for web/CLI login), stored in {}",
                path.display()
            );
            id.admin_secret = Some(secret);
            save_identity(&id)?;
        }

        if let Some(name) = env_node_name {
            if id.node_name.as_deref() != Some(name.as_str()) {
                id.node_name = Some(name);
                save_identity(&id)?;
            }
        } else if id.node_name.is_none() {
            id.node_name = Some("master".to_string());
            save_identity(&id)?;
        }

        info!("Loaded existing identity, public key: {}", id.public_key);
        return Ok(id);
    }

    let kp = r4a_vpn::wireguard::generate_keypair()?;

    let secret = env_secret.unwrap_or_else(generate_secret);
    let admin_secret = env_admin_secret.unwrap_or_else(|| {
        let s = generate_secret();
        info!(
            "Generated admin secret (for web/CLI login), stored in {}",
            path.display()
        );
        s
    });

    let id = Identity {
        private_key: kp.private,
        public_key: kp.public,
        cluster_secret: Some(secret),
        admin_secret: Some(admin_secret),
        agent_token: None,
        node_name: Some(env_node_name.unwrap_or_else(|| "master".to_string())),
    };
    save_identity(&id)?;
    Ok(id)
}

#[derive(Clone)]
struct AppState {
    master_pub_key: String,
    cluster_secret: String,
    admin_secret: String,
    my_vpn_ip: String,
    my_node_name: String,
    peers: Arc<Mutex<HashMap<String, PeerInfo>>>,
    next_ip: Arc<Mutex<u16>>,
    update_pending: Arc<Mutex<bool>>,
    agent_update_states: Arc<Mutex<HashMap<String, AgentUpdateState>>>,
    store: Store,
    log_store: r4a_telemetry::store::LogStore,
}

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

struct RequireSecret;

#[async_trait::async_trait]
impl FromRequestParts<AppState> for RequireSecret {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(auth_header) = parts.headers.get("X-R4A-Secret") {
            if let Ok(auth_str) = auth_header.to_str() {
                // H-1: constant-time comparison to prevent timing attacks
                if constant_time_eq(auth_str.as_bytes(), state.cluster_secret.as_bytes()) {
                    return Ok(RequireSecret);
                }
            }
        }
        Err((
            StatusCode::UNAUTHORIZED,
            "Invalid or missing X-R4A-Secret header",
        ))
    }
}

// Admin login secret: unlike the cluster secret, agents never hold this,
// so only a real administrator can exchange it for an admin token.
struct RequireAdminSecret;

#[async_trait::async_trait]
impl FromRequestParts<AppState> for RequireAdminSecret {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(auth_header) = parts.headers.get("X-R4A-Secret") {
            if let Ok(auth_str) = auth_header.to_str() {
                if !state.admin_secret.is_empty()
                    && constant_time_eq(auth_str.as_bytes(), state.admin_secret.as_bytes())
                {
                    return Ok(RequireAdminSecret);
                }
            }
        }
        Err((
            StatusCode::UNAUTHORIZED,
            "Invalid or missing X-R4A-Secret header (admin secret required)",
        ))
    }
}

struct RequireToken {
    pub token: Token,
}

#[async_trait::async_trait]
impl FromRequestParts<AppState> for RequireToken {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(auth_header) = parts.headers.get("Authorization") {
            if let Ok(auth_str) = auth_header.to_str() {
                if let Some(token_str) = auth_str.strip_prefix("Bearer ") {
                    if let Ok(Some(token)) = state.store.get_token(token_str) {
                        return Ok(RequireToken { token });
                    }
                }
            }
        }

        Err((
            StatusCode::UNAUTHORIZED,
            "Invalid or missing Authorization header",
        ))
    }
}

enum Auth {
    Token(Token),
    Secret,
}

#[async_trait::async_trait]
impl FromRequestParts<AppState> for Auth {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(auth_header) = parts.headers.get("Authorization") {
            if let Ok(auth_str) = auth_header.to_str() {
                if let Some(token_str) = auth_str.strip_prefix("Bearer ") {
                    if let Ok(Some(token)) = state.store.get_token(token_str) {
                        return Ok(Auth::Token(token));
                    }
                }
            }
        }

        if let Some(auth_header) = parts.headers.get("X-R4A-Secret") {
            if let Ok(auth_str) = auth_header.to_str() {
                if constant_time_eq(auth_str.as_bytes(), state.cluster_secret.as_bytes()) {
                    return Ok(Auth::Secret);
                }
            }
        }

        Err((
            StatusCode::UNAUTHORIZED,
            "Invalid or missing authentication (Token or Secret)",
        ))
    }
}

#[derive(Parser)]
#[command(name = "r4a-server", about = "r4a Master Node")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
    /// Публичный endpoint WireGuard (host:port), который получат агенты.
    /// Нужен на облаках с 1:1 NAT (AWS/GCP), где на интерфейсе приватный IP.
    #[arg(long, global = true)]
    public_endpoint: Option<String>,
    /// Имя мастер-ноды (по умолчанию "master"). Сохраняется в identity.json.
    #[arg(long, global = true)]
    node_name: Option<String>,
}

#[derive(Subcommand)]
enum Cmd {
    Init,
    /// Ставит зависимости (WireGuard) через apt/brew, генерит секреты кластера
    /// и запускает r4a-server как системный сервис — для быстрого первого разворачивания.
    Install,
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
const SERVER_BINARY_PATH: &str = "/usr/local/bin/r4a-server";
const TUI_BINARY_PATH: &str = "/usr/local/bin/r4a-tui";

const AGENT_SIG_PATH: &str = "/usr/local/bin/r4a-agent.sig";
const SERVER_SIG_PATH: &str = "/usr/local/bin/r4a-server.sig";
const TUI_SIG_PATH: &str = "/usr/local/bin/r4a-tui.sig";

// Ed25519 public key for verifying official release binaries (C-2).
// IMPORTANT: replace this with the real signing key before production deployment.
// Generate keypair: `openssl genpkey -algorithm ed25519 -out signing.pem`
// Export pubkey bytes: `openssl pkey -in signing.pem -pubout -outform DER | tail -c 32 | xxd -i`
// Sign a binary: `openssl pkeyutl -sign -inkey signing.pem -rawin -in r4a-agent -out r4a-agent.sig`
//
// This is a DEV/TEST key — NOT FOR PRODUCTION.
const RELEASE_SIGNING_PUBKEY: [u8; 32] = [
    0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
    0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
];

/// Verifies an Ed25519 signature over binary data using the hardcoded release key.
/// Set env var R4A_SKIP_SIGNATURE_VERIFY=1 to bypass in development (logs a warning).
fn verify_release_signature(data: &[u8], sig_bytes: &[u8]) -> anyhow::Result<()> {
    if std::env::var("R4A_SKIP_SIGNATURE_VERIFY").as_deref() == Ok("1") {
        warn!("SECURITY: signature verification skipped (R4A_SKIP_SIGNATURE_VERIFY=1) — DO NOT USE IN PRODUCTION");
        return Ok(());
    }
    let key = VerifyingKey::from_bytes(&RELEASE_SIGNING_PUBKEY)
        .map_err(|e| anyhow::anyhow!("invalid signing public key: {e}"))?;
    let sig_arr: [u8; 64] = sig_bytes.try_into().map_err(|_| {
        anyhow::anyhow!(
            "invalid signature length: expected 64 bytes, got {}",
            sig_bytes.len()
        )
    })?;
    let sig = Signature::from_bytes(&sig_arr);
    key.verify(data, &sig)
        .map_err(|e| anyhow::anyhow!("signature verification failed: {e}"))?;
    Ok(())
}

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
        if let Err(e) = r4a_vpn::dns::set_hosts_entries(&ips_ref, "master.r4a.local") {
            warn!("Failed to update /etc/hosts: {}", e);
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // rustls 0.23 requires an explicit crypto provider when multiple are compiled in
    let _ = rustls::crypto::ring::default_provider().install_default();
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    if let Some(ep) = &cli.public_endpoint {
        r4a_vpn::wireguard::validate_endpoint(ep)?;
        std::env::set_var("R4A_PUBLIC_ENDPOINT", ep);
    }
    if let Some(name) = &cli.node_name {
        r4a_vpn::wireguard::validate_node_name(name)?;
        std::env::set_var("R4A_NODE_NAME", name);
    }
    match cli.command {
        Cmd::Init => init("10.42.0.1", None).await,
        Cmd::Install => install(),
        Cmd::JoinMaster { master, name } => join_master(&master, name).await,
        Cmd::PruneNodes => prune_nodes().await,
        Cmd::Service { action } => handle_service(action),
    }
}

/// Best-effort dependency install: WireGuard tooling via the platform's
/// package manager. Failures are logged but non-fatal — the user may already
/// have these installed some other way.
fn install_dependencies() {
    if cfg!(target_os = "linux") {
        info!("Installing WireGuard dependencies via apt-get...");
        let _ = std::process::Command::new("apt-get")
            .args(["update"])
            .status();
        match std::process::Command::new("apt-get")
            .args(["install", "-y", "wireguard-tools", "iproute2", "iptables"])
            .status()
        {
            Ok(s) if s.success() => info!("apt-get install succeeded"),
            _ => warn!(
                "apt-get install failed — install wireguard-tools, iproute2, iptables manually"
            ),
        }
    } else if cfg!(target_os = "macos") {
        info!("Installing WireGuard dependencies via brew...");
        match std::process::Command::new("brew")
            .args(["install", "wireguard-tools", "wireguard-go"])
            .status()
        {
            Ok(s) if s.success() => info!("brew install succeeded"),
            _ => warn!("brew install failed — install wireguard-tools, wireguard-go manually"),
        }
    } else {
        warn!("Unsupported OS — install WireGuard dependencies manually");
    }
}

/// One-shot bootstrap for a fresh master node: installs WireGuard deps,
/// generates (or loads) the cluster identity/secrets, prints them, and
/// starts r4a-server as a system service.
fn install() -> Result<()> {
    install_dependencies();

    let identity = load_identity()?;
    handle_service(ServiceAction::Enable)?;

    let cluster_secret = identity.cluster_secret.clone().unwrap_or_default();
    let admin_secret = identity.admin_secret.clone().unwrap_or_default();

    println!();
    println!("=== r4a-server installed and running ===");
    println!("Cluster join secret (R4A_SECRET, для агентов/r4a-cli connect):");
    println!("  {}", cluster_secret);
    println!("Admin secret (R4A_ADMIN_SECRET, для управления через CLI/TUI/Web):");
    println!("  {}", admin_secret);
    println!("Сохраните оба секрета в надёжном месте — они не выводятся повторно.");

    Ok(())
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
            // Прокидываем публичный endpoint в сервис, если задан при установке
            let ep = std::env::var("R4A_PUBLIC_ENDPOINT").ok();
            let env_pairs: Vec<(&str, &str)> = ep
                .as_deref()
                .map(|e| vec![("R4A_PUBLIC_ENDPOINT", e)])
                .unwrap_or_default();
            manager.enable("r4a-server", "r4a Master Node", exec, &env_pairs)?;
        }
        ServiceAction::Disable => {
            manager.disable("r4a-server")?;
        }
    }
    Ok(())
}

async fn join_master(first_master_url: &str, name: Option<String>) -> Result<()> {
    let identity = load_identity()?;
    let my_endpoint = public_endpoint();
    let my_name = name.unwrap_or_else(|| {
        identity
            .node_name
            .clone()
            .unwrap_or_else(|| "master-node".to_string())
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
        .args([
            "set",
            "wg0",
            "peer",
            &resp.master_pub_key,
            "endpoint",
            &initial_endpoint,
        ])
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
    let my_endpoint = public_endpoint();
    let my_name = identity
        .node_name
        .clone()
        .unwrap_or_else(|| "master".to_string());

    if !peers.contains_key(&identity.public_key) {
        peers.insert(
            identity.public_key.clone(),
            PeerInfo {
                pub_key: identity.public_key.clone(),
                ip: my_vpn_ip.to_string(),
                name: my_name.clone(),
                role: "master".to_string(),
                public_endpoint: Some(my_endpoint.clone()),
                observed_endpoint: None,
                p2p_direct: None,
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

    let saved_peers = load_peers(&store);
    // Account for connection IPs too (pinned labels + active connections),
    // otherwise a master restart could hand out an already-taken IP
    let connection_ips: Vec<String> = store
        .list_connections()
        .map(|conns| conns.into_iter().map(|c| c.vpn_ip).collect())
        .unwrap_or_default();
    let label_ips: Vec<String> = store
        .db
        .open_tree("connection_labels")
        .map(|tree| {
            tree.iter()
                .filter_map(|item| item.ok())
                .map(|(_, v)| String::from_utf8_lossy(&v).to_string())
                .collect()
        })
        .unwrap_or_default();
    let next_ip = saved_peers
        .values()
        .map(|p| p.ip.as_str())
        .chain(connection_ips.iter().map(String::as_str))
        .chain(label_ips.iter().map(String::as_str))
        .filter_map(|ip| ip.split('.').last()?.parse::<u16>().ok())
        .max()
        .map(|m| m + 1)
        .unwrap_or(2);

    let admin_secret = identity.admin_secret.clone().unwrap_or_default();
    if admin_secret.is_empty() {
        warn!("Admin secret is empty — /api/tokens/exchange will reject all requests");
    }

    let log_store = r4a_telemetry::store::LogStore::open(state_dir().join("logs-db"))?;

    let my_node_name = identity
        .node_name
        .clone()
        .unwrap_or_else(|| "master".to_string());

    let state = AppState {
        master_pub_key: identity.public_key.clone(),
        cluster_secret: cluster_secret.clone(),
        admin_secret,
        my_vpn_ip: my_vpn_ip.clone(),
        my_node_name: my_node_name.clone(),
        peers: Arc::new(Mutex::new(saved_peers)),
        next_ip: Arc::new(Mutex::new(next_ip)),
        update_pending: Arc::new(Mutex::new(false)),
        agent_update_states: Arc::new(Mutex::new(HashMap::new())),
        store: store.clone(),
        log_store: log_store.clone(),
    };

    // Retention: удаляем точки метрик старше 3 суток раз в час
    // (логи контейнеров живут в ClickHouse со своим TTL)
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            match log_store.prune_metrics(3 * 24 * 3600) {
                Ok(0) => {}
                Ok(n) => info!("Metrics retention: pruned {} old points", n),
                Err(e) => warn!("Metrics retention failed: {}", e),
            }
        }
    });

    store.set_secret(cluster_secret.clone());

    // Migrate manifests from old single-blob format to per-entry format
    if let Ok(Some(data)) = store.get("core", b"manifests") {
        if let Ok(old) = serde_json::from_slice::<HashMap<String, Manifest>>(&data) {
            let count = old.len();
            for (_, manifest) in old {
                let _ = store.put_manifest(&manifest).await;
            }
            let _ = store.delete("core", b"manifests").await;
            info!(
                "Migrated {} manifest(s) from old blob format to store",
                count
            );
        }
    }

    let _ = store.migrate_rbac_v1_to_v2();

    // Master can host system workloads explicitly targeted to the master node
    // (for example ClickHouse logs storage). Do not include node_selector="all":
    // historically only agents reconciled "all", and changing that would move
    // arbitrary workloads onto the control-plane node.
    let master_reconcile_store = store.clone();
    let master_reconcile_name = my_node_name.clone();
    let master_reconcile_ip = my_vpn_ip.clone();
    tokio::spawn(async move {
        let reconciler =
            match r4a_worker::Reconciler::new(master_reconcile_name.clone(), String::new()) {
                Ok(r) => r,
                Err(e) => {
                    warn!("Master reconciler disabled: {}", e);
                    return;
                }
            };
        loop {
            let manifests = match master_reconcile_store.list_manifests() {
                Ok(items) => items
                    .into_iter()
                    .filter(|m| {
                        m.app.node_selector == master_reconcile_name
                            || m.app.node_selector == master_reconcile_ip
                    })
                    .map(|m| (m.app.name.clone(), m))
                    .collect(),
                Err(e) => {
                    warn!("Master reconciler failed to list manifests: {}", e);
                    HashMap::new()
                }
            };
            if let Err(e) = reconciler.reconcile(manifests).await {
                warn!("Master reconcile failed: {}", e);
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    });

    // Telemetry collector for r4a-managed containers hosted on the master itself.
    // Agents run their own collector; master needs one now that it can host
    // system workloads such as ClickHouse.
    tokio::spawn(r4a_telemetry::collector::run_collector(
        my_node_name.clone(),
        "http://127.0.0.1:3501".to_string(),
        cluster_secret.clone(),
        state_dir().join("logs-state.json"),
    ));

    // Догоняем схему ClickHouse (в т.ч. line_ngram индекс поиска) при старте, если
    // логи уже были настроены раньше — /api/logs/setup запускается только один раз
    // на деплой, а не при каждом рестарте мастера.
    let schema_store = store.clone();
    tokio::spawn(async move {
        if let Ok(Some(cfg)) = load_active_logs_ch_config(&schema_store).await {
            ensure_logs_ch_schema(cfg).await;
        }
    });

    // Background task: expire connections that haven't sent a heartbeat in 90s
    let cleanup_store = store.clone();
    tokio::spawn(async move {
        const TIMEOUT_SECS: u64 = 90;
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if let Ok(conns) = cleanup_store.list_connections() {
                for conn in conns {
                    if now.saturating_sub(conn.last_seen) > TIMEOUT_SECS {
                        info!("Expiring stale connection {} ({})", conn.id, conn.vpn_ip);
                        if let Err(e) = r4a_vpn::wireguard::remove_peer(&conn.pubkey) {
                            warn!("WireGuard remove_peer (cleanup) failed: {}", e);
                        }
                        let _ = cleanup_store.delete_connection(&conn.id).await;
                    }
                }
            }
        }
    });

    // DNS server for *.r4a.local
    let dns_store = store.clone();
    let dns_vpn_ip = my_vpn_ip.clone();
    tokio::spawn(async move {
        run_dns_server(dns_vpn_ip, dns_store).await;
    });

    // TLS certs: generate CA + server cert on first start
    let (server_cert_pem, server_key_pem) = match ensure_tls_certs(&store).await {
        Ok(pair) => pair,
        Err(e) => {
            warn!("Failed to generate TLS certs: {} — HTTPS proxy disabled", e);
            (String::new(), String::new())
        }
    };
    if !server_cert_pem.is_empty() {
        let https_vpn_ip = my_vpn_ip.clone();
        tokio::spawn(async move {
            start_https_proxy(https_vpn_ip, server_cert_pem, server_key_pem).await;
        });
    }

    let broadcast_state = state.clone();
    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap_or_default();
        let mut sys = System::new_all();
        let my_name = broadcast_state.my_node_name.clone();
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
                p2p_direct: None,
            };

            // Своя история метрик — в telemetry-store (агенты пишутся в metrics_handler)
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let _ = broadcast_state
                .log_store
                .append_metric(&r4a_telemetry::MetricPoint {
                    node: my_name.clone(),
                    ts_ms: now_ms,
                    cpu_percent: report.cpu_percent,
                    ram_used_mb: report.ram_used_mb,
                    ram_total_mb: report.ram_total_mb,
                    vram_used_mb: report.vram_used_mb,
                    vram_total_mb: report.vram_total_mb,
                });

            let masters: Vec<String> = {
                let peers = broadcast_state.peers.lock().unwrap();
                peers
                    .values()
                    .filter(|p| p.role == "master" && p.ip != broadcast_state.my_vpn_ip)
                    .map(|p| p.ip.clone())
                    .collect()
            };

            for master_ip in masters {
                let _ = client
                    .post(format!("http://{master_ip}:3501/api/metrics"))
                    .header("X-R4A-Secret", &broadcast_state.cluster_secret)
                    .json(&report)
                    .send()
                    .await;
            }
        }
    });

    // P2P: наблюдаемые endpoint'ы агентов из `wg show` → peers map (мастер как STUN)
    let observed_state = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;
            let observed = match r4a_vpn::wireguard::observed_endpoints() {
                Ok(map) => map,
                Err(e) => {
                    warn!("wg show endpoints failed: {}", e);
                    continue;
                }
            };
            let mut peers = observed_state.peers.lock().unwrap();
            for peer in peers.values_mut() {
                peer.observed_endpoint = observed.get(&peer.pub_key).cloned();
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
                let _ = r4a_vpn::dns::set_hosts_entries(&ips_ref, "master.r4a.local");
            }

            hosts_state.store.set_masters(master_ips);
        }
    });

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/api/join", post(join_handler))
        .route("/api/peers", get(peers_handler))
        .route("/api/nodes", get(nodes_handler))
        .route("/api/metrics", post(metrics_handler))
        .route("/api/metrics/history", get(metrics_history_handler))
        .route(
            "/api/manifests",
            get(manifests_handler)
                .post(manifest_upsert_handler)
                .delete(manifest_delete_handler),
        )
        .route(
            "/api/vault",
            get(vault_get_handler)
                .post(vault_set_handler)
                .delete(vault_delete_handler),
        )
        .route("/api/vault/list", get(vault_list_handler))
        .route(
            "/api/vault/configs",
            get(vault_configs_list_handler).post(vault_config_create_handler),
        )
        .route(
            "/api/tokens",
            get(tokens_list_handler)
                .post(token_create_handler)
                .delete(token_delete_handler),
        )
        .route("/api/tokens/exchange", post(token_exchange_handler))
        .route(
            "/api/users",
            get(users_list_handler)
                .post(user_create_handler)
                .delete(user_delete_handler),
        )
        .route(
            "/api/git/repos",
            get(git_repos_handler).post(git_create_repo_handler),
        )
        .route("/api/registry/repos", get(registry_repos_handler))
        .route(
            "/api/registry/repos/*rest",
            get(registry_repo_tags_handler).delete(registry_repo_tag_delete_handler),
        )
        .route("/api/agent-binary", get(agent_binary_handler))
        .route("/api/server-binary", get(server_binary_handler))
        .route("/api/tui-binary", get(tui_binary_handler))
        .route("/api/agent-checksum", get(agent_checksum_handler))
        .route("/api/server-checksum", get(server_checksum_handler))
        .route("/api/tui-checksum", get(tui_checksum_handler))
        .route("/api/agent-binary-sig", get(agent_binary_sig_handler))
        .route("/api/server-binary-sig", get(server_binary_sig_handler))
        .route("/api/tui-binary-sig", get(tui_binary_sig_handler))
        .route("/api/update/test", post(update_test_handler))
        .route("/api/update/trigger", post(update_trigger_handler))
        .route(
            "/api/update/server-trigger",
            post(server_update_server_trigger_handler),
        )
        .route(
            "/api/update/fetch-github",
            post(update_fetch_github_handler),
        )
        .route("/api/update/poll", get(update_poll_handler))
        .route("/api/update/report", post(update_report_handler))
        .route("/api/update/status", get(update_status_handler))
        .route("/api/nodes/:node/containers", get(node_containers_handler))
        .route(
            "/api/nodes/:node/containers/:container/logs",
            get(node_container_logs_handler),
        )
        .route(
            "/api/nodes/:node/containers/:container/restart",
            post(node_container_restart_handler),
        )
        .route(
            "/api/nodes/:node/containers/:container/stop",
            post(node_container_stop_handler),
        )
        .route(
            "/api/nodes/:node/containers/:container/start",
            post(node_container_start_handler),
        )
        .route("/api/logs", get(logs_query_handler))
        .route("/api/logs/containers", get(logs_containers_handler))
        .route("/api/logs/config", get(logs_config_handler))
        .route("/api/logs/setup", post(logs_setup_handler))
        .route("/api/logs/agent-config", get(logs_agent_config_handler))
        .route(
            "/api/connections",
            get(connections_list_handler).post(connect_handler),
        )
        .route(
            "/api/connections/:id",
            axum::routing::delete(disconnect_handler),
        )
        .route(
            "/api/connections/:id/heartbeat",
            post(connection_heartbeat_handler),
        )
        .route("/api/ca-cert", get(ca_cert_handler))
        .nest_service(
            "/git",
            Router::new()
                .route("/*path", any(r4a_git_registry::handler::git_handler))
                .with_state(git_root),
        )
        .nest_service(
            "/v2",
            Router::new()
                .route("/", any(r4a_git_registry::registry::registry_root_handler))
                .route("/*path", any(r4a_git_registry::registry::registry_handler))
                .with_state(r4a_git_registry::RegistryState::new(
                    r4a_git_registry::default_registry_root(),
                    store.clone(),
                )),
        )
        // H-6: restrict CORS to VPN and localhost origins only
        .layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::predicate(|origin, _| {
                    // Parse the Origin and match the host exactly — prefix checks
                    // are bypassable (e.g. http://10.42.evil.com, http://localhost.evil.com)
                    let s = origin.to_str().unwrap_or("");
                    let rest = match s.split_once("://") {
                        Some(("http", rest)) | Some(("https", rest)) => rest,
                        _ => return false,
                    };
                    let host = rest.split(':').next().unwrap_or("");
                    matches!(
                        host,
                        "master.r4a.local"
                            | "web.master.r4a.local"
                            | "api.master.r4a.local"
                            | "localhost"
                            | "127.0.0.1"
                    ) || host
                        .parse::<std::net::Ipv4Addr>()
                        .is_ok_and(|ip| ip.octets()[0] == 10 && ip.octets()[1] == 42)
                }))
                .allow_methods(tower_http::cors::Any)
                .allow_headers(vec![
                    axum::http::header::AUTHORIZATION,
                    axum::http::header::CONTENT_TYPE,
                    axum::http::header::ACCEPT,
                    axum::http::header::ORIGIN,
                    axum::http::HeaderName::from_static("x-r4a-secret"),
                ]),
        )
        // C-2b: VPN-only middleware — all routes except / and /api/join require VPN src IP
        .layer(axum::middleware::from_fn(require_vpn_for_api))
        .with_state(state)
        .merge(r4a_store::store_router(store.clone()));

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", API_PORT)).await?;
    info!(
        "API listening on 0.0.0.0:{} (non-VPN IPs restricted to /api/join)",
        API_PORT
    );

    let pingora_store = store.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));

        let mut my_server =
            pingora::server::Server::new(None).expect("Failed to create Pingora server");
        my_server.bootstrap();

        let mut proxy = pingora::proxy::http_proxy_service(
            &my_server.configuration,
            r4a_ingress::IngressProxy {
                store: pingora_store,
            },
        );

        let addr = format!("0.0.0.0:{}", INGRESS_PORT);
        proxy.add_tcp(&addr);

        my_server.add_service(proxy);
        my_server.run_forever();
    });

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

/// C-2b: Reject requests to sensitive endpoints from public internet IPs.
/// Only "/" and "/api/join" are reachable from anywhere.
/// All RFC-1918 private ranges are allowed (VPN, docker bridge, LAN).
async fn require_vpn_for_api(
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<SocketAddr>,
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let path = req.uri().path();
    let is_public = path == "/"
        || path == "/api/join"
        || (path == "/api/connections" && req.method() == axum::http::Method::POST)
        || path == "/api/ca-cert";
    if !is_public {
        let allowed = match addr.ip() {
            std::net::IpAddr::V4(v4) => {
                let o = v4.octets();
                v4.is_loopback()                                        // 127.x.x.x
                    || o[0] == 10                                       // 10.x.x.x  (VPN, private)
                    || (o[0] == 172 && o[1] >= 16 && o[1] <= 31)       // 172.16-31 (docker bridge)
                    || (o[0] == 192 && o[1] == 168) // 192.168.x (LAN)
            }
            std::net::IpAddr::V6(v6) => v6.is_loopback(),
        };
        if !allowed {
            return (
                StatusCode::FORBIDDEN,
                "Access restricted to private network",
            )
                .into_response();
        }
    }
    next.run(req).await
}

async fn index_handler() -> (StatusCode, &'static str) {
    (StatusCode::OK, "r4a master node OK\n")
}

fn generate_token() -> String {
    let mut b = [0u8; 32];
    thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

async fn join_handler(
    State(state): State<AppState>,
    _auth: RequireSecret,
    Json(req): Json<JoinRequest>,
) -> Result<Json<JoinResponse>, (StatusCode, String)> {
    // C-1: validate all fields that end up in the WireGuard config file before
    // any processing — prevents newline injection into wg0.conf directives.
    r4a_vpn::wireguard::validate_wg_pubkey(&req.pub_key)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    if let Some(ref ep) = req.public_endpoint {
        r4a_vpn::wireguard::validate_endpoint(ep)
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    }
    if let Some(ref name) = req.name {
        r4a_vpn::wireguard::validate_node_name(name)
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    }

    let mut peers = state.peers.lock().unwrap();

    // H-2: master role is only granted when explicitly enabled via env var
    let allow_master_join = std::env::var("R4A_ALLOW_MASTER_JOIN")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let requested_role = req.role.as_deref().unwrap_or("agent");
    let role_str = if requested_role == "master" && allow_master_join {
        "master".to_string()
    } else {
        "agent".to_string()
    };

    let agent_ip = if let Some(existing) = peers.get(&req.pub_key) {
        existing.ip.clone()
    } else {
        // H-3: bounds check to prevent IP allocation overflow
        let mut next = state.next_ip.lock().unwrap();
        if *next > 254 {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "VPN IP pool exhausted (max 254 peers)".to_string(),
            ));
        }
        let ip = format!("10.42.0.{}", *next);
        *next += 1;
        ip
    };

    let name = req.name.clone().unwrap_or_else(|| {
        format!(
            "{}-{}",
            role_str,
            &agent_ip[agent_ip.rfind('.').unwrap_or(0) + 1..]
        )
    });

    let mut to_remove = vec![];
    for (pk, p) in peers.iter() {
        if p.name == name && pk != &req.pub_key {
            to_remove.push(pk.clone());
        }
    }
    for pk in to_remove {
        info!(
            "Removing old peer entry for name '{}' with old pub_key",
            name
        );
        peers.remove(&pk);
    }

    if let Ok(tree) = state.store.db.open_tree("tokens") {
        let mut to_delete = vec![];
        for item in tree.iter() {
            if let Ok((k, v)) = item {
                if let Ok(token) = serde_json::from_slice::<Token>(&v) {
                    if token.username == name {
                        to_delete.push(k);
                    }
                }
            }
        }
        for k in to_delete {
            info!("Removing old token for agent '{}'", name);
            let _ = tree.remove(k);
        }
    }

    let mut wg_cmd = std::process::Command::new("wg");
    wg_cmd.args([
        "set",
        "wg0",
        "peer",
        &req.pub_key,
        "allowed-ips",
        &format!("{agent_ip}/32"),
        "persistent-keepalive",
        "25",
    ]);
    let _ = wg_cmd.status();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut existing_token_id = None;
    if let Ok(tree) = state.store.db.open_tree("tokens") {
        for item in tree.iter() {
            if let Ok((_, v)) = item {
                if let Ok(token) = serde_json::from_slice::<Token>(&v) {
                    if token.username == name {
                        existing_token_id = Some(token.id);
                        break;
                    }
                }
            }
        }
    }

    peers.retain(|k, v| v.name != name || k == &req.pub_key);

    peers.insert(
        req.pub_key.clone(),
        PeerInfo {
            pub_key: req.pub_key.clone(),
            ip: agent_ip.clone(),
            name: name.clone(),
            role: role_str.clone(),
            public_endpoint: req.public_endpoint.clone(),
            observed_endpoint: None,
            p2p_direct: None,
            cpu_percent: None,
            ram_used_mb: None,
            ram_total_mb: None,
            vram_used_mb: None,
            vram_total_mb: None,
            last_seen: Some(now),
        },
    );

    let cloned_peers = peers.clone();
    let cloned_store = state.store.clone();

    let token_str = existing_token_id.unwrap_or_else(generate_token);
    let policy_id = format!("policy-{}", token_str);
    let binding_id = format!("binding-{}", token_str);

    let policy = if role_str == "master" {
        Policy {
            id: policy_id.clone(),
            rules: vec![Rule {
                verbs: vec![Verb::All],
                resources: vec![Resource::All],
                resource_names: None,
            }],
        }
    } else {
        Policy {
            id: policy_id.clone(),
            rules: vec![Rule {
                verbs: vec![Verb::Get],
                resources: vec![Resource::Vault, Resource::Manifests],
                resource_names: None,
            }],
        }
    };

    let binding = Binding {
        id: binding_id,
        subject: name.clone(),
        policy_id: policy_id.clone(),
    };

    let token = Token {
        id: token_str.clone(),
        username: name.clone(),
        created_at: now,
    };

    let cloned_token = token.clone();
    let cloned_policy = policy;
    let cloned_binding = binding;
    tokio::spawn(async move {
        let _ = cloned_store.put_policy(cloned_policy).await;
        let _ = cloned_store.put_binding(cloned_binding).await;
        let _ = cloned_store.put_token(cloned_token).await;
        save_peers(&cloned_store, &cloned_peers).await;
    });

    Ok(Json(JoinResponse {
        master_pub_key: state.master_pub_key.clone(),
        agent_vpn_ip: agent_ip,
        master_endpoint: public_endpoint(),
        peers: peers.clone(),
        agent_token: Some(token_str),
    }))
}

/// Список peer'ов с endpoint'ами для P2P-координации. Агенты опрашивают
/// периодически (cluster secret); VPN-only middleware закрывает от интернета.
async fn peers_handler(
    State(state): State<AppState>,
    _auth: RequireSecret,
) -> Json<HashMap<String, PeerInfo>> {
    Json(state.peers.lock().unwrap().clone())
}

#[derive(Deserialize)]
struct VaultSetRequest {
    config_id: String,
    key: String,
    value: String,
}

async fn vault_set_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Json(req): Json<VaultSetRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !state
        .store
        .can(&auth.token.username, Verb::Create, Resource::Vault, None)
    {
        return Err((
            StatusCode::FORBIDDEN,
            "Only admins can set secrets".to_string(),
        ));
    }

    let dek = {
        let keys = state.store.vault_keys.read().unwrap();
        *keys.get(&req.config_id).ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("Vault config {} not found", req.config_id),
            )
        })?
    };

    let (encrypted_value, nonce) = r4a_core::crypto::encrypt(&dek, req.value.as_bytes())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let secret = VaultSecret {
        config_id: req.config_id.clone(),
        key: req.key.clone(),
        encrypted_value,
        nonce,
        updated_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };

    state
        .store
        .put_vault_secret(secret)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!(
        "Vault secret set: {}/{} by {}",
        req.config_id, req.key, auth.token.username
    );
    Ok(StatusCode::OK)
}

async fn vault_delete_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Query(query): Query<HashMap<String, String>>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !state
        .store
        .can(&auth.token.username, Verb::Delete, Resource::Vault, None)
    {
        return Err((
            StatusCode::FORBIDDEN,
            "Only admins can delete secrets".to_string(),
        ));
    }

    let config_id = query
        .get("config_id")
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing config_id".to_string()))?;
    let key = query
        .get("key")
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing key".to_string()))?;

    state
        .store
        .delete_vault_secret(config_id, key)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!(
        "Vault secret deleted: {}/{} by {}",
        config_id, key, auth.token.username
    );
    Ok(StatusCode::OK)
}

async fn vault_get_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<String>, (StatusCode, String)> {
    let config_id = query.get("config_id").ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "Missing config_id parameter".to_string(),
        )
    })?;
    let key = query
        .get("key")
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing key parameter".to_string()))?;
    let full_key = format!("{}/{}", config_id, key);

    if !state.store.can(
        &auth.token.username,
        Verb::Get,
        Resource::Vault,
        Some(&full_key),
    ) {
        error!(
            "Vault access denied: {} for {}",
            full_key, auth.token.username
        );
        return Err((
            StatusCode::FORBIDDEN,
            "Access denied by RBAC policy".to_string(),
        ));
    }

    let secret = state
        .store
        .get_vault_secret(config_id, key)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Secret not found".to_string()))?;

    let decrypted = state
        .store
        .decrypt_secret(&secret)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!(
        "Vault secret accessed: {}/{} by {}",
        config_id, key, auth.token.username
    );
    Ok(Json(decrypted))
}

async fn vault_list_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Vec<String>>, (StatusCode, String)> {
    if !state
        .store
        .can(&auth.token.username, Verb::List, Resource::Vault, None)
    {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    let config_id = query
        .get("config_id")
        .map(|s| s.as_str())
        .unwrap_or("default");

    let keys = state
        .store
        .list_vault_secrets(config_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(keys))
}

#[derive(Deserialize)]
struct VaultConfigCreateRequest {
    name: String,
}

async fn vault_configs_list_handler(State(state): State<AppState>, auth: RequireToken) -> Response {
    if !state
        .store
        .can(&auth.token.username, Verb::List, Resource::Vault, None)
    {
        return (StatusCode::FORBIDDEN, "Access denied".to_string()).into_response();
    }

    match state.store.get_vault_configs() {
        Ok(configs) => Json(configs).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[axum::debug_handler]
async fn vault_config_create_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Json(req): Json<VaultConfigCreateRequest>,
) -> Result<Json<VaultConfig>, (StatusCode, String)> {
    if !state
        .store
        .can(&auth.token.username, Verb::Create, Resource::Vault, None)
    {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    match state.store.create_vault_config(req.name).await {
        Ok(config) => Ok(Json(config)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

async fn tokens_list_handler(
    State(state): State<AppState>,
    auth: RequireToken,
) -> Result<Json<Vec<Token>>, (StatusCode, String)> {
    if !state
        .store
        .can(&auth.token.username, Verb::List, Resource::Tokens, None)
    {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    let mut tokens = vec![];
    let tree = state
        .store
        .db
        .open_tree("tokens")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    for item in tree.iter() {
        if let Ok((_, v)) = item {
            if let Ok(token) = serde_json::from_slice::<Token>(&v) {
                tokens.push(token);
            }
        }
    }
    Ok(Json(tokens))
}

#[derive(Deserialize)]
struct TokenCreateRequest {
    username: String,
    verbs: Vec<Verb>,
    resources: Vec<Resource>,
    #[serde(default)]
    resource_names: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct UserCreateRequest {
    username: String,
    password: String,
    verbs: Vec<Verb>,
    resources: Vec<Resource>,
    #[serde(default)]
    resource_names: Option<Vec<String>>,
}

#[derive(Serialize)]
struct UserInfo {
    username: String,
}

async fn token_create_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Json(req): Json<TokenCreateRequest>,
) -> Result<Json<Token>, (StatusCode, String)> {
    if !state
        .store
        .can(&auth.token.username, Verb::Create, Resource::Tokens, None)
    {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    let token_str = generate_token();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let policy_id = format!("policy-{}", token_str);
    let binding_id = format!("binding-{}", token_str);

    let policy = Policy {
        id: policy_id.clone(),
        rules: vec![Rule {
            verbs: req.verbs,
            resources: req.resources,
            resource_names: req.resource_names,
        }],
    };

    state
        .store
        .put_policy(policy)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let binding = Binding {
        id: binding_id,
        subject: req.username.clone(),
        policy_id,
    };
    state
        .store
        .put_binding(binding)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let token = Token {
        id: token_str,
        username: req.username,
        created_at: now,
    };

    state
        .store
        .put_token(token.clone())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(token))
}

async fn token_delete_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Query(query): Query<HashMap<String, String>>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !state
        .store
        .can(&auth.token.username, Verb::Delete, Resource::Tokens, None)
    {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }
    let id = query
        .get("id")
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing id".to_string()))?;

    state
        .store
        .delete("tokens", id.as_bytes())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::OK)
}

async fn users_list_handler(
    State(state): State<AppState>,
    auth: RequireToken,
) -> Result<Json<Vec<UserInfo>>, (StatusCode, String)> {
    if !state
        .store
        .can(&auth.token.username, Verb::List, Resource::Tokens, None)
    {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    let mut users = vec![];
    let tree = state
        .store
        .db
        .open_tree("users")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    for item in tree.iter() {
        if let Ok((_, v)) = item {
            if let Ok(user) = serde_json::from_slice::<User>(&v) {
                users.push(UserInfo {
                    username: user.username,
                });
            }
        }
    }
    Ok(Json(users))
}

async fn user_create_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Json(req): Json<UserCreateRequest>,
) -> Result<Json<UserInfo>, (StatusCode, String)> {
    if !state
        .store
        .can(&auth.token.username, Verb::Create, Resource::Tokens, None)
    {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    let username = req.username.trim().to_string();
    if username.is_empty()
        || username.len() > 64
        || !username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid username".to_string()));
    }
    if req.password.len() < 8 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Password must be at least 8 characters".to_string(),
        ));
    }

    let users_tree = state
        .store
        .db
        .open_tree("users")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if users_tree
        .get(username.as_bytes())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .is_some()
    {
        return Err((StatusCode::CONFLICT, "User already exists".to_string()));
    }

    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    let password_hash = Argon2::default()
        .hash_password(req.password.as_bytes(), &salt)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .to_string();

    let policy_id = format!("policy-user-{}", username);
    let binding_id = format!("binding-user-{}", username);
    let policy = Policy {
        id: policy_id.clone(),
        rules: vec![Rule {
            verbs: req.verbs,
            resources: req.resources,
            resource_names: req.resource_names,
        }],
    };
    state
        .store
        .put_policy(policy)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let binding = Binding {
        id: binding_id,
        subject: username.clone(),
        policy_id,
    };
    state
        .store
        .put_binding(binding)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let user = User {
        username: username.clone(),
        password_hash,
    };
    users_tree
        .insert(
            username.as_bytes(),
            serde_json::to_vec(&user)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
        )
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    state
        .store
        .db
        .flush_async()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(UserInfo { username }))
}

async fn user_delete_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Query(query): Query<HashMap<String, String>>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !state
        .store
        .can(&auth.token.username, Verb::Delete, Resource::Tokens, None)
    {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    let username = query
        .get("username")
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing username".to_string()))?
        .trim()
        .to_string();
    if username.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Missing username".to_string()));
    }

    let users_tree = state
        .store
        .db
        .open_tree("users")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if users_tree
        .remove(username.as_bytes())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .is_none()
    {
        return Err((StatusCode::NOT_FOUND, "User not found".to_string()));
    }

    let policy_id = format!("policy-user-{}", username);
    let binding_id = format!("binding-user-{}", username);
    state
        .store
        .delete("policies", policy_id.as_bytes())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    state
        .store
        .delete("bindings", binding_id.as_bytes())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    state
        .store
        .db
        .flush_async()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::OK)
}

async fn metrics_handler(
    State(state): State<AppState>,
    _auth: RequireSecret,
    Json(report): Json<MetricsReport>,
) -> StatusCode {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let node_name = {
        let mut peers = state.peers.lock().unwrap();
        match peers.values_mut().find(|p| p.ip == report.agent_vpn_ip) {
            Some(peer) => {
                peer.cpu_percent = Some(report.cpu_percent);
                peer.ram_used_mb = Some(report.ram_used_mb);
                peer.ram_total_mb = Some(report.ram_total_mb);
                peer.vram_used_mb = report.vram_used_mb;
                peer.vram_total_mb = report.vram_total_mb;
                peer.last_seen = Some(now);
                if report.p2p_direct.is_some() {
                    peer.p2p_direct = report.p2p_direct.clone();
                }
                Some(peer.name.clone())
            }
            None => None,
        }
    };

    // История метрик в telemetry-store (retention как у логов)
    if let Some(name) = node_name {
        let _ = state.log_store.append_metric(&r4a_telemetry::MetricPoint {
            node: name,
            ts_ms: now * 1000,
            cpu_percent: report.cpu_percent,
            ram_used_mb: report.ram_used_mb,
            ram_total_mb: report.ram_total_mb,
            vram_used_mb: report.vram_used_mb,
            vram_total_mb: report.vram_total_mb,
        });
    }
    StatusCode::OK
}

#[derive(Deserialize)]
struct MetricsHistoryParams {
    node: String,
    tail: Option<usize>,
}

async fn metrics_history_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Query(q): Query<MetricsHistoryParams>,
) -> Result<Json<Vec<r4a_telemetry::MetricPoint>>, (StatusCode, String)> {
    if !state
        .store
        .can(&auth.token.username, Verb::Get, Resource::Nodes, None)
    {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }
    let tail = q.tail.unwrap_or(720).min(10_000);
    let points = state
        .log_store
        .query_metrics(&q.node, tail)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(points))
}

async fn nodes_handler(
    State(state): State<AppState>,
    auth: RequireToken,
) -> Result<Json<Vec<NodeInfo>>, (StatusCode, String)> {
    if !state.store.can(
        &auth.token.username,
        r4a_core::models::Verb::List,
        r4a_core::models::Resource::Nodes,
        None,
    ) {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }
    let mut sys = System::new_all();
    sys.refresh_all();
    let master_name = state.my_node_name.clone();
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
        p2p_direct: None,
    }];

    for peer in state.peers.lock().unwrap().values() {
        if peer.ip == state.my_vpn_ip {
            continue;
        }
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
            p2p_direct: peer.p2p_direct.clone(),
        });
    }

    Ok(Json(nodes))
}

#[derive(Deserialize)]
struct ManifestsQuery {
    node: Option<String>,
}

async fn manifests_handler(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<ManifestsQuery>,
) -> Result<Json<HashMap<String, Manifest>>, (StatusCode, String)> {
    match &auth {
        Auth::Token(token) => {
            if !state
                .store
                .can(&token.username, Verb::Get, Resource::Manifests, None)
            {
                return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
            }
        }
        Auth::Secret => {}
    }

    let manifests = state
        .store
        .list_manifests()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let result: HashMap<String, Manifest> = if let Some(node_name) = query.node {
        manifests
            .into_iter()
            .filter(|m| m.app.node_selector == node_name || m.app.node_selector == "all")
            .map(|m| (m.app.name.clone(), m))
            .collect()
    } else {
        manifests
            .into_iter()
            .map(|m| (m.app.name.clone(), m))
            .collect()
    };

    Ok(Json(result))
}

async fn manifest_upsert_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Json(manifest): Json<Manifest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !state.store.can(
        &auth.token.username,
        Verb::Create,
        Resource::Manifests,
        None,
    ) && !state.store.can(
        &auth.token.username,
        Verb::Update,
        Resource::Manifests,
        None,
    ) {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    let name = manifest.app.name.trim().to_string();
    if name.is_empty() || name.contains('/') || name.contains("..") {
        return Err((StatusCode::BAD_REQUEST, "Invalid manifest name".to_string()));
    }

    state
        .store
        .put_manifest(&manifest)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!("Manifest '{}' upserted by {}", name, auth.token.username);
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct ManifestDeleteQuery {
    name: String,
}

async fn manifest_delete_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Query(query): Query<ManifestDeleteQuery>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !state.store.can(
        &auth.token.username,
        Verb::Delete,
        Resource::Manifests,
        None,
    ) {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    state
        .store
        .delete_manifest(&query.name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if query.name == "clickhouse" {
        state
            .store
            .delete("core", LOGS_CH_CONFIG_KEY)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        info!(
            "Logs ClickHouse config cleared after manifest deletion by {}",
            auth.token.username
        );
    }

    info!(
        "Manifest '{}' deleted by {}",
        query.name, auth.token.username
    );
    Ok(StatusCode::OK)
}

fn query_vram() -> (Option<u64>, Option<u64>) {
    let out = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=memory.used,memory.total",
            "--format=csv,noheader,nounits",
        ])
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

#[derive(Serialize)]
struct RegistryRepoInfo {
    name: String,
    tag_count: usize,
    total_size: u64,
}

#[derive(Serialize)]
struct RegistryTagInfo {
    tag: String,
    digest: String,
    size: u64,
    pushed_at: u64,
}

#[derive(Deserialize)]
struct RegistryManifestRecord {
    size: u64,
    created_at: u64,
    body: Vec<u8>,
}

#[derive(Deserialize)]
struct RegistryManifestBody {
    config: Option<RegistryDescriptor>,
    #[serde(default)]
    layers: Vec<RegistryDescriptor>,
}

#[derive(Deserialize)]
struct RegistryDescriptor {
    size: u64,
}

async fn git_repos_handler(
    State(state): State<AppState>,
    auth: RequireToken,
) -> Result<Json<Vec<RepoInfo>>, (StatusCode, String)> {
    if !state.store.can(
        &auth.token.username,
        r4a_core::models::Verb::List,
        r4a_core::models::Resource::GitRepos,
        None,
    ) {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }
    let git_root = r4a_git_registry::default_git_root();
    let mut repos = vec![];
    if let Ok(entries) = std::fs::read_dir(&git_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("HEAD").exists() {
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let clone_url = format!("http://{}:{}/git/{}", state.my_vpn_ip, API_PORT, name);
                repos.push(RepoInfo { name, clone_url });
            }
        }
    }
    repos.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(repos))
}

#[derive(Deserialize)]
struct CreateRepoRequest {
    name: String,
}

async fn git_create_repo_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Json(req): Json<CreateRepoRequest>,
) -> Result<Json<RepoInfo>, (StatusCode, String)> {
    if !state.store.can(
        &auth.token.username,
        r4a_core::models::Verb::Create,
        r4a_core::models::Resource::GitRepos,
        None,
    ) {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }
    let name = req.name.trim().to_string();
    if name.is_empty() || name.contains('/') || name.contains("..") {
        return Err((StatusCode::BAD_REQUEST, "invalid repo name".to_string()));
    }
    let repo_name = if name.ends_with(".git") {
        name.clone()
    } else {
        format!("{}.git", name)
    };
    let path = r4a_git_registry::default_git_root().join(&repo_name);
    if path.exists() {
        return Err((
            StatusCode::CONFLICT,
            format!("repository '{}' already exists", repo_name),
        ));
    }
    r4a_git_registry::init_repo(&path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let clone_url = format!("http://{}:{}/git/{}", state.my_vpn_ip, API_PORT, repo_name);
    Ok(Json(RepoInfo {
        name: repo_name,
        clone_url,
    }))
}

async fn registry_repos_handler(
    State(state): State<AppState>,
    auth: RequireToken,
) -> Result<Json<Vec<RegistryRepoInfo>>, (StatusCode, String)> {
    if !state
        .store
        .can(&auth.token.username, Verb::List, Resource::Registry, None)
    {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    let tree = state
        .store
        .db
        .open_tree("registry_meta")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut repo_tags: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for item in tree.iter() {
        let (key, value) = item.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let key = String::from_utf8_lossy(&key);
        let Some((repo, tag)) = parse_registry_tag_key(&key) else {
            continue;
        };
        let digest = String::from_utf8(value.to_vec())
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        repo_tags
            .entry(repo.to_string())
            .or_default()
            .push((tag.to_string(), digest));
    }

    let mut repos = Vec::new();
    for (repo, tags) in repo_tags {
        let mut seen = std::collections::HashSet::new();
        let mut total_size = 0u64;
        for (_, digest) in &tags {
            if !seen.insert(digest.clone()) {
                continue;
            }
            if let Some(record) = load_registry_manifest_record(&state.store, &repo, digest)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            {
                total_size = total_size.saturating_add(registry_manifest_total_size(&record));
            }
        }
        repos.push(RegistryRepoInfo {
            name: repo,
            tag_count: tags.len(),
            total_size,
        });
    }

    repos.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(repos))
}

async fn registry_repo_tags_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Path(rest): Path<String>,
) -> Result<Json<Vec<RegistryTagInfo>>, (StatusCode, String)> {
    if !state
        .store
        .can(&auth.token.username, Verb::List, Resource::Registry, None)
    {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    let repo =
        parse_registry_tags_route(&rest).ok_or((StatusCode::NOT_FOUND, "Not found".to_string()))?;

    let tree = state
        .store
        .db
        .open_tree("registry_meta")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let prefix = format!("repo:{repo}\0tag:");

    let mut tags = Vec::new();
    for item in tree.scan_prefix(prefix.as_bytes()) {
        let (key, value) = item.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let key = String::from_utf8_lossy(&key);
        let Some((_, tag)) = parse_registry_tag_key(&key) else {
            continue;
        };
        let digest = String::from_utf8(value.to_vec())
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let record = load_registry_manifest_record(&state.store, repo, &digest)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Missing manifest for digest {digest}"),
            ))?;
        tags.push(RegistryTagInfo {
            tag: tag.to_string(),
            digest,
            size: registry_manifest_total_size(&record),
            pushed_at: record.created_at,
        });
    }

    tags.sort_by(|a, b| a.tag.cmp(&b.tag));
    Ok(Json(tags))
}

async fn registry_repo_tag_delete_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Path(rest): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !state
        .store
        .can(&auth.token.username, Verb::Delete, Resource::Registry, None)
    {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    let (repo, tag) = parse_registry_delete_route(&rest)
        .ok_or((StatusCode::NOT_FOUND, "Not found".to_string()))?;

    let tree = state
        .store
        .db
        .open_tree("registry_meta")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let tag_key = format!("repo:{repo}\0tag:{tag}");
    let Some(raw_digest) = tree
        .get(tag_key.as_bytes())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    else {
        return Err((StatusCode::NOT_FOUND, "Tag not found".to_string()));
    };
    let digest = String::from_utf8(raw_digest.to_vec())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tree.remove(tag_key.as_bytes())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let prefix = format!("repo:{repo}\0tag:");
    let digest_still_referenced = tree.scan_prefix(prefix.as_bytes()).any(|item| match item {
        Ok((_, value)) => value.as_ref() == digest.as_bytes(),
        Err(_) => false,
    });
    if !digest_still_referenced {
        let manifest_key = format!("repo:{repo}\0manifest:{digest}");
        tree.remove(manifest_key.as_bytes())
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    state
        .store
        .db
        .flush_async()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!(
        "Registry tag '{}:{}' deleted by {}",
        repo, tag, auth.token.username
    );
    Ok(StatusCode::OK)
}

fn parse_registry_tag_key(key: &str) -> Option<(&str, &str)> {
    let rest = key.strip_prefix("repo:")?;
    let (repo, tag) = rest.split_once("\0tag:")?;
    Some((repo, tag))
}

fn parse_registry_tags_route(rest: &str) -> Option<&str> {
    let trimmed = rest.trim_matches('/');
    trimmed
        .strip_suffix("/tags")
        .filter(|repo| !repo.is_empty())
}

fn parse_registry_delete_route(rest: &str) -> Option<(&str, &str)> {
    let trimmed = rest.trim_matches('/');
    let (repo, tag) = trimmed.rsplit_once("/tags/")?;
    if repo.is_empty() || tag.is_empty() {
        return None;
    }
    Some((repo, tag))
}

fn load_registry_manifest_record(
    store: &Store,
    repo: &str,
    digest: &str,
) -> anyhow::Result<Option<RegistryManifestRecord>> {
    let key = format!("repo:{repo}\0manifest:{digest}");
    let tree = store.db.open_tree("registry_meta")?;
    let Some(raw) = tree.get(key.as_bytes())? else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_slice(&raw)?))
}

fn registry_manifest_total_size(record: &RegistryManifestRecord) -> u64 {
    match serde_json::from_slice::<RegistryManifestBody>(&record.body) {
        Ok(manifest) => manifest
            .config
            .map(|cfg| cfg.size)
            .unwrap_or(0)
            .saturating_add(
                manifest
                    .layers
                    .into_iter()
                    .map(|layer| layer.size)
                    .sum::<u64>(),
            ),
        Err(_) => record.size,
    }
}

fn sha256_file(path: &str) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    Some(format!("{:x}", hasher.finalize()))
}

// Binary and signature endpoints accept both Bearer token and X-R4A-Secret,
// because the auto-update agent only has the cluster secret (no token).
async fn agent_binary_handler(_auth: Auth) -> Response {
    match tokio::fs::read(AGENT_BINARY_PATH).await {
        Ok(data) => Response::builder()
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(data))
            .unwrap(),
        Err(e) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from(e.to_string()))
            .unwrap(),
    }
}

async fn server_binary_handler(_auth: Auth) -> Response {
    match tokio::fs::read(SERVER_BINARY_PATH).await {
        Ok(data) => Response::builder()
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(data))
            .unwrap(),
        Err(e) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from(e.to_string()))
            .unwrap(),
    }
}

async fn tui_binary_handler(_auth: Auth) -> Response {
    match tokio::fs::read(TUI_BINARY_PATH).await {
        Ok(data) => Response::builder()
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(data))
            .unwrap(),
        Err(e) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from(e.to_string()))
            .unwrap(),
    }
}

// C-2: signature endpoints — agents download and re-verify before applying updates.
async fn agent_binary_sig_handler(_auth: Auth) -> Response {
    match tokio::fs::read(AGENT_SIG_PATH).await {
        Ok(data) => Response::builder()
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(data))
            .unwrap(),
        Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("signature not found"))
            .unwrap(),
    }
}

async fn server_binary_sig_handler(_auth: Auth) -> Response {
    match tokio::fs::read(SERVER_SIG_PATH).await {
        Ok(data) => Response::builder()
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(data))
            .unwrap(),
        Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("signature not found"))
            .unwrap(),
    }
}

async fn tui_binary_sig_handler(_auth: Auth) -> Response {
    match tokio::fs::read(TUI_SIG_PATH).await {
        Ok(data) => Response::builder()
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(data))
            .unwrap(),
        Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("signature not found"))
            .unwrap(),
    }
}

async fn agent_checksum_handler(
    _auth: RequireToken,
) -> Result<Json<ChecksumResponse>, (StatusCode, String)> {
    match sha256_file(AGENT_BINARY_PATH) {
        Some(checksum) => Ok(Json(ChecksumResponse { checksum })),
        None => Err((StatusCode::NOT_FOUND, "binary not found".to_string())),
    }
}

async fn server_checksum_handler(
    _auth: RequireToken,
) -> Result<Json<ChecksumResponse>, (StatusCode, String)> {
    match sha256_file(SERVER_BINARY_PATH) {
        Some(checksum) => Ok(Json(ChecksumResponse { checksum })),
        None => Err((StatusCode::NOT_FOUND, "binary not found".to_string())),
    }
}

async fn tui_checksum_handler(
    _auth: RequireToken,
) -> Result<Json<ChecksumResponse>, (StatusCode, String)> {
    match sha256_file(TUI_BINARY_PATH) {
        Some(checksum) => Ok(Json(ChecksumResponse { checksum })),
        None => Err((StatusCode::NOT_FOUND, "binary not found".to_string())),
    }
}

#[derive(Serialize)]
struct ChecksumResponse {
    checksum: String,
}

async fn token_exchange_handler(
    State(state): State<AppState>,
    _auth: RequireAdminSecret,
) -> Result<Json<Token>, (StatusCode, String)> {
    {
        let tree = state
            .store
            .db
            .open_tree("tokens")
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        for item in tree.iter() {
            if let Ok((_, v)) = item {
                if let Ok(token) = serde_json::from_slice::<Token>(&v) {
                    if token.username == "admin"
                        && state
                            .store
                            .can(&token.username, Verb::All, Resource::All, None)
                    {
                        return Ok(Json(token));
                    }
                }
            }
        }
    }

    let token_str = generate_token();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let policy_id = format!("policy-{}", token_str);
    let binding_id = format!("binding-{}", token_str);

    let policy = Policy {
        id: policy_id.clone(),
        rules: vec![Rule {
            verbs: vec![Verb::All],
            resources: vec![Resource::All],
            resource_names: None,
        }],
    };

    state
        .store
        .put_policy(policy)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let binding = Binding {
        id: binding_id,
        subject: "admin".to_string(),
        policy_id,
    };
    state
        .store
        .put_binding(binding)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let token = Token {
        id: token_str.clone(),
        username: "admin".to_string(),
        created_at: now,
    };

    state
        .store
        .put_token(token.clone())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(token))
}

#[derive(Serialize)]
struct TestResponse {
    ok: bool,
    checksum: Option<String>,
    message: String,
}

async fn update_test_handler(_auth: RequireToken) -> Json<TestResponse> {
    match sha256_file(AGENT_BINARY_PATH) {
        Some(checksum) => Json(TestResponse {
            ok: true,
            checksum: Some(checksum),
            message: "binary OK".to_string(),
        }),
        None => Json(TestResponse {
            ok: false,
            checksum: None,
            message: "not found".to_string(),
        }),
    }
}

async fn update_trigger_handler(State(state): State<AppState>, _auth: RequireToken) -> StatusCode {
    *state.update_pending.lock().unwrap() = true;
    StatusCode::OK
}

async fn server_update_server_trigger_handler(_auth: RequireToken) -> StatusCode {
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        info!("Server update triggered, exiting for restart...");
        std::process::exit(0);
    });
    StatusCode::OK
}

#[derive(Deserialize)]
struct GithubReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Serialize)]
struct FetchGithubResponse {
    success: bool,
    message: String,
    version: Option<String>,
}

async fn update_fetch_github_handler(_auth: RequireToken) -> Json<FetchGithubResponse> {
    match do_fetch_github().await {
        Ok(version) => Json(FetchGithubResponse {
            success: true,
            message: format!("Successfully downloaded version {}", version),
            version: Some(version),
        }),
        Err(e) => Json(FetchGithubResponse {
            success: false,
            message: format!("Failed to fetch from GitHub: {}", e),
            version: None,
        }),
    }
}

async fn do_fetch_github() -> Result<String> {
    let client = reqwest::Client::builder()
        .user_agent("r4a-cluster-manager")
        .build()?;

    let repo_url = "https://api.github.com/repos/rockxi/rust4eska/releases/latest";
    let release: GithubRelease = client
        .get(repo_url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let arch = std::env::consts::ARCH;
    let target = format!("{}-unknown-linux-musl", arch);
    let skip_verify = std::env::var("R4A_SKIP_SIGNATURE_VERIFY").as_deref() == Ok("1");

    for binary in ["r4a-server", "r4a-agent", "r4a-tui"] {
        let bin_pattern = format!("{}-{}", binary, target);
        let sig_pattern = format!("{}-{}.sig", binary, target);

        let binary_asset = release
            .assets
            .iter()
            .find(|a| a.name.contains(&bin_pattern) || a.name == *binary);

        let sig_asset = release
            .assets
            .iter()
            .find(|a| a.name.contains(&sig_pattern) || a.name == format!("{}.sig", binary));

        let binary_asset = match binary_asset {
            Some(a) => a,
            None => {
                warn!(
                    "Could not find asset for {} in release {}",
                    binary, release.tag_name
                );
                continue;
            }
        };

        // C-2: require signature unless explicitly bypassed for development
        let sig_bytes = match sig_asset {
            Some(sa) => {
                let b = client
                    .get(&sa.browser_download_url)
                    .send()
                    .await?
                    .error_for_status()?
                    .bytes()
                    .await?;
                Some(b.to_vec())
            }
            None if skip_verify => {
                warn!(
                    "SECURITY: no signature for {} in release {} (R4A_SKIP_SIGNATURE_VERIFY=1)",
                    binary, release.tag_name
                );
                None
            }
            None => {
                error!(
                    "No signature asset for {} in release {} — refusing to deploy unsigned binary",
                    binary, release.tag_name
                );
                continue;
            }
        };

        let bytes = client
            .get(&binary_asset.browser_download_url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;

        // C-2: verify Ed25519 signature before writing to disk
        if let Some(ref sig) = sig_bytes {
            verify_release_signature(&bytes, sig)
                .map_err(|e| anyhow::anyhow!("Refusing to deploy {}: {}", binary, e))?;
            info!("Signature verified for {}", binary);
        }

        let (dest, sig_dest) = match binary {
            "r4a-server" => (
                std::path::Path::new(SERVER_BINARY_PATH),
                std::path::Path::new(SERVER_SIG_PATH),
            ),
            "r4a-agent" => (
                std::path::Path::new(AGENT_BINARY_PATH),
                std::path::Path::new(AGENT_SIG_PATH),
            ),
            "r4a-tui" => (
                std::path::Path::new(TUI_BINARY_PATH),
                std::path::Path::new(TUI_SIG_PATH),
            ),
            _ => continue,
        };

        let tmp_path = dest.with_extension("tmp_download");
        tokio::fs::write(&tmp_path, &bytes).await?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755));
        }

        tokio::fs::rename(&tmp_path, dest).await?;

        // Store signature alongside binary so agents can re-verify it
        if let Some(sig) = sig_bytes {
            tokio::fs::write(sig_dest, &sig).await?;
        }

        info!(
            "Downloaded and replaced {} (release {})",
            binary, release.tag_name
        );
    }

    Ok(release.tag_name)
}

#[derive(Serialize)]
struct UpdatePollResponse {
    update_pending: bool,
    checksum: Option<String>,
}

async fn update_poll_handler(
    State(state): State<AppState>,
    _auth: RequireSecret,
) -> Json<UpdatePollResponse> {
    let update_pending = *state.update_pending.lock().unwrap();
    let checksum = if update_pending {
        sha256_file(AGENT_BINARY_PATH)
    } else {
        None
    };
    Json(UpdatePollResponse {
        update_pending,
        checksum,
    })
}

#[derive(Deserialize)]
struct UpdateReport {
    agent_vpn_ip: String,
    checksum: String,
    status: String,
}

async fn update_report_handler(
    State(state): State<AppState>,
    _auth: RequireSecret,
    Json(report): Json<UpdateReport>,
) -> StatusCode {
    let update_status = match report.status.as_str() {
        "updated" => AgentUpdateStatus::Updated,
        "updating" => AgentUpdateStatus::Updating,
        "failed" => AgentUpdateStatus::Failed("failed".to_string()),
        _ => AgentUpdateStatus::Unknown,
    };
    {
        let mut states = state.agent_update_states.lock().unwrap();
        states.insert(
            report.agent_vpn_ip,
            AgentUpdateState {
                status: update_status,
                checksum: Some(report.checksum),
            },
        );

        // Auto-reset update_pending once all known agents have the master checksum
        let master_checksum = sha256_file(AGENT_BINARY_PATH);
        if let Some(ref mc) = master_checksum {
            let peers = state.peers.lock().unwrap();
            let agent_ips: Vec<String> = peers
                .values()
                .filter(|p| p.role == "agent")
                .map(|p| p.ip.clone())
                .collect();
            drop(peers);

            let all_updated = !agent_ips.is_empty()
                && agent_ips.iter().all(|ip| match states.get(ip) {
                    Some(s) => {
                        matches!(s.status, AgentUpdateStatus::Updated)
                            && s.checksum.as_deref() == Some(mc.as_str())
                    }
                    None => false,
                });

            if all_updated {
                *state.update_pending.lock().unwrap() = false;
                info!(
                    "All agents updated to {}, clearing update_pending",
                    &mc[..8]
                );
            }
        }
    }
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

async fn update_status_handler(
    State(state): State<AppState>,
    _auth: RequireToken,
) -> Json<UpdateStatusResponse> {
    let master_checksum = sha256_file(AGENT_BINARY_PATH);
    let update_pending = *state.update_pending.lock().unwrap();
    let states = state.agent_update_states.lock().unwrap();
    let peers = state.peers.lock().unwrap();

    // Start with all connected agent peers so they appear even before reporting
    let mut agents: HashMap<String, AgentUpdateStateDto> = peers
        .values()
        .filter(|p| p.role == "agent")
        .map(|p| {
            (
                p.ip.clone(),
                AgentUpdateStateDto {
                    status: "idle".to_string(),
                    checksum: None,
                },
            )
        })
        .collect();

    // Overlay with any reported update states
    for (ip, s) in states.iter() {
        let status_str = match &s.status {
            AgentUpdateStatus::Updated => "updated",
            AgentUpdateStatus::Updating => "updating",
            AgentUpdateStatus::Failed(_) => "failed",
            // If checksum matches master, agent is effectively up-to-date
            _ if s.checksum.as_deref() == master_checksum.as_deref() => "updated",
            _ => "idle",
        }
        .to_string();
        agents.insert(
            ip.clone(),
            AgentUpdateStateDto {
                status: status_str,
                checksum: s.checksum.clone(),
            },
        );
    }

    Json(UpdateStatusResponse {
        master_checksum,
        update_pending,
        agents,
    })
}

const AGENT_API_PORT: u16 = 8082;

#[derive(Deserialize)]
struct NodePath {
    node: String,
}

#[derive(Deserialize)]
struct NodeContainerPath {
    node: String,
    container: String,
}

#[derive(Deserialize)]
struct ContainerLogsQuery {
    tail: Option<u64>,
}

fn find_peer_ip(state: &AppState, node_name: &str) -> Option<String> {
    let peers = state.peers.lock().unwrap();
    peers
        .values()
        .find(|p| p.name == node_name)
        .map(|p| p.ip.clone())
}

fn is_current_master_node(state: &AppState, node_name: &str) -> bool {
    node_name == state.my_node_name || node_name == state.my_vpn_ip
}

#[derive(Serialize)]
struct ContainerInfo {
    id: String,
    name: String,
    image: String,
    status: String,
    state: String,
}

async fn master_docker() -> Result<Docker, (StatusCode, String)> {
    Docker::connect_with_local_defaults()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn master_containers(
    state: &AppState,
) -> Result<Json<Vec<ContainerInfo>>, (StatusCode, String)> {
    let docker = master_docker().await?;
    let mut filters = HashMap::new();
    filters.insert(
        "label".to_string(),
        vec![format!("r4a.node={}", state.my_node_name)],
    );
    let opts = ListContainersOptions {
        all: true,
        filters,
        ..Default::default()
    };
    let containers = docker
        .list_containers(Some(opts))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let result = containers
        .into_iter()
        .map(|c| {
            let name = c
                .names
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
        })
        .collect();
    let _ = state; // keep signature parallel to future auth/audit needs
    Ok(Json(result))
}

async fn master_container_logs(
    container: &str,
    tail: u64,
) -> Result<Json<Vec<String>>, (StatusCode, String)> {
    let docker = master_docker().await?;
    let opts = LogsOptions::<String> {
        stdout: true,
        stderr: true,
        tail: tail.to_string(),
        timestamps: false,
        ..Default::default()
    };
    let mut stream = docker.logs(container, Some(opts));
    let mut lines = Vec::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(output) => lines.push(output.to_string()),
            Err(e) => lines.push(format!("[error] {}", e)),
        }
    }
    Ok(Json(lines))
}

async fn master_container_restart(container: &str) -> Result<StatusCode, (StatusCode, String)> {
    let docker = master_docker().await?;
    docker
        .restart_container(container, Some(RestartContainerOptions { t: 5 }))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::OK)
}

async fn master_container_stop(container: &str) -> Result<StatusCode, (StatusCode, String)> {
    let docker = master_docker().await?;
    docker
        .stop_container(container, Some(StopContainerOptions { t: 5 }))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::OK)
}

async fn master_container_start(container: &str) -> Result<StatusCode, (StatusCode, String)> {
    let docker = master_docker().await?;
    docker
        .start_container(container, None::<StartContainerOptions<String>>)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::OK)
}

async fn proxy_get(url: &str, secret: &str) -> Result<reqwest::Response, (StatusCode, String)> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();
    client
        .get(url)
        .header("X-R4A-Secret", secret)
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))
}

async fn proxy_post(url: &str, secret: &str) -> Result<reqwest::Response, (StatusCode, String)> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default();
    client
        .post(url)
        .header("X-R4A-Secret", secret)
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))
}

async fn node_containers_handler(
    State(state): State<AppState>,
    _auth: RequireToken,
    Path(NodePath { node }): Path<NodePath>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if is_current_master_node(&state, &node) {
        return master_containers(&state)
            .await
            .map(IntoResponse::into_response);
    }
    let ip = find_peer_ip(&state, &node)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Node '{}' not found", node)))?;
    let url = format!("http://{}:{}/containers", ip, AGENT_API_PORT);
    let resp = proxy_get(&url, &state.cluster_secret).await?;
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let body = resp
        .bytes()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok((status, axum::body::Body::from(body)).into_response())
}

async fn node_container_logs_handler(
    State(state): State<AppState>,
    _auth: RequireToken,
    Path(NodeContainerPath { node, container }): Path<NodeContainerPath>,
    Query(q): Query<ContainerLogsQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if is_current_master_node(&state, &node) {
        let tail = q.tail.unwrap_or(200);
        return master_container_logs(&container, tail)
            .await
            .map(IntoResponse::into_response);
    }
    let ip = find_peer_ip(&state, &node)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Node '{}' not found", node)))?;
    let tail = q.tail.unwrap_or(200);
    let url = format!(
        "http://{}:{}/containers/{}/logs?tail={}",
        ip, AGENT_API_PORT, container, tail
    );
    let resp = proxy_get(&url, &state.cluster_secret).await?;
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let body = resp
        .bytes()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok((status, axum::body::Body::from(body)).into_response())
}

async fn node_container_restart_handler(
    State(state): State<AppState>,
    _auth: RequireToken,
    Path(NodeContainerPath { node, container }): Path<NodeContainerPath>,
) -> Result<StatusCode, (StatusCode, String)> {
    if is_current_master_node(&state, &node) {
        return master_container_restart(&container).await;
    }
    let ip = find_peer_ip(&state, &node)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Node '{}' not found", node)))?;
    let url = format!(
        "http://{}:{}/containers/{}/restart",
        ip, AGENT_API_PORT, container
    );
    let resp = proxy_post(&url, &state.cluster_secret).await?;
    Ok(StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY))
}

async fn node_container_stop_handler(
    State(state): State<AppState>,
    _auth: RequireToken,
    Path(NodeContainerPath { node, container }): Path<NodeContainerPath>,
) -> Result<StatusCode, (StatusCode, String)> {
    if is_current_master_node(&state, &node) {
        return master_container_stop(&container).await;
    }
    let ip = find_peer_ip(&state, &node)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Node '{}' not found", node)))?;
    let url = format!(
        "http://{}:{}/containers/{}/stop",
        ip, AGENT_API_PORT, container
    );
    let resp = proxy_post(&url, &state.cluster_secret).await?;
    Ok(StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY))
}

async fn node_container_start_handler(
    State(state): State<AppState>,
    _auth: RequireToken,
    Path(NodeContainerPath { node, container }): Path<NodeContainerPath>,
) -> Result<StatusCode, (StatusCode, String)> {
    if is_current_master_node(&state, &node) {
        return master_container_start(&container).await;
    }
    let ip = find_peer_ip(&state, &node)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Node '{}' not found", node)))?;
    let url = format!(
        "http://{}:{}/containers/{}/start",
        ip, AGENT_API_PORT, container
    );
    let resp = proxy_post(&url, &state.cluster_secret).await?;
    Ok(StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY))
}

/// WireGuard endpoint мастера, который получат агенты.
/// R4A_PUBLIC_ENDPOINT (или --public-endpoint) всегда выигрывает у автодетекта.
fn public_endpoint() -> String {
    if let Ok(ep) = std::env::var("R4A_PUBLIC_ENDPOINT") {
        let ep = ep.trim().to_string();
        if !ep.is_empty() {
            return ep;
        }
    }
    format!("{}:{}", get_external_ip(), WG_PORT)
}

fn get_external_ip() -> String {
    static CACHE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CACHE.get_or_init(detect_external_ip).clone()
}

/// Приватные диапазоны, включая CGNAT (100.64/10 — например Tailscale) и link-local.
fn is_private_ipv4(ip: &str) -> bool {
    match ip.parse::<std::net::Ipv4Addr>() {
        Ok(a) => {
            let o = a.octets();
            a.is_private()
                || a.is_loopback()
                || a.is_link_local()
                || (o[0] == 100 && (64..128).contains(&o[1]))
        }
        Err(_) => true,
    }
}

fn detect_external_ip() -> String {
    let out = std::process::Command::new("ip")
        .args(["-4", "addr", "show"])
        .output();
    let mut fallback = "127.0.0.1".to_string();

    if let Ok(o) = out {
        let text = String::from_utf8_lossy(&o.stdout);
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with("inet ") && !line.contains("127.") && !line.contains("10.42.") {
                if let Some(ip_with_mask) = line.split_whitespace().nth(1) {
                    if let Some(ip) = ip_with_mask.split('/').next() {
                        if !is_private_ipv4(ip) {
                            return ip.to_string();
                        }
                        if fallback == "127.0.0.1" {
                            fallback = ip.to_string();
                        }
                    }
                }
            }
        }
    }

    // На интерфейсах только приватные адреса (1:1 NAT облака) — спрашиваем внешний сервис
    if let Some(ip) = query_external_ip_service() {
        return ip;
    }
    fallback
}

/// GET api.ipify.org по plain HTTP через std TcpStream — без reqwest, чтобы
/// безопасно вызываться из синхронного кода внутри tokio-рантайма.
fn query_external_ip_service() -> Option<String> {
    use std::io::{Read, Write};
    let timeout = std::time::Duration::from_secs(3);
    let addr = std::net::ToSocketAddrs::to_socket_addrs(&"api.ipify.org:80")
        .ok()?
        .next()?;
    let mut stream = std::net::TcpStream::connect_timeout(&addr, timeout).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: api.ipify.org\r\nConnection: close\r\n\r\n")
        .ok()?;
    let mut resp = String::new();
    stream.read_to_string(&mut resp).ok()?;
    let body = resp.split("\r\n\r\n").nth(1)?.trim();
    body.parse::<std::net::Ipv4Addr>().ok()?;
    tracing::info!("External IP detected via ipify: {}", body);
    Some(body.to_string())
}

// --- Connection handlers ---

async fn connections_list_handler(
    State(state): State<AppState>,
    auth: RequireToken,
) -> Result<Json<Vec<Connection>>, (StatusCode, String)> {
    if !state.store.can(
        &auth.token.username,
        Verb::List,
        Resource::Connections,
        None,
    ) {
        return Err((
            StatusCode::FORBIDDEN,
            "Insufficient permissions".to_string(),
        ));
    }
    let conns = state
        .store
        .list_connections()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(conns))
}

async fn connect_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Json(req): Json<ConnectRequest>,
) -> Result<Json<ConnectResponse>, (StatusCode, String)> {
    if !state.store.can(
        &auth.token.username,
        Verb::Create,
        Resource::Connections,
        None,
    ) {
        return Err((
            StatusCode::FORBIDDEN,
            "Insufficient permissions".to_string(),
        ));
    }

    r4a_vpn::wireguard::validate_wg_pubkey(&req.pubkey)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Evict any existing active connection with the same pubkey or label
    if let Ok(existing) = state.store.list_connections() {
        for old in existing {
            if old.pubkey == req.pubkey || (req.label.is_some() && old.label == req.label) {
                if let Err(e) = r4a_vpn::wireguard::remove_peer(&old.pubkey) {
                    warn!("WireGuard remove_peer (evict) failed: {}", e);
                }
                let _ = state.store.delete_connection(&old.id).await;
            }
        }
    }

    // Use pinned IP for this label, or allocate a new one and pin it
    let vpn_ip = if let Some(label) = &req.label {
        match state.store.get_label_ip(label) {
            Ok(Some(ip)) => ip,
            _ => {
                let ip = {
                    let mut next = state.next_ip.lock().unwrap();
                    if *next > 254 {
                        return Err((
                            StatusCode::SERVICE_UNAVAILABLE,
                            "VPN IP pool exhausted".to_string(),
                        ));
                    }
                    let ip = format!("10.42.0.{}", *next);
                    *next += 1;
                    ip
                };
                if let Err(e) = state.store.set_label_ip(label, &ip).await {
                    warn!("Failed to pin label IP: {}", e);
                }
                ip
            }
        }
    } else {
        let mut next = state.next_ip.lock().unwrap();
        if *next > 254 {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "VPN IP pool exhausted".to_string(),
            ));
        }
        let ip = format!("10.42.0.{}", *next);
        *next += 1;
        ip
    };

    if let Err(e) = r4a_vpn::wireguard::add_peer(&req.pubkey, &vpn_ip) {
        warn!("WireGuard add_peer failed: {}", e);
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let id = {
        let mut bytes = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut bytes);
        hex::encode(bytes)
    };

    let conn = Connection {
        id: id.clone(),
        pubkey: req.pubkey.clone(),
        vpn_ip: vpn_ip.clone(),
        label: req.label,
        connected_at: now,
        last_seen: now,
    };

    state
        .store
        .put_connection(&conn)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let master_endpoint = public_endpoint();

    Ok(Json(ConnectResponse {
        id,
        vpn_ip,
        master_pubkey: state.master_pub_key.clone(),
        master_endpoint,
        heartbeat_interval_secs: 30,
    }))
}

async fn disconnect_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !state.store.can(
        &auth.token.username,
        Verb::Delete,
        Resource::Connections,
        None,
    ) {
        return Err((
            StatusCode::FORBIDDEN,
            "Insufficient permissions".to_string(),
        ));
    }

    let conn = state
        .store
        .get_connection(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Connection not found".to_string()))?;

    if let Err(e) = r4a_vpn::wireguard::remove_peer(&conn.pubkey) {
        warn!("WireGuard remove_peer failed: {}", e);
    }

    state
        .store
        .delete_connection(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

async fn connection_heartbeat_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !state.store.can(
        &auth.token.username,
        Verb::Update,
        Resource::Connections,
        None,
    ) {
        return Err((
            StatusCode::FORBIDDEN,
            "Insufficient permissions".to_string(),
        ));
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let found = state
        .store
        .update_connection_heartbeat(&id, now)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if found {
        Ok(StatusCode::OK)
    } else {
        Err((StatusCode::NOT_FOUND, "Connection not found".to_string()))
    }
}

// ── Telemetry: centralized container logs in ClickHouse ──────────────────────

const LOGS_CH_CONFIG_KEY: &[u8] = b"logs_ch_config";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LogsChConfig {
    node: String,
    endpoint: String,
    password: String,
}

#[derive(Debug, Serialize)]
struct LogsConfigResponse {
    configured: bool,
    node: Option<String>,
    endpoint: Option<String>,
    ready: bool,
}

#[derive(Debug, Deserialize)]
struct LogsSetupRequest {
    node: String,
    endpoint: Option<String>,
}

fn load_logs_ch_config(store: &Store) -> Result<Option<LogsChConfig>, (StatusCode, String)> {
    store
        .get("core", LOGS_CH_CONFIG_KEY)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map(|bytes| {
            serde_json::from_slice::<LogsChConfig>(&bytes)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
        })
        .transpose()
}

async fn load_active_logs_ch_config(
    store: &Store,
) -> Result<Option<LogsChConfig>, (StatusCode, String)> {
    let Some(cfg) = load_logs_ch_config(store)? else {
        return Ok(None);
    };

    let has_manifest = store
        .list_manifests()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .into_iter()
        .any(|m| m.app.name == "clickhouse");

    if has_manifest {
        Ok(Some(cfg))
    } else {
        store
            .delete("core", LOGS_CH_CONFIG_KEY)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        info!("Removed stale Logs ClickHouse config because manifest 'clickhouse' is absent");
        Ok(None)
    }
}

fn ch_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

async fn ch_ping(client: &reqwest::Client, cfg: &LogsChConfig) -> bool {
    client
        .get(format!("{}/ping", cfg.endpoint.trim_end_matches('/')))
        .basic_auth("default", Some(&cfg.password))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

async fn ch_exec(
    client: &reqwest::Client,
    cfg: &LogsChConfig,
    sql: &str,
) -> Result<String, (StatusCode, String)> {
    let resp = client
        .post(cfg.endpoint.trim_end_matches('/'))
        .basic_auth("default", Some(&cfg.password))
        .body(sql.to_string())
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if status.is_success() {
        Ok(body)
    } else {
        Err((
            StatusCode::BAD_GATEWAY,
            format!("ClickHouse HTTP {}: {}", status, body),
        ))
    }
}

async fn ensure_logs_ch_schema(cfg: LogsChConfig) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    for _ in 0..60 {
        if ch_ping(&client, &cfg).await {
            let db = ch_exec(&client, &cfg, r4a_telemetry::CH_CREATE_DATABASE).await;
            let table = ch_exec(&client, &cfg, r4a_telemetry::CH_CREATE_LOGS_TABLE).await;
            match (db, table) {
                (Ok(_), Ok(_)) => {
                    // Догоняем индекс поиска на таблицах, созданных до его появления.
                    if let Err((_, e)) =
                        ch_exec(&client, &cfg, r4a_telemetry::CH_ADD_LOGS_LINE_INDEX).await
                    {
                        warn!("ClickHouse line index add failed: {}", e);
                    } else if let Err((_, e)) =
                        ch_exec(&client, &cfg, r4a_telemetry::CH_MATERIALIZE_LOGS_LINE_INDEX).await
                    {
                        warn!("ClickHouse line index materialize failed: {}", e);
                    }
                    info!("ClickHouse logs schema is ready at {}", cfg.endpoint);
                    return;
                }
                (Err((_, e)), _) | (_, Err((_, e))) => {
                    warn!("ClickHouse schema init failed: {}", e)
                }
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }

    warn!("ClickHouse logs schema init timed out for {}", cfg.endpoint);
}

async fn logs_config_handler(
    State(state): State<AppState>,
    auth: RequireToken,
) -> Result<Json<LogsConfigResponse>, (StatusCode, String)> {
    if !state
        .store
        .can(&auth.token.username, Verb::Get, Resource::Logs, None)
    {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }
    let Some(cfg) = load_active_logs_ch_config(&state.store).await? else {
        return Ok(Json(LogsConfigResponse {
            configured: false,
            node: None,
            endpoint: None,
            ready: false,
        }));
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap_or_default();
    let ready = ch_ping(&client, &cfg).await;
    Ok(Json(LogsConfigResponse {
        configured: true,
        node: Some(cfg.node),
        endpoint: Some(cfg.endpoint),
        ready,
    }))
}

async fn logs_setup_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Json(req): Json<LogsSetupRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !state
        .store
        .can(&auth.token.username, Verb::Create, Resource::Logs, None)
        && !state
            .store
            .can(&auth.token.username, Verb::Update, Resource::Logs, None)
    {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }

    let node = req.node.trim().to_string();
    if node.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "node is required".to_string()));
    }

    let node_ip = if node == state.my_node_name || node == state.my_vpn_ip {
        state.my_vpn_ip.clone()
    } else {
        let peers = state.peers.lock().unwrap();
        peers
            .values()
            .find(|p| p.name == node)
            .map(|p| p.ip.clone())
            .ok_or((StatusCode::BAD_REQUEST, "Unknown node".to_string()))?
    };

    let endpoint = req
        .endpoint
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("http://{}:8123", node_ip));
    let password = generate_secret();
    let cfg = LogsChConfig {
        node: node.clone(),
        endpoint: endpoint.clone(),
        password: password.clone(),
    };

    let mut env = HashMap::new();
    env.insert("CLICKHOUSE_PASSWORD".to_string(), password);
    env.insert(
        "CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT".to_string(),
        "1".to_string(),
    );

    let manifest = Manifest {
        app: r4a_core::AppConfig {
            name: "clickhouse".to_string(),
            node_selector: node.clone(),
        },
        container: Some(r4a_core::ContainerConfig {
            image: "clickhouse/clickhouse-server:24.8-alpine".to_string(),
            restart: "always".to_string(),
            command: None,
            ports: Some(vec!["8123:8123".to_string()]),
            volumes: Some(vec!["r4a-clickhouse-data:/var/lib/clickhouse".to_string()]),
        }),
        systemd: None,
        ingress: None,
        env,
    };

    state
        .store
        .put_manifest(&manifest)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    state
        .store
        .put(
            "core",
            LOGS_CH_CONFIG_KEY,
            &serde_json::to_vec(&cfg)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tokio::spawn(ensure_logs_ch_schema(cfg));
    info!(
        "Logs ClickHouse setup requested by {} on node {}",
        auth.token.username, node
    );
    Ok(StatusCode::ACCEPTED)
}

async fn logs_agent_config_handler(
    State(state): State<AppState>,
    _auth: RequireSecret,
) -> Result<Json<r4a_telemetry::LogsChTarget>, (StatusCode, String)> {
    let cfg = load_active_logs_ch_config(&state.store).await?.ok_or((
        StatusCode::NOT_FOUND,
        "Logs ClickHouse is not configured".to_string(),
    ))?;
    Ok(Json(r4a_telemetry::LogsChTarget {
        endpoint: cfg.endpoint,
        password: cfg.password,
    }))
}

#[derive(Deserialize)]
struct LogsQueryParams {
    node: String,
    container: String,
    tail: Option<usize>,
    /// Полнотекстовый поиск по строке (case-insensitive substring).
    q: Option<String>,
    /// "stdout" | "stderr" — без параметра ищем в обоих.
    stream: Option<String>,
}

async fn logs_query_handler(
    State(state): State<AppState>,
    auth: RequireToken,
    Query(q): Query<LogsQueryParams>,
) -> Result<Json<Vec<r4a_telemetry::LogEntry>>, (StatusCode, String)> {
    if !state
        .store
        .can(&auth.token.username, Verb::Get, Resource::Logs, None)
    {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }
    let tail = q.tail.unwrap_or(200).min(5000);
    let cfg = load_active_logs_ch_config(&state.store).await?.ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Logs ClickHouse is not configured".to_string(),
    ))?;

    let mut filters = format!(
        "node = '{}' AND container = '{}'",
        ch_escape(&q.node),
        ch_escape(&q.container),
    );
    if let Some(stream) = q.stream.as_deref().filter(|s| !s.is_empty()) {
        filters.push_str(&format!(" AND stream = '{}'", ch_escape(stream)));
    }
    if let Some(search) = q.q.as_deref().filter(|s| !s.is_empty()) {
        // positionCaseInsensitive работает по индексируемому line — узкий диапазон
        // node/container уже отфильтрован по primary key, поиск быстрый даже без
        // попадания в line_ngram skip-индекс (он всё равно помогает CH пропускать
        // гранулы, не содержащие искомую подстроку, на больших партициях).
        filters.push_str(&format!(
            " AND positionCaseInsensitive(line, '{}') > 0",
            ch_escape(search)
        ));
    }

    let sql = format!(
        "SELECT node, container, ts_ms, stream, line FROM \
         (SELECT node, container, ts_ms, stream, line FROM r4a.logs \
          WHERE {} ORDER BY ts_ms DESC LIMIT {}) \
         ORDER BY ts_ms ASC FORMAT JSONEachRow",
        filters, tail,
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();
    let body = ch_exec(&client, &cfg, &sql).await?;
    #[derive(Deserialize)]
    struct LogRow {
        node: String,
        container: String,
        ts_ms: serde_json::Value,
        stream: String,
        line: String,
    }
    let entries = body
        .lines()
        .filter_map(|line| {
            let row = serde_json::from_str::<LogRow>(line).ok()?;
            let ts_ms = row
                .ts_ms
                .as_u64()
                .or_else(|| row.ts_ms.as_str().and_then(|s| s.parse::<u64>().ok()))?;
            Some(r4a_telemetry::LogEntry {
                node: row.node,
                container: row.container,
                ts_ms,
                stream: row.stream,
                line: row.line,
            })
        })
        .collect();
    Ok(Json(entries))
}

async fn logs_containers_handler(
    State(state): State<AppState>,
    auth: RequireToken,
) -> Result<Json<Vec<(String, String)>>, (StatusCode, String)> {
    if !state
        .store
        .can(&auth.token.username, Verb::List, Resource::Logs, None)
    {
        return Err((StatusCode::FORBIDDEN, "Access denied".to_string()));
    }
    let cfg = load_active_logs_ch_config(&state.store).await?.ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Logs ClickHouse is not configured".to_string(),
    ))?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();
    let body = ch_exec(
        &client,
        &cfg,
        "SELECT node, container FROM r4a.logs GROUP BY node, container ORDER BY node, container FORMAT JSONEachRow",
    ).await?;
    #[derive(Deserialize)]
    struct Row {
        node: String,
        container: String,
    }
    let list = body
        .lines()
        .filter_map(|line| serde_json::from_str::<Row>(line).ok())
        .map(|r| (r.node, r.container))
        .collect();
    Ok(Json(list))
}

// ── DNS server for *.r4a.local ────────────────────────────────────────────────

async fn run_dns_server(vpn_ip: String, store: Store) {
    let bind_addr = format!("{}:53", vpn_ip);
    let socket = match tokio::net::UdpSocket::bind(&bind_addr).await {
        Ok(s) => {
            info!("DNS server listening on {}", bind_addr);
            Arc::new(s)
        }
        Err(e) => {
            warn!("DNS server: failed to bind {}: {}", bind_addr, e);
            return;
        }
    };

    let mut buf = [0u8; 512];
    loop {
        let (len, src) = match socket.recv_from(&mut buf).await {
            Ok(x) => x,
            Err(e) => {
                warn!("DNS recv error: {}", e);
                continue;
            }
        };
        let query = buf[..len].to_vec();
        let sock = socket.clone();
        let st = store.clone();
        tokio::spawn(async move {
            if let Some(response) = handle_dns_query(&query, &st).await {
                let _ = sock.send_to(&response, src).await;
            }
        });
    }
}

async fn handle_dns_query(query: &[u8], store: &Store) -> Option<Vec<u8>> {
    if query.len() < 13 {
        return None;
    }

    let (qname, after_qname) = dns_parse_qname(query, 12)?;
    if after_qname + 4 > query.len() {
        return None;
    }
    let qtype = u16::from_be_bytes([query[after_qname], query[after_qname + 1]]);
    let qname_lower = qname.to_lowercase();

    if qname_lower == "r4a.local" || qname_lower.ends_with(".r4a.local") {
        // AAAA query — we have no IPv6, respond NOERROR with empty answers
        if qtype == 28 {
            return Some(dns_noerror_empty(query));
        }
        // A query (1) or ANY (255) — try to resolve
        if qtype == 1 || qtype == 255 {
            let ip = dns_resolve(&qname_lower, store).await;
            return Some(match ip {
                Some(ip) => dns_a_response(query, ip),
                None => dns_nxdomain(query),
            });
        }
        // Other types for our domain — NOERROR, no answers
        return Some(dns_noerror_empty(query));
    }

    // Not our domain — forward to upstream
    dns_forward(query).await
}

async fn dns_resolve(qname: &str, store: &Store) -> Option<std::net::Ipv4Addr> {
    // master and all its subdomains (e.g. web.master.r4a.local, api.master.r4a.local)
    if qname == "master.r4a.local" || qname.ends_with(".master.r4a.local") {
        return "10.42.0.1".parse().ok();
    }

    let label = qname.strip_suffix(".r4a.local")?;

    let peers: HashMap<String, PeerInfo> = store
        .get("core", b"peers")
        .ok()
        .flatten()
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or_default();

    // Direct node name: <node>.r4a.local
    if let Some(peer) = peers.values().find(|p| p.name.to_lowercase() == label) {
        return peer.ip.parse().ok();
    }

    // Subdomain of a node: <sub>.<node>.r4a.local → node's VPN IP
    if let Some(dot_pos) = label.rfind('.') {
        let node_name = &label[dot_pos + 1..];
        if let Some(peer) = peers.values().find(|p| p.name.to_lowercase() == node_name) {
            return peer.ip.parse().ok();
        }
    }

    // Connection labels: <label>.r4a.local
    store.get_label_ip(label).ok().flatten()?.parse().ok()
}

async fn dns_forward(query: &[u8]) -> Option<Vec<u8>> {
    let upstream = tokio::net::UdpSocket::bind("0.0.0.0:0").await.ok()?;
    upstream.send_to(query, "8.8.8.8:53").await.ok()?;
    let mut buf = vec![0u8; 512];
    let len = tokio::time::timeout(std::time::Duration::from_secs(3), upstream.recv(&mut buf))
        .await
        .ok()?
        .ok()?;
    buf.truncate(len);
    Some(buf)
}

fn dns_parse_qname(buf: &[u8], start: usize) -> Option<(String, usize)> {
    let mut labels: Vec<String> = Vec::new();
    let mut pos = start;
    loop {
        if pos >= buf.len() {
            return None;
        }
        let len = buf[pos] as usize;
        if len == 0 {
            pos += 1;
            break;
        }
        if len & 0xC0 == 0xC0 {
            return None;
        } // no pointers in queries
        pos += 1;
        if pos + len > buf.len() {
            return None;
        }
        labels.push(std::str::from_utf8(&buf[pos..pos + len]).ok()?.to_string());
        pos += len;
    }
    Some((labels.join("."), pos))
}

fn dns_a_response(query: &[u8], ip: std::net::Ipv4Addr) -> Vec<u8> {
    let mut r = Vec::new();
    r.extend_from_slice(&query[0..2]); // transaction ID
    r.extend_from_slice(&[0x81, 0x80]); // QR=1 AA=0 RD=1 RA=1 RCODE=0
    r.extend_from_slice(&[0x00, 0x01]); // QDCOUNT=1
    r.extend_from_slice(&[0x00, 0x01]); // ANCOUNT=1
    r.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // NSCOUNT=0 ARCOUNT=0
    r.extend_from_slice(&query[12..]); // question section
    r.extend_from_slice(&[0xC0, 0x0C]); // NAME: pointer to offset 12
    r.extend_from_slice(&[0x00, 0x01]); // TYPE=A
    r.extend_from_slice(&[0x00, 0x01]); // CLASS=IN
    r.extend_from_slice(&[0x00, 0x00, 0x00, 0x3C]); // TTL=60
    r.extend_from_slice(&[0x00, 0x04]); // RDLENGTH=4
    r.extend_from_slice(&ip.octets()); // RDATA
    r
}

fn dns_nxdomain(query: &[u8]) -> Vec<u8> {
    let mut r = Vec::new();
    r.extend_from_slice(&query[0..2]);
    r.extend_from_slice(&[0x81, 0x83]); // QR=1 RD=1 RA=1 RCODE=3 (NXDOMAIN)
    r.extend_from_slice(&[0x00, 0x01]);
    r.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    r.extend_from_slice(&query[12..]);
    r
}

fn dns_noerror_empty(query: &[u8]) -> Vec<u8> {
    let mut r = Vec::new();
    r.extend_from_slice(&query[0..2]);
    r.extend_from_slice(&[0x81, 0x80]); // QR=1 RD=1 RA=1 RCODE=0 (NOERROR)
    r.extend_from_slice(&[0x00, 0x01]);
    r.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    r.extend_from_slice(&query[12..]);
    r
}

// ── TLS / CA ──────────────────────────────────────────────────────────────────

async fn ca_cert_handler(State(state): State<AppState>) -> impl IntoResponse {
    match state.store.get("vault_meta", b"tls_ca_cert") {
        Ok(Some(bytes)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/x-pem-file")],
            String::from_utf8_lossy(&bytes).to_string(),
        ),
        _ => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain")],
            "CA cert not found".to_string(),
        ),
    }
}

async fn ensure_tls_certs(store: &Store) -> anyhow::Result<(String, String)> {
    // Return existing certs if already generated
    if let (Ok(Some(cert_b)), Ok(Some(key_b))) = (
        store.get("vault_meta", b"tls_server_cert"),
        store.get("vault_meta", b"tls_server_key"),
    ) {
        info!("Loaded existing TLS server certificate");
        return Ok((
            String::from_utf8(cert_b.to_vec())?,
            String::from_utf8(key_b.to_vec())?,
        ));
    }

    // Generate CA
    let mut ca_params = CertificateParams::new(vec![]);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "r4a Local CA");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca_cert = Certificate::from_params(ca_params)?;
    let ca_cert_pem = ca_cert.serialize_pem()?;
    let ca_key_pem = ca_cert.serialize_private_key_pem();

    // Generate server cert with SANs covering all *.r4a.local and *.master.r4a.local
    let mut srv_params = CertificateParams::new(vec![
        "master.r4a.local".to_string(),
        "*.master.r4a.local".to_string(),
        "*.r4a.local".to_string(),
    ]);
    srv_params
        .distinguished_name
        .push(DnType::CommonName, "r4a Server");
    srv_params
        .subject_alt_names
        .push(SanType::IpAddress(std::net::IpAddr::V4(
            "10.42.0.1".parse()?,
        )));
    srv_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let srv_cert = Certificate::from_params(srv_params)?;
    let srv_cert_pem = srv_cert.serialize_pem_with_signer(&ca_cert)?;
    let srv_key_pem = srv_cert.serialize_private_key_pem();

    store
        .put("vault_meta", b"tls_ca_cert", ca_cert_pem.as_bytes())
        .await?;
    store
        .put("vault_meta", b"tls_ca_key", ca_key_pem.as_bytes())
        .await?;
    store
        .put("vault_meta", b"tls_server_cert", srv_cert_pem.as_bytes())
        .await?;
    store
        .put("vault_meta", b"tls_server_key", srv_key_pem.as_bytes())
        .await?;

    info!("Generated new TLS CA and server certificate");
    Ok((srv_cert_pem, srv_key_pem))
}

// ── HTTPS proxy on port 443 ───────────────────────────────────────────────────

async fn start_https_proxy(vpn_ip: String, cert_pem: String, key_pem: String) {
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use hyper_util::server::conn::auto::Builder as HyperBuilder;
    use rustls::ServerConfig;
    use rustls_pemfile;
    use std::sync::Arc;
    use tokio_rustls::TlsAcceptor;

    let certs: Vec<rustls::pki_types::CertificateDer<'static>> =
        rustls_pemfile::certs(&mut cert_pem.as_bytes())
            .filter_map(|c| c.ok().map(|c| c.into_owned()))
            .collect();

    let key = match rustls_pemfile::private_key(&mut key_pem.as_bytes()) {
        Ok(Some(k)) => k.clone_key(),
        _ => {
            warn!("HTTPS proxy: no private key found in PEM");
            return;
        }
    };

    let tls_config = match ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
    {
        Ok(c) => c,
        Err(e) => {
            warn!("HTTPS proxy: TLS config error: {}", e);
            return;
        }
    };

    let acceptor = TlsAcceptor::from(Arc::new(tls_config));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap();

    let proxy_app = Router::new()
        .route("/", any(proxy_handler))
        .route("/*path", any(proxy_handler))
        .with_state(client);

    let addr: std::net::SocketAddr = format!("{}:{}", vpn_ip, HTTPS_PORT).parse().unwrap();
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            warn!("HTTPS proxy: bind {} failed: {}", addr, e);
            return;
        }
    };
    info!("HTTPS proxy listening on https://{}", addr);

    loop {
        let (tcp_stream, _) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                warn!("HTTPS accept error: {}", e);
                continue;
            }
        };
        let acceptor = acceptor.clone();
        let svc = proxy_app.clone();

        tokio::spawn(async move {
            let tls_stream = match acceptor.accept(tcp_stream).await {
                Ok(s) => s,
                Err(e) => {
                    warn!("TLS handshake failed: {}", e);
                    return;
                }
            };
            let io = TokioIo::new(tls_stream);
            let service = hyper::service::service_fn(move |req| {
                let mut s = svc.clone();
                async move {
                    use tower::Service as _;
                    s.call(req.map(axum::body::Body::new)).await
                }
            });
            if let Err(e) = HyperBuilder::new(TokioExecutor::new())
                .serve_connection(io, service)
                .await
            {
                warn!("HTTPS connection error: {}", e);
            }
        });
    }
}

async fn proxy_handler(
    State(client): State<reqwest::Client>,
    req: axum::http::Request<Body>,
) -> Response {
    let host = req
        .headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");

    let target_base = if host.starts_with("web.") {
        "http://127.0.0.1:3502"
    } else if host.starts_with("api.") {
        "http://127.0.0.1:3501"
    } else {
        "http://127.0.0.1:3500"
    };

    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/");
    let url = format!("{}{}", target_base, path_and_query);

    let method = match reqwest::Method::from_bytes(req.method().as_str().as_bytes()) {
        Ok(m) => m,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    let mut builder = client.request(method, &url);
    for (k, v) in req.headers() {
        if k != axum::http::header::HOST {
            builder = builder.header(k, v);
        }
    }

    let body_bytes = match axum::body::to_bytes(req.into_body(), 64 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    builder = builder.body(body_bytes);

    let upstream = match builder.send().await {
        Ok(r) => r,
        Err(e) => {
            warn!("HTTPS proxy: upstream error for {}: {}", url, e);
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };

    let status = axum::http::StatusCode::from_u16(upstream.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut resp = Response::builder().status(status);
    for (k, v) in upstream.headers() {
        resp = resp.header(k, v);
    }
    let body_bytes = match upstream.bytes().await {
        Ok(b) => b,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };
    resp.body(Body::from(body_bytes))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
