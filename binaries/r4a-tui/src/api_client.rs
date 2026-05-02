use anyhow::Result;
pub use r4a_core::{NodeInfo, RepoInfo};
use serde::Deserialize;
use std::collections::HashMap;

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
    secret: Option<String>,
    client: reqwest::Client,
}

impl ApiClient {
    pub fn new(base_url: &str, secret: Option<String>) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            secret,
            client: reqwest::Client::new(),
        }
    }

    fn authenticated_get(&self, url: String) -> reqwest::RequestBuilder {
        let mut req = self.client.get(url);
        if let Some(ref s) = self.secret {
            req = req.header("X-R4A-Secret", s);
        }
        req
    }

    fn authenticated_post(&self, url: String) -> reqwest::RequestBuilder {
        let mut req = self.client.post(url);
        if let Some(ref s) = self.secret {
            req = req.header("X-R4A-Secret", s);
        }
        req
    }

    pub async fn nodes(&self) -> Result<Vec<NodeInfo>> {
        let nodes = self
            .authenticated_get(format!("{}/api/nodes", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(nodes)
    }

    pub async fn git_repos(&self) -> Result<Vec<RepoInfo>> {
        let repos = self
            .authenticated_get(format!("{}/api/git/repos", self.base_url))
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
            .authenticated_post(format!("{}/api/git/repos", self.base_url))
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
            .authenticated_get(format!("{}/api/update/status", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(status)
    }

    pub async fn update_test(&self) -> Result<TestResponse> {
        let resp = self
            .authenticated_post(format!("{}/api/update/test", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp)
    }

    pub async fn update_trigger(&self) -> Result<()> {
        self.authenticated_post(format!("{}/api/update/trigger", self.base_url))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}
