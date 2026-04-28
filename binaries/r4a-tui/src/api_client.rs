use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;

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

#[derive(Deserialize, Clone)]
pub struct RepoInfo {
    pub name: String,
    pub clone_url: String,
}

#[derive(Deserialize, Clone)]
pub struct AgentUpdateInfo {
    pub status: String,
    pub checksum: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct UpdateStatus {
    pub master_checksum: Option<String>,
    pub update_pending: bool,
    pub agents: HashMap<String, AgentUpdateInfo>,
}

#[derive(Deserialize)]
pub struct TestResponse {
    pub ok: bool,
    pub checksum: Option<String>,
    pub message: String,
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

    pub async fn git_repos(&self) -> Result<Vec<RepoInfo>> {
        let repos = self
            .client
            .get(format!("{}/api/git/repos", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(repos)
    }

    pub async fn create_repo(&self, name: &str) -> Result<RepoInfo> {
        let body = serde_json::json!({"name": name});
        let repo = self
            .client
            .post(format!("{}/api/git/repos", self.base_url))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(repo)
    }

    pub async fn update_status(&self) -> Result<UpdateStatus> {
        let status = self
            .client
            .get(format!("{}/api/update/status", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(status)
    }

    pub async fn update_test(&self) -> Result<TestResponse> {
        let resp = self
            .client
            .post(format!("{}/api/update/test", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp)
    }

    pub async fn update_trigger(&self) -> Result<()> {
        self.client
            .post(format!("{}/api/update/trigger", self.base_url))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}
