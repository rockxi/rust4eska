use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize, Clone)]
pub struct NodeInfo {
    pub ip: String,
    pub name: String,
    pub role: String,
    pub cpu_percent: Option<f32>,
    pub ram_used_mb: Option<u64>,
    pub ram_total_mb: Option<u64>,
    pub vram_used_mb: Option<u64>,
    pub vram_total_mb: Option<u64>,
}

pub struct ApiClient {
    base_url: String,
    client: reqwest::Client,
}

impl ApiClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub async fn nodes(&self) -> Result<Vec<NodeInfo>> {
        let nodes = self
            .client
            .get(format!("{}/api/nodes", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(nodes)
    }
}
