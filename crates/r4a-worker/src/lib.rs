use anyhow::Result;
use bollard::container::{Config, CreateContainerOptions, StartContainerOptions};
use bollard::models::{HostConfig, PortBinding};
use bollard::Docker;
use r4a_core::Manifest;
use std::collections::HashMap;
use tracing::{info, error, warn};

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
        
        let containers = match self.docker.list_containers::<String>(None).await {
            Ok(c) => c,
            Err(e) => {
                error!("Failed to list containers: {}", e);
                return Err(e.into());
            }
        };

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

        for (name, manifest) in &manifests {
            let container_name = format!("r4a-{}", name);
            if let Some(container_config) = &manifest.container {
                if !managed_containers.contains_key(&container_name) {
                    info!("Starting container: {}", container_name);
                    if let Err(e) = self.start_container(&container_name, container_config, &manifest.env).await {
                        error!("Failed to start container {}: {}", container_name, e);
                    }
                }
                managed_containers.remove(&container_name);
            }

            if let Some(systemd_config) = &manifest.systemd {
                let service_name = format!("r4a-{}", name);
                info!("Ensuring systemd service: {}", service_name);
                let _ = self.service_manager.enable(&service_name, &format!("r4a managed {}", name), &systemd_config.exec);
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

    async fn start_container(&self, name: &str, config: &r4a_core::ContainerConfig, env: &HashMap<String, String>) -> Result<()> {
        use futures_util::stream::StreamExt;
        
        let mut pull_stream = self.docker.create_image(
            Some(bollard::image::CreateImageOptions {
                from_image: config.image.clone(),
                ..Default::default()
            }),
            None,
            None,
        );
        while let Some(pull_result) = pull_stream.next().await {
            if let Err(e) = pull_result {
                warn!("Pulling {} status: {}", config.image, e);
            }
        }

        let env_list: Vec<String> = env.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
        
        let mut port_bindings = HashMap::new();
        if let Some(ports) = &config.ports {
            for port_mapping in ports {
                let parts: Vec<&str> = port_mapping.split(':').collect();
                if parts.len() == 2 {
                    let host_port = parts[0];
                    let container_port = parts[1];
                    let mut bindings = Vec::new();
                    bindings.push(PortBinding {
                        host_ip: Some("0.0.0.0".to_string()),
                        host_port: Some(host_port.to_string()),
                    });
                    port_bindings.insert(format!("{}/tcp", container_port), Some(bindings));
                }
            }
        }

        let host_config = HostConfig {
            port_bindings: Some(port_bindings),
            restart_policy: Some(bollard::models::RestartPolicy {
                name: Some(bollard::models::RestartPolicyNameEnum::ALWAYS),
                ..Default::default()
            }),
            ..Default::default()
        };

        let docker_config = Config {
            image: Some(config.image.clone()),
            env: Some(env_list),
            cmd: config.command.clone(),
            host_config: Some(host_config),
            ..Default::default()
        };

        if let Err(e) = self.docker.create_container(
            Some(CreateContainerOptions { name: name.to_string(), ..Default::default() }),
            docker_config.clone(),
        ).await {
            if let bollard::errors::Error::DockerResponseServerError { status_code: 409, .. } = e {
                info!("Container {} already exists, recreating...", name);
            } else {
                return Err(e.into());
            }
        }

        if let Err(e) = self.docker.start_container(name, None::<StartContainerOptions<String>>).await {
             if let bollard::errors::Error::DockerResponseServerError { status_code: 304, .. } = e {
             } else {
                 let _ = self.docker.stop_container(name, None).await;
                 let _ = self.docker.remove_container(name, None).await;
                 
                 self.docker.create_container(
                    Some(CreateContainerOptions { name: name.to_string(), ..Default::default() }),
                    docker_config,
                ).await?;
                self.docker.start_container(name, None::<StartContainerOptions<String>>).await?;
             }
        }
        
        info!("Container {} started successfully", name);
        Ok(())
    }
}
