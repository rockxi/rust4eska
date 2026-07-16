use axum::{extract::State, http::{StatusCode, HeaderMap}, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use sled::Db;
use std::{path::Path, sync::Arc};
use tracing::{debug, error, info};
use r4a_core::crypto;
use r4a_core::{Manifest, models::{VaultSecret, VaultConfig, KekDek, Token, Policy, Binding, Verb, Resource, Rule, Connection}};

#[derive(Clone)]
pub struct Store {
    pub db: Db,
    pub cluster_secret: Arc<std::sync::RwLock<String>>,
    pub vault_keys: Arc<std::sync::RwLock<std::collections::HashMap<String, [u8; 32]>>>,
    // VPN IP других мастеров (например, "10.42.0.1", "10.42.0.2")
    pub masters: Arc<std::sync::RwLock<Vec<String>>>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SyncRequest {
    pub tree: String,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    #[serde(default)]
    pub delete: bool,
}

impl Store {
    pub fn open<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let db = sled::open(path)?;
        Ok(Self {
            db,
            cluster_secret: Arc::new(std::sync::RwLock::new(String::new())),
            vault_keys: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
            masters: Arc::new(std::sync::RwLock::new(Vec::new())),
        })
    }

    pub fn set_masters(&self, master_ips: Vec<String>) {
        let mut w = self.masters.write().unwrap();
        *w = master_ips;
    }

    pub fn set_secret(&self, secret: String) {
        {
            let mut w = self.cluster_secret.write().unwrap();
            *w = secret;
        }
        // После установки секрета инициализируем Vault
        if let Err(e) = self.init_vault() {
            error!("Failed to initialize vault: {}", e);
        }
    }

    fn init_vault(&self) -> anyhow::Result<()> {
        let secret = self.cluster_secret.read().unwrap().clone();
        if secret.is_empty() {
            return Ok(());
        }

        let tree = self.db.open_tree("vault_meta")?;

        // C-1: load or generate a random per-instance master salt (never hardcoded)
        let salt: Vec<u8> = if let Some(s) = tree.get("master_salt")? {
            s.to_vec()
        } else {
            let mut s = [0u8; 32];
            rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut s);
            tree.insert("master_salt", &s[..])?;
            s.to_vec()
        };

        let master_key = crypto::derive_key_simple(&secret, &salt)?;
        
        if let Some(kek_dek_bytes) = tree.get("kek_dek")? {
            if let Ok(kek_dek) = serde_json::from_slice::<serde_json::Value>(&kek_dek_bytes) {
                if kek_dek.get("config_id").is_none() {
                    info!("Migrating legacy default Vault config");
                    let default_config = VaultConfig {
                        id: "default".to_string(),
                        name: "Default".to_string(),
                        created_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
                    };
                    
                    let mut legacy_kek_dek: KekDek = serde_json::from_slice(&kek_dek_bytes)?;
                    legacy_kek_dek.config_id = "default".to_string();
                    
                    let id = default_config.id.clone();
                    let value = serde_json::to_vec(&default_config)?;
                    self.db.open_tree("vault_configs")?.insert(id.as_bytes(), value)?;
                    
                    tree.insert("kek_dek_default", serde_json::to_vec(&legacy_kek_dek)?)?;
                    tree.remove("kek_dek")?;
                }
            }
        }

        // Legacy hardcoded salt used before C-1 fix — needed for one-time migration only
        let legacy_master_key = crypto::derive_key_simple(&secret, b"r4a-master-salt-v1").ok();

        let mut keys = std::collections::HashMap::new();
        let mut to_reencrypt: Vec<(sled::IVec, KekDek)> = Vec::new();

        for item in tree.iter() {
            if let Ok((k, v)) = item {
                let key_str = String::from_utf8_lossy(&k);
                if !key_str.starts_with("kek_dek_") {
                    continue;
                }
                let config_id = key_str.trim_start_matches("kek_dek_").to_string();
                let kek_dek = match serde_json::from_slice::<KekDek>(&v) {
                    Ok(kd) => kd,
                    Err(_) => continue,
                };

                // Try current (random) master key first
                let dek_bytes = match crypto::decrypt(&master_key, &kek_dek.encrypted_dek, &kek_dek.nonce) {
                    Ok(b) => b,
                    Err(_) => {
                        // Migration path: try legacy hardcoded-salt key
                        match legacy_master_key.as_ref().and_then(|lk| {
                            crypto::decrypt(lk, &kek_dek.encrypted_dek, &kek_dek.nonce).ok()
                        }) {
                            Some(b) => {
                                info!("Vault config '{}': re-encrypting DEK with random salt (one-time migration)", config_id);
                                to_reencrypt.push((k, kek_dek));
                                b
                            }
                            None => {
                                error!("Vault config '{}': cannot decrypt DEK with any key — skipping", config_id);
                                continue;
                            }
                        }
                    }
                };

                let mut dek = [0u8; 32];
                dek.copy_from_slice(&dek_bytes);
                keys.insert(config_id, dek);
            }
        }

        // Re-encrypt migrated DEKs under the new random master key
        for (tree_key, old_kek_dek) in to_reencrypt {
            let config_id = old_kek_dek.config_id.clone();
            if let Some(dek) = keys.get(&config_id) {
                match crypto::encrypt(&master_key, dek) {
                    Ok((encrypted_dek, nonce)) => {
                        let new_kek_dek = KekDek { config_id, encrypted_dek, nonce };
                        if let Ok(bytes) = serde_json::to_vec(&new_kek_dek) {
                            let _ = tree.insert(tree_key, bytes);
                        }
                    }
                    Err(e) => error!("Failed to re-encrypt DEK during migration: {}", e),
                }
            }
        }

        if keys.is_empty() {
            info!("Creating default Vault config");
            let default_config = VaultConfig {
                id: "default".to_string(),
                name: "Default".to_string(),
                created_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
            };
            self.create_vault_config_internal(&master_key, default_config, &mut keys)?;
        }

        let mut w = self.vault_keys.write().unwrap();
        *w = keys;
        
        info!("Vault initialized successfully");
        Ok(())
    }

    async fn put_vault_config(&self, config: VaultConfig) -> anyhow::Result<()> {
        let id = config.id.clone();
        let value = serde_json::to_vec(&config)?;
        self.put("vault_configs", id.as_bytes(), &value).await
    }

    pub fn get_vault_configs(&self) -> anyhow::Result<Vec<VaultConfig>> {
        let mut configs = vec![];
        let tree = self.db.open_tree("vault_configs")?;
        for item in tree.iter() {
            if let Ok((_, v)) = item {
                if let Ok(config) = serde_json::from_slice::<VaultConfig>(&v) {
                    configs.push(config);
                }
            }
        }
        Ok(configs)
    }

    pub async fn create_vault_config(&self, name: String) -> anyhow::Result<VaultConfig> {
        let secret = self.cluster_secret.read().unwrap().clone();
        let tree_meta = self.db.open_tree("vault_meta")?;
        let salt: Vec<u8> = tree_meta.get("master_salt")?
            .map(|s| s.to_vec())
            .ok_or_else(|| anyhow::anyhow!("Vault not initialized: master_salt missing"))?;
        let master_key = crypto::derive_key_simple(&secret, &salt)?;

        let id = format!("vault-{}", uuid::Uuid::new_v4());
        let config = VaultConfig {
            id: id.clone(),
            name,
            created_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
        };

        let mut keys = self.vault_keys.read().unwrap().clone();
        self.create_vault_config_internal(&master_key, config.clone(), &mut keys)?;
        
        {
            let mut w = self.vault_keys.write().unwrap();
            *w = keys;
        }

        self.put_vault_config(config.clone()).await?;
        Ok(config)
    }

    fn create_vault_config_internal(&self, master_key: &[u8], config: VaultConfig, keys: &mut std::collections::HashMap<String, [u8; 32]>) -> anyhow::Result<()> {
        let mut dek = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut dek);
        
        let master_key_32: &[u8; 32] = master_key.try_into().map_err(|_| anyhow::anyhow!("Master key must be 32 bytes"))?;
        let (encrypted_dek, nonce) = crypto::encrypt(master_key_32, &dek)?;
        
        let kek_dek = KekDek {
            config_id: config.id.clone(),
            encrypted_dek,
            nonce,
        };
        
        let tree_meta = self.db.open_tree("vault_meta")?;
        let key_meta = format!("kek_dek_{}", config.id);
        tree_meta.insert(key_meta.as_bytes(), serde_json::to_vec(&kek_dek)?)?;
        
        let tree_configs = self.db.open_tree("vault_configs")?;
        tree_configs.insert(config.id.as_bytes(), serde_json::to_vec(&config)?)?;
        
        keys.insert(config.id, dek);
        Ok(())
    }

    pub async fn put_vault_secret(&self, secret: VaultSecret) -> anyhow::Result<()> {
        let full_key = format!("{}/{}", secret.config_id, secret.key);
        let value = serde_json::to_vec(&secret)?;
        self.put("vault", full_key.as_bytes(), &value).await
    }

    pub fn get_vault_secret(&self, config_id: &str, key: &str) -> anyhow::Result<Option<VaultSecret>> {
        let full_key = format!("{}/{}", config_id, key);
        if let Some(bytes) = self.get("vault", full_key.as_bytes())? {
            let secret: VaultSecret = serde_json::from_slice(&bytes)?;
            Ok(Some(secret))
        } else {
            Ok(None)
        }
    }

    pub fn list_vault_secrets(&self, config_id: &str) -> anyhow::Result<Vec<String>> {
        let mut keys = vec![];
        let tree = self.db.open_tree("vault")?;
        let prefix = format!("{}/", config_id);
        for item in tree.iter() {
            if let Ok((k, _)) = item {
                let key_str = String::from_utf8_lossy(&k);
                if key_str.starts_with(&prefix) {
                    keys.push(key_str.trim_start_matches(&prefix).to_string());
                }
            }
        }
        Ok(keys)
    }

    pub async fn delete_vault_secret(&self, config_id: &str, key: &str) -> anyhow::Result<()> {
        let full_key = format!("{}/{}", config_id, key);
        self.delete("vault", full_key.as_bytes()).await
    }

    pub fn decrypt_secret(&self, secret: &VaultSecret) -> anyhow::Result<String> {
        let keys = self.vault_keys.read().unwrap();
        let dek = keys.get(&secret.config_id).ok_or_else(|| anyhow::anyhow!("Vault config {} not found or not initialized", secret.config_id))?;
        let decrypted = crypto::decrypt(dek, &secret.encrypted_value, &secret.nonce)?;
        Ok(String::from_utf8(decrypted)?)
    }

    pub async fn put_manifest(&self, manifest: &Manifest) -> anyhow::Result<()> {
        let key = manifest.app.name.as_bytes().to_vec();
        let value = serde_json::to_vec(manifest)?;
        self.put("manifests", &key, &value).await
    }

    pub fn list_manifests(&self) -> anyhow::Result<Vec<Manifest>> {
        let mut manifests = vec![];
        let tree = self.db.open_tree("manifests")?;
        for item in tree.iter() {
            if let Ok((_, v)) = item {
                if let Ok(m) = serde_json::from_slice::<Manifest>(&v) {
                    manifests.push(m);
                }
            }
        }
        Ok(manifests)
    }

    pub async fn delete_manifest(&self, name: &str) -> anyhow::Result<()> {
        self.delete("manifests", name.as_bytes()).await
    }

    pub async fn put_token(&self, token: Token) -> anyhow::Result<()> {
        let id = token.id.clone();
        let value = serde_json::to_vec(&token)?;
        self.put("tokens", id.as_bytes(), &value).await
    }

    pub fn get_token(&self, id: &str) -> anyhow::Result<Option<Token>> {
        if let Some(bytes) = self.get("tokens", id.as_bytes())? {
            let token: Token = serde_json::from_slice(&bytes)?;
            Ok(Some(token))
        } else {
            Ok(None)
        }
    }

    pub async fn put(&self, tree_name: &str, key: &[u8], value: &[u8]) -> anyhow::Result<()> {
        let tree = self.db.open_tree(tree_name)?;
        tree.insert(key, value)?;
        self.db.flush_async().await?;

        self.broadcast_sync(SyncRequest {
            tree: tree_name.to_string(),
            key: key.to_vec(),
            value: value.to_vec(),
            delete: false,
        });
        Ok(())
    }

    fn broadcast_sync(&self, req: SyncRequest) {
        let masters = self.masters.read().unwrap().clone();
        let secret = self.cluster_secret.read().unwrap().clone();
        for master_ip in masters {
            let req = req.clone();
            let secret = secret.clone();
            tokio::spawn(async move {
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(2))
                    .build()
                    .unwrap_or_default();
                
                let target = format!("http://{master_ip}:3501/api/store/sync");
                if let Err(e) = client.post(&target)
                    .header("X-R4A-Secret", &secret)
                    .json(&req)
                    .send()
                    .await {
                    debug!("Store sync to {} failed: {}", master_ip, e);
                }
            });
        }
    }

    pub fn get(&self, tree_name: &str, key: &[u8]) -> anyhow::Result<Option<sled::IVec>> {
        let tree = self.db.open_tree(tree_name)?;
        Ok(tree.get(key)?)
    }

    pub async fn delete(&self, tree_name: &str, key: &[u8]) -> anyhow::Result<()> {
        let tree = self.db.open_tree(tree_name)?;
        tree.remove(key)?;
        self.db.flush_async().await?;

        self.broadcast_sync(SyncRequest {
            tree: tree_name.to_string(),
            key: key.to_vec(),
            value: Vec::new(),
            delete: true,
        });
        Ok(())
    }

    pub async fn put_policy(&self, policy: Policy) -> anyhow::Result<()> {
        let id = policy.id.clone();
        let value = serde_json::to_vec(&policy)?;
        self.put("policies", id.as_bytes(), &value).await
    }

    pub fn get_policy(&self, id: &str) -> anyhow::Result<Option<Policy>> {
        if let Some(bytes) = self.get("policies", id.as_bytes())? {
            let policy: Policy = serde_json::from_slice(&bytes)?;
            Ok(Some(policy))
        } else {
            Ok(None)
        }
    }

    pub async fn put_binding(&self, binding: Binding) -> anyhow::Result<()> {
        let id = binding.id.clone();
        let value = serde_json::to_vec(&binding)?;
        self.put("bindings", id.as_bytes(), &value).await
    }

    pub fn get_binding(&self, id: &str) -> anyhow::Result<Option<Binding>> {
        if let Some(bytes) = self.get("bindings", id.as_bytes())? {
            let binding: Binding = serde_json::from_slice(&bytes)?;
            Ok(Some(binding))
        } else {
            Ok(None)
        }
    }

    pub fn get_bindings_for_subject(&self, subject: &str) -> anyhow::Result<Vec<Binding>> {
        let mut bindings = vec![];
        let tree = self.db.open_tree("bindings")?;
        for item in tree.iter() {
            if let Ok((_, v)) = item {
                if let Ok(binding) = serde_json::from_slice::<Binding>(&v) {
                    if binding.subject == subject {
                        bindings.push(binding);
                    }
                }
            }
        }
        Ok(bindings)
    }

    pub fn get_all_bindings(&self) -> anyhow::Result<Vec<Binding>> {
        let mut bindings = vec![];
        let tree = self.db.open_tree("bindings")?;
        for item in tree.iter() {
            if let Ok((_, v)) = item {
                if let Ok(binding) = serde_json::from_slice::<Binding>(&v) {
                    bindings.push(binding);
                }
            }
        }
        Ok(bindings)
    }

    pub fn can(&self, subject: &str, verb: Verb, resource: Resource, resource_name: Option<&str>) -> bool {
        let bindings = match self.get_bindings_for_subject(subject) {
            Ok(b) => b,
            Err(_) => return false,
        };

        for binding in bindings {
            if let Ok(Some(policy)) = self.get_policy(&binding.policy_id) {
                for rule in &policy.rules {
                    let verb_match = rule.verbs.contains(&verb) || rule.verbs.contains(&Verb::All);
                    let res_match = rule.resources.contains(&resource) || rule.resources.contains(&Resource::All);
                    if !verb_match || !res_match {
                        continue;
                    }
                    if let Some(ref names) = rule.resource_names {
                        if let Some(name) = resource_name {
                            // Exact match only; prefix match requires an explicit trailing '*'
                            if names.iter().any(|n| match n.strip_suffix('*') {
                                Some(prefix) => name.starts_with(prefix),
                                None => n == name,
                            }) {
                                return true;
                            }
                        }
                    } else {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn get_policies(&self) -> anyhow::Result<Vec<Policy>> {
        let mut policies = vec![];
        let tree = self.db.open_tree("policies")?;
        for item in tree.iter() {
            if let Ok((_, v)) = item {
                if let Ok(policy) = serde_json::from_slice::<Policy>(&v) {
                    policies.push(policy);
                }
            }
        }
        Ok(policies)
    }

    pub fn migrate_rbac_v1_to_v2(&self) -> anyhow::Result<()> {
        let tokens_tree = self.db.open_tree("tokens")?;
        let mut legacy_count = 0;
        let mut seen_subjects: std::collections::HashSet<String> = std::collections::HashSet::new();

        for item in tokens_tree.iter() {
            if let Ok((k, v)) = item {
                if let Ok(legacy) = serde_json::from_value::<serde_json::Value>(serde_json::from_slice(&v)?) {
                    let has_role = legacy.get("role").is_some();
                    let has_permissions = legacy.get("permissions").is_some();
                    if !has_role && !has_permissions {
                        continue;
                    }

                    let username = legacy.get("username").and_then(|u| u.as_str()).unwrap_or("unknown");
                    let id = legacy.get("id").and_then(|i| i.as_str()).unwrap_or("");
                    let created_at = legacy.get("created_at").and_then(|t| t.as_u64()).unwrap_or(0);

                    let is_admin = legacy.get("role").and_then(|r| r.as_str()) == Some("admin");

                    let policy_id = format!("policy-{username}");
                    let binding_id = format!("binding-{username}");

                    if is_admin {
                        if !seen_subjects.insert(username.to_string()) {
                            let _ = tokens_tree.remove(k);
                            continue;
                        }
                        let admin_policy = Policy {
                            id: policy_id.clone(),
                            rules: vec![Rule {
                                verbs: vec![Verb::All],
                                resources: vec![Resource::All],
                                resource_names: None,
                            }],
                        };
                        self.db.open_tree("policies")?.insert(policy_id.as_bytes(), serde_json::to_vec(&admin_policy)?)?;
                    } else {
                        let perms: Vec<String> = legacy.get("permissions")
                            .and_then(|p| p.as_array())
                            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                            .unwrap_or_default();

                        let mut resource_names = Vec::new();
                        for perm in &perms {
                            if perm == "*" || perm == "global/*" {
                                resource_names.clear();
                                break;
                            }
                            // Keep the trailing '*' — can() treats it as an explicit prefix match
                            resource_names.push(perm.clone());
                        }

                        if resource_names.is_empty() || perms.contains(&"*".to_string()) || perms.contains(&"global/*".to_string()) {
                            let agent_policy = Policy {
                                id: policy_id.clone(),
                                rules: vec![Rule {
                                    verbs: vec![Verb::Get],
                                    resources: vec![Resource::All],
                                    resource_names: None,
                                }],
                            };
                            self.db.open_tree("policies")?.insert(policy_id.as_bytes(), serde_json::to_vec(&agent_policy)?)?;
                        } else {
                            let agent_policy = Policy {
                                id: policy_id.clone(),
                                rules: vec![Rule {
                                    verbs: vec![Verb::Get],
                                    resources: vec![Resource::Vault],
                                    resource_names: Some(resource_names),
                                }],
                            };
                            self.db.open_tree("policies")?.insert(policy_id.as_bytes(), serde_json::to_vec(&agent_policy)?)?;
                        }
                    }

                    let binding = Binding {
                        id: binding_id.clone(),
                        subject: username.to_string(),
                        policy_id,
                    };
                    self.db.open_tree("bindings")?.insert(binding_id.as_bytes(), serde_json::to_vec(&binding)?)?;

                    let new_token = Token {
                        id: id.to_string(),
                        username: username.to_string(),
                        created_at,
                    };
                    tokens_tree.insert(k, serde_json::to_vec(&new_token)?)?;
                    legacy_count += 1;
                }
            }
        }

        if legacy_count > 0 {
            info!("Migrated {} legacy RBAC token(s) to Policy/Binding system", legacy_count);
        }
        Ok(())
    }

    pub async fn put_connection(&self, conn: &Connection) -> anyhow::Result<()> {
        let value = serde_json::to_vec(conn)?;
        let tree = self.db.open_tree("connections")?;
        tree.insert(conn.id.as_bytes(), value)?;
        self.db.flush_async().await?;
        Ok(())
    }

    pub fn get_connection(&self, id: &str) -> anyhow::Result<Option<Connection>> {
        let tree = self.db.open_tree("connections")?;
        if let Some(bytes) = tree.get(id.as_bytes())? {
            Ok(Some(serde_json::from_slice(&bytes)?))
        } else {
            Ok(None)
        }
    }

    pub fn list_connections(&self) -> anyhow::Result<Vec<Connection>> {
        let tree = self.db.open_tree("connections")?;
        let mut out = vec![];
        for item in tree.iter() {
            if let Ok((_, v)) = item {
                if let Ok(c) = serde_json::from_slice::<Connection>(&v) {
                    out.push(c);
                }
            }
        }
        Ok(out)
    }

    pub async fn delete_connection(&self, id: &str) -> anyhow::Result<()> {
        let tree = self.db.open_tree("connections")?;
        tree.remove(id.as_bytes())?;
        self.db.flush_async().await?;
        Ok(())
    }

    pub async fn update_connection_heartbeat(&self, id: &str, now: u64) -> anyhow::Result<bool> {
        let tree = self.db.open_tree("connections")?;
        if let Some(bytes) = tree.get(id.as_bytes())? {
            let mut conn: Connection = serde_json::from_slice(&bytes)?;
            conn.last_seen = now;
            tree.insert(id.as_bytes(), serde_json::to_vec(&conn)?)?;
            self.db.flush_async().await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Returns the pinned VPN IP for a label, if one was assigned before.
    pub fn get_label_ip(&self, label: &str) -> anyhow::Result<Option<String>> {
        let tree = self.db.open_tree("connection_labels")?;
        Ok(tree.get(label.as_bytes())?.map(|v| String::from_utf8_lossy(&v).to_string()))
    }

    /// Pins label → vpn_ip permanently (survives disconnects).
    pub async fn set_label_ip(&self, label: &str, vpn_ip: &str) -> anyhow::Result<()> {
        let tree = self.db.open_tree("connection_labels")?;
        tree.insert(label.as_bytes(), vpn_ip.as_bytes())?;
        self.db.flush_async().await?;
        Ok(())
    }
}

pub fn store_router(store: Store) -> Router {
    Router::new()
        .route("/api/store/sync", post(sync_handler))
        .with_state(store)
}

// C-3: only these trees may be replicated between masters
const ALLOWED_SYNC_TREES: &[&str] = &[
    "tokens", "bindings", "policies", "manifests", "vault", "vault_configs",
];

async fn sync_handler(
    headers: HeaderMap,
    State(store): State<Store>,
    Json(req): Json<SyncRequest>
) -> StatusCode {
    if let Some(auth_header) = headers.get("X-R4A-Secret") {
        if let Ok(auth_str) = auth_header.to_str() {
            let is_auth = {
                let secret = store.cluster_secret.read().unwrap();
                !secret.is_empty() && constant_time_eq::constant_time_eq(auth_str.as_bytes(), secret.as_bytes())
            };

            if is_auth {
                if !ALLOWED_SYNC_TREES.contains(&req.tree.as_str()) {
                    error!("Sync rejected: tree '{}' is not in the allowed list", req.tree);
                    return StatusCode::FORBIDDEN;
                }
                if let Ok(tree) = store.db.open_tree(&req.tree) {
                    let result = if req.delete {
                        tree.remove(req.key).map(|_| ())
                    } else {
                        tree.insert(req.key, req.value).map(|_| ())
                    };
                    if let Err(e) = result {
                        error!("Sync write failed: {}", e);
                        return StatusCode::INTERNAL_SERVER_ERROR;
                    }
                    let _ = store.db.flush_async().await;
                }
                return StatusCode::OK;
            }
        }
    }
    StatusCode::UNAUTHORIZED
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store_with_names(names: Vec<String>) -> Store {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("db")).unwrap();
        store
            .put_policy(Policy {
                id: "p1".to_string(),
                rules: vec![Rule {
                    verbs: vec![Verb::Get],
                    resources: vec![Resource::Vault],
                    resource_names: Some(names),
                }],
            })
            .await
            .unwrap();
        store
            .put_binding(Binding {
                id: "b1".to_string(),
                subject: "alice".to_string(),
                policy_id: "p1".to_string(),
            })
            .await
            .unwrap();
        std::mem::forget(dir);
        store
    }

    #[tokio::test]
    async fn can_matches_resource_names_exactly() {
        let store = store_with_names(vec!["db-pass".to_string()]).await;
        assert!(store.can("alice", Verb::Get, Resource::Vault, Some("db-pass")));
        // no implicit prefix match
        assert!(!store.can("alice", Verb::Get, Resource::Vault, Some("db-pass-prod")));
        assert!(!store.can("alice", Verb::Get, Resource::Vault, Some("db")));
    }

    #[tokio::test]
    async fn can_matches_explicit_wildcard_prefix() {
        let store = store_with_names(vec!["prod-*".to_string()]).await;
        assert!(store.can("alice", Verb::Get, Resource::Vault, Some("prod-db")));
        assert!(store.can("alice", Verb::Get, Resource::Vault, Some("prod-")));
        assert!(!store.can("alice", Verb::Get, Resource::Vault, Some("staging-db")));
    }
}
