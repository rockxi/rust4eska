use async_trait::async_trait;
use pingora::prelude::*;
use r4a_store::Store;
use r4a_core::Manifest;
use std::collections::HashMap;

pub struct IngressProxy {
    pub store: Store,
}

#[async_trait]
impl ProxyHttp for IngressProxy {
    type CTX = ();
    fn new_ctx(&self) -> Self::CTX {}

    async fn upstream_peer(&self, session: &mut Session, _ctx: &mut ()) -> Result<Box<HttpPeer>> {
        let host = session.get_header("Host").and_then(|h| h.to_str().ok()).unwrap_or("");
        
        if let Ok(Some(data)) = self.store.get("core", b"manifests") {
            if let Ok(manifests) = serde_json::from_slice::<HashMap<String, Manifest>>(&data) {
                for manifest in manifests.values() {
                    if host.starts_with(&manifest.app.name) {
                        if let Ok(Some(nodes_data)) = self.store.get("core", b"peers") {
                            if let Ok(peers) = serde_json::from_slice::<HashMap<String, r4a_core::PeerInfo>>(&nodes_data) {
                                for peer in peers.values() {
                                    if peer.name == manifest.app.node_selector || manifest.app.node_selector == "all" {
                                        let target_addr = format!("{}:8080", peer.ip);
                                        return Ok(Box::new(HttpPeer::new(
                                            target_addr,
                                            false,
                                            "".to_string()
                                        )));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Err(Error::explain(ErrorType::HTTPStatus(404), "App not found or no node available"))
    }
}
