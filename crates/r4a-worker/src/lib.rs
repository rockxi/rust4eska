use anyhow::Result;
use bollard::container::{Config, CreateContainerOptions, StartContainerOptions};
use bollard::models::{HostConfig, PortBinding};
use bollard::Docker;
use r4a_core::Manifest;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info, warn};

pub struct Reconciler {
    docker: Docker,
    service_manager: r4a_service::ServiceManager,
    node_name: String,
    agent_token: Arc<std::sync::RwLock<String>>,
}

impl Reconciler {
    pub fn new(node_name: String, agent_token: String) -> Result<Self> {
        let docker = Docker::connect_with_local_defaults()?;
        let service_manager = r4a_service::ServiceManager::detect()?;
        Ok(Self {
            docker,
            service_manager,
            node_name,
            agent_token: Arc::new(std::sync::RwLock::new(agent_token)),
        })
    }

    pub fn set_token(&self, token: String) {
        let mut w = self.agent_token.write().unwrap();
        *w = token;
    }

    pub async fn reconcile(&self, manifests: HashMap<String, Manifest>) -> Result<()> {
        info!("Reconciling {} manifests...", manifests.len());

        let mut filters = HashMap::new();
        filters.insert(
            "label".to_string(),
            vec![format!("r4a.node={}", self.node_name)],
        );

        let containers = match self
            .docker
            .list_containers(Some(bollard::container::ListContainersOptions {
                all: true,
                filters,
                ..Default::default()
            }))
            .await
        {
            Ok(c) => c,
            Err(e) => {
                error!("Failed to list containers: {}", e);
                return Err(e.into());
            }
        };

        let mut managed_containers = HashMap::new();
        for c in containers {
            if let Some(names) = &c.names {
                for name in names {
                    let name = name.trim_start_matches('/');
                    managed_containers.insert(name.to_string(), c.id.clone());
                }
            }
        }

        for (name, manifest) in &manifests {
            // When node_selector="all", multiple agents share the same Docker socket in dev,
            // so we namespace the container name to avoid agents fighting over the same container.
            let container_name = if manifest.app.node_selector == "all" {
                format!("r4a-{}-{}", name, self.node_name)
            } else {
                format!("r4a-{}", name)
            };
            if let Some(container_config) = &manifest.container {
                let resolved_env = self.resolve_env(&manifest.env).await;

                let mut should_start = false;
                if let Some(id_opt) = managed_containers.get(&container_name) {
                    if let Some(id) = id_opt {
                        // Проверяем, не изменился ли конфиг или секреты
                        if let Ok(inspect) = self.docker.inspect_container(id, None).await {
                            if let Some(config) = inspect.config {
                                let mut env_changed = false;
                                if let Some(existing_env) = config.env {
                                    for (k, v) in &resolved_env {
                                        let env_str = format!("{}={}", k, v);
                                        if !existing_env.contains(&env_str) {
                                            env_changed = true;
                                            break;
                                        }
                                    }
                                } else if !resolved_env.is_empty() {
                                    env_changed = true;
                                }

                                if env_changed
                                    || config.image != Some(container_config.image.clone())
                                {
                                    info!(
                                        "Container {} config or secrets changed, restarting...",
                                        container_name
                                    );
                                    let _ = self.docker.stop_container(id, None).await;
                                    let _ = self.docker.remove_container(id, None).await;
                                    should_start = true;
                                }
                            }
                        }
                    } else {
                        should_start = true;
                    }
                    managed_containers.remove(&container_name);
                } else {
                    should_start = true;
                }

                if should_start {
                    info!("Starting container: {}", container_name);
                    if let Err(e) = self
                        .start_container(&container_name, container_config, &resolved_env)
                        .await
                    {
                        error!("Failed to start container {}: {}", container_name, e);
                    }
                }
            }

            if let Some(systemd_config) = &manifest.systemd {
                let service_name = format!("r4a-{}", name);
                info!("Ensuring systemd service: {}", service_name);
                let _ = self.service_manager.enable(
                    &service_name,
                    &format!("r4a managed {}", name),
                    &systemd_config.exec,
                    &[],
                );
            }
        }

        for (name, id) in managed_containers {
            if let Some(id) = id {
                info!("Removing orphaned container: {}", name);
                let _ = self
                    .docker
                    .remove_container(
                        &id,
                        Some(bollard::container::RemoveContainerOptions {
                            force: true,
                            ..Default::default()
                        }),
                    )
                    .await;
            }
        }

        Ok(())
    }

