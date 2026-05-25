use async_trait::async_trait;
use pingora::prelude::*;
use r4a_store::Store;
use r4a_core::PeerInfo;
use std::collections::HashMap;
use tracing::warn;

pub struct IngressProxy {
    pub store: Store,
}

#[async_trait]
impl ProxyHttp for IngressProxy {
    type CTX = ();
    fn new_ctx(&self) -> Self::CTX {}

    async fn upstream_peer(&self, session: &mut Session, _ctx: &mut ()) -> Result<Box<HttpPeer>> {
        let host_header = session
            .get_header("Host")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");

        // Strip port from Host header (e.g. "app.cluster.local:8000" → "app.cluster.local")
        let host = host_header.split(':').next().unwrap_or("");

        let manifests = match self.store.list_manifests() {
            Ok(m) => m,
            Err(e) => {
                warn!("Ingress: failed to load manifests: {}", e);
                return Err(Error::explain(ErrorType::HTTPStatus(503), "failed to load manifests"));
            }
        };

        let manifest = manifests.into_iter().find(|m| {
            m.ingress.as_ref().map(|ing| ing.domain == host).unwrap_or(false)
        });

        let manifest = match manifest {
            Some(m) => m,
            None => {
                return Err(Error::explain(ErrorType::HTTPStatus(404), "no app for this domain"));
            }
        };

        let ingress_cfg = manifest.ingress.as_ref().unwrap();
        let node_selector = &manifest.app.node_selector;
        let container_port = ingress_cfg.container_port;

        let peers: HashMap<String, PeerInfo> = self
            .store
            .get("core", b"peers")
            .ok()
            .flatten()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();

        let peer = peers.values().find(|p| {
            p.name == *node_selector || node_selector == "all"
        });

        let peer = match peer {
            Some(p) => p,
            None => {
                warn!("Ingress: no node found for selector={}", node_selector);
                return Err(Error::explain(ErrorType::HTTPStatus(503), "no node available for this app"));
            }
        };

        let target = format!("{}:{}", peer.ip, container_port);
        Ok(Box::new(HttpPeer::new(target, false, host.to_string())))
    }
}
