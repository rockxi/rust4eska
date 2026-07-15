use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub mod models;
pub mod crypto;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub app: AppConfig,
    pub container: Option<ContainerConfig>,
    pub systemd: Option<SystemdConfig>,
    pub ingress: Option<IngressConfig>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub name: String,
    pub node_selector: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngressConfig {
    pub domain: String,
    pub container_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerConfig {
    pub image: String,
    #[serde(default = "default_restart")]
    pub restart: String,
    pub command: Option<Vec<String>>,
    pub ports: Option<Vec<String>>,
    #[serde(default)]
    pub volumes: Option<Vec<String>>,
}

fn default_restart() -> String {
    "always".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemdConfig {
    pub exec: String,
    pub user: Option<String>,
    pub working_dir: Option<String>,
}

pub use models::*;
