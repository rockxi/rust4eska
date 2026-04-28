use anyhow::Result;
use bollard::container::{Config, CreateContainerOptions, StartContainerOptions};
use bollard::Docker;
use r4a_core::Manifest;
use std::collections::HashMap;
use tracing::{info, error};

pub struct Reconciler {
    docker: Docker,
    service_manager: r4a_service::ServiceManager,
    _node_name: String,
}

impl Reconciler {
    pub fn new(node_name: String) -> Result<Self> {
        let docker = Docker::connect_with_local_defaults()?;
        let service_manager = r4a_service::ServiceManager::detect()?;
        Ok(Self { docker, service_manager, _node_name: node_name })
    }

    pub async fn reconcile(&self, manifests: HashMap<String, Manifest>) -> Result<()> {
        info!("Reconciling {} manifests...", manifests.len());
        
        let containers = self.docker.list_containers::<String>(None).await?;
        let mut managed_containers = HashMap::new();
        for c in containers {
            if let Some(names) = c.names {
                for name in names {
                    let name = name.trim_start_matches('/');
                    if name.starts_with("r4a-") {
                        managed_containers.insert(name.to_string(), c.id.clone());
                    }
                }
            }
        }

        let mut managed_services = Vec::new();

        for (name, manifest) in &manifests {
            let container_name = format!("r4a-{}", name);
            if let Some(container_config) = &manifest.container {
                if !managed_containers.contains_key(&container_name) {
                    info!("Starting container: {}", container_name);
                    self.start_container(&container_name, &container_config.image, &manifest.env, &container_config.command).await?;
                }
                managed_containers.remove(&container_name);
            }

            if let Some(systemd_config) = &manifest.systemd {
                let service_name = format!("r4a-{}", name);
                info!("Ensuring systemd service: {}", service_name);
                let _ = self.service_manager.enable(&service_name, &format!("r4a managed {}", name), &systemd_config.exec);
                managed_services.push(service_name);
            }
        }

        for (name, id) in managed_containers {
            if let Some(id) = id {
                info!("Stopping orphaned container: {} ({})", name, id);
                let _ = self.docker.stop_container(&id, None).await;
                let _ = self.docker.remove_container(&id, None).await;
            }
        }

        Ok(())
    }

    async fn start_container(&self, name: &str, image: &str, env: &HashMap<String, String>, command: &Option<Vec<String>>) -> Result<()> {
        use futures_util::stream::StreamExt;
        let mut pull_stream = self.docker.create_image(
            Some(bollard::image::CreateImageOptions {
                from_image: image.to_string(),
                ..Default::default()
            }),
            None,
            None,
        );
        while let Some(pull_result) = pull_stream.next().await {
            if let Err(e) = pull_result {
                error!("Pull error for {}: {}", image, e);
            }
        }

        let env_list: Vec<String> = env.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
        
        let config = Config {
            image: Some(image.to_string()),
            env: Some(env_list),
            cmd: command.clone(),
            ..Default::default()
        };

        self.docker.create_container(
            Some(CreateContainerOptions { name: name.to_string(), ..Default::default() }),
            config,
        ).await?;

        self.docker.start_container(name, None::<StartContainerOptions<String>>).await?;
        
        Ok(())
    }
}
