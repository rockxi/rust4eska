use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub private_key: String,
    pub public_key: String,
    pub cluster_secret: Option<String>,
    // Admin login secret: exchanged for an admin token via /api/tokens/exchange.
    // Separate from cluster_secret so agents (which hold cluster_secret) cannot
    // obtain admin tokens.
    #[serde(default)]
    pub admin_secret: Option<String>,
    pub agent_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub pub_key: String,
    pub ip: String,
    pub name: String,
    pub role: String,
    pub public_endpoint: Option<String>,
    /// ip:port агента, наблюдаемый мастером через `wg show` (адрес после NAT).
    #[serde(default)]
    pub observed_endpoint: Option<String>,
    /// Имена нод, с которыми у этого агента установлен прямой P2P-туннель
    /// (репортится самим агентом в MetricsReport).
    #[serde(default)]
    pub p2p_direct: Option<Vec<String>>,
    pub cpu_percent: Option<f32>,
    pub ram_used_mb: Option<u64>,
    pub ram_total_mb: Option<u64>,
    pub vram_used_mb: Option<u64>,
    pub vram_total_mb: Option<u64>,
    pub last_seen: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinRequest {
    pub pub_key: String,
    pub name: Option<String>,
    pub role: Option<String>,
    pub public_endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinResponse {
    pub master_pub_key: String,
    pub agent_vpn_ip: String,
    pub master_endpoint: String,
    pub peers: HashMap<String, PeerInfo>,
    pub agent_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Verb {
    Get,
    List,
    Create,
    Update,
    Delete,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Resource {
    Nodes,
    Manifests,
    Vault,
    GitRepos,
    Registry,
    Tokens,
    Policies,
    Bindings,
    Connections,
    Logs,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub verbs: Vec<Verb>,
    pub resources: Vec<Resource>,
    pub resource_names: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub id: String,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Binding {
    pub id: String,
    pub subject: String,
    pub policy_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub username: String,
    pub password_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub id: String,
    pub username: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultSecret {
    pub config_id: String,
    pub key: String,
    pub encrypted_value: Vec<u8>,
    pub nonce: Vec<u8>,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultConfig {
    pub id: String,
    pub name: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KekDek {
    #[serde(default = "default_config_id")]
    pub config_id: String,
    pub encrypted_dek: Vec<u8>,
    pub nonce: Vec<u8>,
}

fn default_config_id() -> String {
    "default".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsReport {
    pub agent_vpn_ip: String,
    pub cpu_percent: f32,
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
    pub vram_used_mb: Option<u64>,
    pub vram_total_mb: Option<u64>,
    /// Имена нод, с которыми установлен прямой P2P-туннель (пустой список = все через хаб).
    #[serde(default)]
    pub p2p_direct: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AgentUpdateStatus {
    Unknown,
    Pending,
    Updating,
    Updated,
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentUpdateState {
    pub status: AgentUpdateStatus,
    pub checksum: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoInfo {
    pub name: String,
    pub clone_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub ip: String,
    pub name: String,
    pub role: String,
    pub cpu_percent: Option<f32>,
    pub ram_used_mb: Option<u64>,
    pub ram_total_mb: Option<u64>,
    pub vram_used_mb: Option<u64>,
    pub vram_total_mb: Option<u64>,
    pub last_seen: Option<u64>,
    /// Имена нод, с которыми у этой ноды прямой P2P-туннель (None у мастера).
    #[serde(default)]
    pub p2p_direct: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    pub hash: String,
    pub author: String,
    pub date: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub pubkey: String,
    pub vpn_ip: String,
    pub label: Option<String>,
    pub connected_at: u64,
    pub last_seen: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectRequest {
    pub pubkey: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectResponse {
    pub id: String,
    pub vpn_ip: String,
    pub master_pubkey: String,
    pub master_endpoint: String,
    pub heartbeat_interval_secs: u64,
}
