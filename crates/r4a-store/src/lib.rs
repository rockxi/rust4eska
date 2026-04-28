use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use serde::{Deserialize, Serialize};
use sled::Db;
use std::{path::Path, sync::Arc};
use tracing::{debug, error};

#[derive(Clone)]
pub struct Store {
    pub db: Db,
    // VPN IP других мастеров (например, "10.42.0.1", "10.42.0.2")
    pub masters: Arc<std::sync::RwLock<Vec<String>>>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SyncRequest {
    pub tree: String,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

impl Store {
    pub fn open<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let db = sled::open(path)?;
        Ok(Self {
            db,
            masters: Arc::new(std::sync::RwLock::new(Vec::new())),
        })
    }

    pub fn set_masters(&self, master_ips: Vec<String>) {
        let mut w = self.masters.write().unwrap();
        *w = master_ips;
    }

    pub async fn put(&self, tree_name: &str, key: &[u8], value: &[u8]) -> anyhow::Result<()> {
        let tree = self.db.open_tree(tree_name)?;
        tree.insert(key, value)?;
        self.db.flush_async().await?;

        let req = SyncRequest {
            tree: tree_name.to_string(),
            key: key.to_vec(),
            value: value.to_vec(),
        };

        let masters = self.masters.read().unwrap().clone();
        for master_ip in masters {
            let req = req.clone();
            tokio::spawn(async move {
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(2))
                    .build()
                    .unwrap_or_default();
                
                let target = format!("http://{master_ip}:8080/api/store/sync");
                if let Err(e) = client.post(&target).json(&req).send().await {
                    debug!("Store sync to {} failed: {}", master_ip, e);
                }
            });
        }
        Ok(())
    }

    pub fn get(&self, tree_name: &str, key: &[u8]) -> anyhow::Result<Option<sled::IVec>> {
        let tree = self.db.open_tree(tree_name)?;
        Ok(tree.get(key)?)
    }

    pub async fn delete(&self, tree_name: &str, key: &[u8]) -> anyhow::Result<()> {
        let tree = self.db.open_tree(tree_name)?;
        tree.remove(key)?;
        self.db.flush_async().await?;
        Ok(())
    }
}

pub fn store_router(store: Store) -> Router {
    Router::new()
        .route("/api/store/sync", post(sync_handler))
        .with_state(store)
}

async fn sync_handler(State(store): State<Store>, Json(req): Json<SyncRequest>) -> StatusCode {
    if let Ok(tree) = store.db.open_tree(&req.tree) {
        if let Err(e) = tree.insert(req.key, req.value) {
            error!("Sync write failed: {}", e);
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
        let _ = store.db.flush();
    }
    StatusCode::OK
}