    async fn resolve_env(&self, env: &HashMap<String, String>) -> HashMap<String, String> {
        let mut resolved = HashMap::new();
        let client = reqwest::Client::new();
        for (k, v) in env {
            if v.starts_with("vault://") {
                let path = &v[8..];
                let (config_id, key) = if let Some(idx) = path.find('/') {
                    (&path[..idx], &path[idx + 1..])
                } else {
                    ("default", path)
                };

                let token = {
                    let r = self.agent_token.read().unwrap();
                    r.clone()
                };

                if token.is_empty() {
                    warn!("Agent token is empty, skipping vault secret: {}", path);
                    resolved.insert(k.clone(), v.clone());
                    continue;
                }

                match client
                    .get("http://master.r4a.local:3501/api/vault")
                    .query(&[("config_id", config_id), ("key", key)])
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await
                {
                    Ok(resp) if resp.status() == 200 => {
                        if let Ok(val) = resp.json::<String>().await {
                            resolved.insert(k.clone(), val);
                            continue;
                        }
                    }
                    Ok(resp) => warn!("Vault fetch failed for {}: {}", path, resp.status()),
                    Err(e) => error!("Vault fetch error for {}: {}", path, e),
                }
            }
            resolved.insert(k.clone(), v.clone());
        }
        resolved
    }

    async fn start_container(
        &self,
        name: &str,
        config: &r4a_core::ContainerConfig,
        env: &HashMap<String, String>,
    ) -> Result<()> {
        let node_name = self.node_name.clone();
        use futures_util::stream::StreamExt;

        let image_exists = self.docker.inspect_image(&config.image).await.is_ok();
        if !image_exists {
            info!("Image {} not found locally, pulling...", config.image);
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
            info!("Image {} pulled successfully", config.image);
        }

        let env_list: Vec<String> = env.iter().map(|(k, v)| format!("{}={}", k, v)).collect();

        let mut port_bindings = HashMap::new();
        let mut exposed_ports: HashMap<String, HashMap<(), ()>> = HashMap::new();
        if let Some(ports) = &config.ports {
            for port_mapping in ports {
                let parts: Vec<&str> = port_mapping.split(':').collect();
                if parts.len() == 2 {
                    let host_port = parts[0];
                    let container_port = parts[1];
                    let port_key = format!("{}/tcp", container_port);
                    port_bindings.insert(
                        port_key.clone(),
                        Some(vec![PortBinding {
                            host_ip: Some("0.0.0.0".to_string()),
                            host_port: Some(host_port.to_string()),
                        }]),
                    );
                    exposed_ports.insert(port_key, HashMap::new());
                }
            }
        }

        let host_config = HostConfig {
            binds: config.volumes.clone(),
            port_bindings: Some(port_bindings),
            restart_policy: Some(bollard::models::RestartPolicy {
                name: Some(bollard::models::RestartPolicyNameEnum::ALWAYS),
                ..Default::default()
            }),
            ..Default::default()
        };

        let mut labels = HashMap::new();
        labels.insert("r4a.node".to_string(), node_name);

        let docker_config = Config {
            image: Some(config.image.clone()),
            env: Some(env_list),
            cmd: config.command.clone(),
            host_config: Some(host_config),
            labels: Some(labels),
            exposed_ports: if exposed_ports.is_empty() {
                None
            } else {
                Some(exposed_ports)
            },
            ..Default::default()
        };

        if let Err(e) = self
            .docker
            .create_container(
                Some(CreateContainerOptions {
                    name: name.to_string(),
                    ..Default::default()
                }),
                docker_config.clone(),
            )
            .await
        {
            if let bollard::errors::Error::DockerResponseServerError {
                status_code: 409, ..
            } = e
            {
                // Only remove if the existing container is r4a-managed (has our label).
                // Never touch containers created outside of r4a.
                let is_ours = self
                    .docker
                    .inspect_container(name, None)
                    .await
                    .ok()
                    .and_then(|c| c.config)
                    .and_then(|c| c.labels)
                    .map(|l| l.contains_key("r4a.node"))
                    .unwrap_or(false);

                if !is_ours {
                    return Err(anyhow::anyhow!(
                        "Container '{}' already exists and was not created by r4a — refusing to remove it",
                        name
                    ));
                }

                info!(
                    "Container {} already exists with r4a label, recreating...",
                    name
                );
                let _ = self.docker.stop_container(name, None).await;
                let _ = self
                    .docker
                    .remove_container(
                        name,
                        Some(bollard::container::RemoveContainerOptions {
                            force: true,
                            ..Default::default()
                        }),
                    )
                    .await;
                self.docker
                    .create_container(
                        Some(CreateContainerOptions {
                            name: name.to_string(),
                            ..Default::default()
                        }),
                        docker_config.clone(),
                    )
                    .await?;
            } else {
                return Err(e.into());
            }
        }

        if let Err(e) = self
            .docker
            .start_container(name, None::<StartContainerOptions<String>>)
            .await
        {
            if let bollard::errors::Error::DockerResponseServerError {
                status_code: 304, ..
            } = e
            {
            } else {
                return Err(e.into());
            }
        }

        info!("Container {} started successfully", name);
        Ok(())
    }
}
