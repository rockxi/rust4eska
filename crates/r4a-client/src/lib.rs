use anyhow::Result;
pub use r4a_core::{NodeInfo, RepoInfo, Manifest, AppConfig, ContainerConfig, models::{Token, Policy, Binding, Verb, Resource, VaultConfig, Connection, ConnectRequest, ConnectResponse}};
use serde::Deserialize;
use std::collections::HashMap;
use std::cell::RefCell;

#[derive(Deserialize, Clone, Debug)]
pub struct AgentUpdateInfo {
    pub status: String,
    pub checksum: Option<String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct UpdateStatus {
    pub master_checksum: Option<String>,
    pub update_pending: bool,
    pub agents: HashMap<String, AgentUpdateInfo>,
}

#[derive(Deserialize, Debug)]
pub struct TestResponse {
    pub ok: bool,
    pub checksum: Option<String>,
    pub message: String,
}

pub struct ApiClient {
    base_url: String,
    secret: Option<String>,
    bearer_token: RefCell<Option<String>>,
    client: reqwest::Client,
}

impl ApiClient {
    pub fn new(base_url: &str, secret: Option<String>) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            secret,
            bearer_token: RefCell::new(None),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_token(base_url: &str, token: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            secret: None,
            bearer_token: RefCell::new(Some(token)),
            client: reqwest::Client::new(),
        }
    }

    async fn ensure_token(&self) -> Result<()> {
        if self.bearer_token.borrow().is_some() {
            return Ok(());
        }
        if let Some(ref secret) = self.secret {
            let resp = self
                .client
                .post(format!("{}/api/tokens/exchange", self.base_url))
                .header("X-R4A-Secret", secret)
                .send()
                .await?
                .error_for_status()?;
            let token: Token = resp.json().await?;
            *self.bearer_token.borrow_mut() = Some(token.id);
        }
        Ok(())
    }

    fn authenticated_get(&self, url: String) -> reqwest::RequestBuilder {
        let mut req = self.client.get(url);
        if let Some(ref token) = *self.bearer_token.borrow() {
            req = req.header("Authorization", format!("Bearer {}", token));
        } else if let Some(ref s) = self.secret {
            req = req.header("X-R4A-Secret", s);
        }
        req
    }

    fn authenticated_post(&self, url: String) -> reqwest::RequestBuilder {
        let mut req = self.client.post(url);
        if let Some(ref token) = *self.bearer_token.borrow() {
            req = req.header("Authorization", format!("Bearer {}", token));
        } else if let Some(ref s) = self.secret {
            req = req.header("X-R4A-Secret", s);
        }
        req
    }

    fn authenticated_delete(&self, url: String) -> reqwest::RequestBuilder {
        let mut req = self.client.delete(url);
        if let Some(ref token) = *self.bearer_token.borrow() {
            req = req.header("Authorization", format!("Bearer {}", token));
        } else if let Some(ref s) = self.secret {
            req = req.header("X-R4A-Secret", s);
        }
        req
    }

    pub async fn nodes(&self) -> Result<Vec<NodeInfo>> {
        self.ensure_token().await?;
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
        self.ensure_token().await?;
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
        self.ensure_token().await?;
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
        self.ensure_token().await?;
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
        self.ensure_token().await?;
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
        self.ensure_token().await?;
        self.authenticated_post(format!("{}/api/update/trigger", self.base_url))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn server_update_trigger(&self) -> Result<()> {
        self.ensure_token().await?;
        self.authenticated_post(format!("{}/api/update/server-trigger", self.base_url))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn get_tui_checksum(&self) -> Result<String> {
        self.ensure_token().await?;
        #[derive(Deserialize)]
        struct Resp { checksum: String }
        let resp: Resp = self.authenticated_get(format!("{}/api/tui-checksum", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp.checksum)
    }

    pub async fn fetch_github_release(&self) -> Result<String> {
        self.ensure_token().await?;
        #[derive(Deserialize)]
        struct Resp { success: bool, message: String, version: Option<String> }
        let resp: Resp = self.authenticated_post(format!("{}/api/update/fetch-github", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if !resp.success {
            anyhow::bail!("{}", resp.message);
        }
        Ok(resp.version.unwrap_or_else(|| "unknown".to_string()))
    }

    pub async fn download_tui_binary(&self) -> Result<Vec<u8>> {
        self.ensure_token().await?;
        let bytes = self.authenticated_get(format!("{}/api/tui-binary", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        Ok(bytes.to_vec())
    }

    pub async fn vault_list(&self, config_id: &str) -> Result<Vec<String>> {
        self.ensure_token().await?;
        let keys = self
            .authenticated_get(format!("{}/api/vault/list?config_id={}", self.base_url, config_id))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(keys)
    }

    pub async fn vault_get(&self, config_id: &str, key: &str) -> Result<String> {
        self.ensure_token().await?;
        let val = self
            .authenticated_get(format!("{}/api/vault?config_id={}&key={}", self.base_url, config_id, key))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(val)
    }

    pub async fn vault_set(&self, config_id: &str, key: &str, value: &str) -> Result<()> {
        self.ensure_token().await?;
        let body = serde_json::json!({"config_id": config_id, "key": key, "value": value});
        self.authenticated_post(format!("{}/api/vault", self.base_url))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn vault_delete(&self, config_id: &str, key: &str) -> Result<()> {
        self.ensure_token().await?;
        self.authenticated_delete(format!("{}/api/vault?config_id={}&key={}", self.base_url, config_id, key))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn vault_configs_list(&self) -> Result<Vec<VaultConfig>> {
        self.ensure_token().await?;
        let configs = self
            .authenticated_get(format!("{}/api/vault/configs", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(configs)
    }

    pub async fn vault_config_create(&self, name: &str) -> Result<VaultConfig> {
        self.ensure_token().await?;
        let body = serde_json::json!({"name": name});
        let config = self
            .authenticated_post(format!("{}/api/vault/configs", self.base_url))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(config)
    }

    pub async fn tokens_list(&self) -> Result<Vec<Token>> {
        self.ensure_token().await?;
        let tokens = self
            .authenticated_get(format!("{}/api/tokens", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(tokens)
    }

    pub async fn token_delete(&self, id: &str) -> Result<()> {
        self.ensure_token().await?;
        self.authenticated_delete(format!("{}/api/tokens?id={}", self.base_url, id))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn token_create(&self, username: &str, verbs: Vec<Verb>, resources: Vec<Resource>, resource_names: Option<Vec<String>>) -> Result<Token> {
        self.ensure_token().await?;
        let body = serde_json::json!({
            "username": username,
            "verbs": verbs,
            "resources": resources,
            "resource_names": resource_names,
        });
        let token = self.authenticated_post(format!("{}/api/tokens", self.base_url))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(token)
    }

    pub async fn manifests(&self, node: Option<&str>) -> Result<HashMap<String, Manifest>> {
        self.ensure_token().await?;
        let mut url = format!("{}/api/manifests", self.base_url);
        if let Some(n) = node {
            url.push_str(&format!("?node={}", n));
        }
        let manifests = self
            .authenticated_get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(manifests)
    }

    pub async fn manifest_upsert(&self, manifest: &Manifest) -> Result<()> {
        self.ensure_token().await?;
        self.authenticated_post(format!("{}/api/manifests", self.base_url))
            .json(manifest)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn manifest_delete(&self, name: &str) -> Result<()> {
        self.ensure_token().await?;
        self.authenticated_delete(format!("{}/api/manifests?name={}", self.base_url, name))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn connections_list(&self) -> Result<Vec<Connection>> {
        self.ensure_token().await?;
        Ok(self.authenticated_get(format!("{}/api/connections", self.base_url))
            .send().await?.error_for_status()?.json().await?)
    }

    pub async fn connection_create(&self, pubkey: &str, label: Option<&str>) -> Result<ConnectResponse> {
        self.ensure_token().await?;
        Ok(self.authenticated_post(format!("{}/api/connections", self.base_url))
            .json(&ConnectRequest { pubkey: pubkey.to_string(), label: label.map(|s| s.to_string()) })
            .send().await?.error_for_status()?.json().await?)
    }

    pub async fn connection_delete(&self, id: &str) -> Result<()> {
        self.ensure_token().await?;
        self.authenticated_delete(format!("{}/api/connections/{}", self.base_url, id))
            .send().await?.error_for_status()?;
        Ok(())
    }

    pub async fn connection_heartbeat(&self, id: &str) -> Result<()> {
        self.ensure_token().await?;
        self.authenticated_post(format!("{}/api/connections/{}/heartbeat", self.base_url, id))
            .send().await?.error_for_status()?;
        Ok(())
    }

    pub async fn ca_cert(&self) -> Result<String> {
        let resp = self.client
            .get(format!("{}/api/ca-cert", self.base_url))
            .send().await?.error_for_status()?;
        Ok(resp.text().await?)
    }
}
