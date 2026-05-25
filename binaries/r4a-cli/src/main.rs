use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use r4a_client::ApiClient;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "r4a-cli", about = "r4a cluster management CLI")]
struct Cli {
    /// Master node API URL
    #[arg(long, env = "R4A_MASTER", default_value = "http://master.local:8080")]
    master: String,
    
    #[arg(long, env = "R4A_SECRET")]
    secret: Option<String>,

    /// Bearer token for RBAC auth (alternative to --secret)
    #[arg(long, env = "R4A_TOKEN")]
    token: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Node management
    Nodes {
        #[command(subcommand)]
        cmd: NodeCommands,
    },
    /// Git repository management
    Git {
        #[command(subcommand)]
        cmd: GitCommands,
    },
    /// Vault secret management
    Vault {
        #[command(subcommand)]
        cmd: VaultCommands,
    },
    /// RBAC and token management
    Rbac {
        #[command(subcommand)]
        cmd: RbacCommands,
    },
    /// Cluster updates
    Update {
        #[command(subcommand)]
        cmd: UpdateCommands,
    },
    /// Manifest management
    Manifests {
        #[command(subcommand)]
        cmd: ManifestCommands,
    },
    /// Client VPN connection management
    Connect {
        #[command(subcommand)]
        cmd: ConnectCommands,
    },
}

#[derive(Subcommand)]
enum ConnectCommands {
    /// Connect this machine to the cluster via WireGuard (no node registration)
    Up {
        /// Optional label for this connection
        #[arg(long)]
        label: Option<String>,
        /// Override WireGuard endpoint (host:port). Useful when master is behind NAT/Docker.
        /// Example: --wg-endpoint localhost:51820
        #[arg(long)]
        wg_endpoint: Option<String>,
    },
    /// Disconnect from the cluster
    Down,
    /// Show active connection state
    Status,
    /// List all active connections on the master
    List,
}

#[derive(Serialize, Deserialize)]
struct ConnectionState {
    id: String,
    master: String,
    vpn_ip: String,
    master_pubkey: String,
    master_endpoint: String,
    label: Option<String>,
    #[serde(default)]
    added_hosts: Vec<String>,
}

fn connection_state_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".r4a-connection.json")
}

fn load_connection_state() -> Result<ConnectionState> {
    let path = connection_state_path();
    let data = std::fs::read_to_string(&path)
        .with_context(|| format!("No active connection ({})", path.display()))?;
    Ok(serde_json::from_str(&data)?)
}

fn save_connection_state(s: &ConnectionState) -> Result<()> {
    let path = connection_state_path();
    std::fs::write(&path, serde_json::to_string_pretty(s)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn remove_connection_state() {
    let _ = std::fs::remove_file(connection_state_path());
}

#[derive(Subcommand)]
enum ManifestCommands {
    List {
        #[arg(long)]
        node: Option<String>,
    },
}

#[derive(Subcommand)]
enum NodeCommands {
    /// List all nodes in cluster
    List,
}

#[derive(Subcommand)]
enum GitCommands {
    /// List all git repositories
    List,
    /// Create a new git repository
    Create { name: String },
}

#[derive(Subcommand)]
enum VaultCommands {
    Configs,
    CreateConfig { name: String },
    List {
        #[arg(long, default_value = "default")]
        config: String,
    },
    Get { 
        key: String,
        #[arg(long, default_value = "default")]
        config: String,
    },
    Set { 
        key: String,
        value: String,
        #[arg(long, default_value = "default")]
        config: String,
    },
    Delete { 
        key: String,
        #[arg(long, default_value = "default")]
        config: String,
    },
}

#[derive(Subcommand)]
enum RbacCommands {
    /// List all tokens
    List,
    /// Create a new token
    CreateToken {
        username: String,
        #[arg(long, use_value_delimiter = true)]
        verbs: Vec<String>,
        #[arg(long, use_value_delimiter = true)]
        resources: Vec<String>,
        #[arg(long, use_value_delimiter = true)]
        resource_names: Option<Vec<String>>,
    },
    /// Delete a token
    DeleteToken { id: String },
}

#[derive(Subcommand)]
enum UpdateCommands {
    /// Show update status
    Status,
    /// Test update system
    Test,
    /// Trigger cluster-wide update
    Run,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = if let Some(ref tok) = cli.token {
        ApiClient::with_token(&cli.master, tok.clone())
    } else {
        ApiClient::new(&cli.master, cli.secret.clone())
    };

    match cli.command {
        Commands::Nodes { cmd } => match cmd {
            NodeCommands::List => {
                let nodes = client.nodes().await?;
                println!("{:<20} {:<15} {:<10} {:<10} {:<10}", "NAME", "ROLE", "STATUS", "CPU", "RAM");
                for n in nodes {
                    let online = n.last_seen.map(|ls| {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        now - ls < 30
                    }).unwrap_or(false);

                    println!("{:<20} {:<15} {:<10} {:<10} {:<10}", 
                        n.name, 
                        n.role,
                        if online { "Online" } else { "Offline" },
                        n.cpu_percent.map(|c| format!("{:.1}%", c)).unwrap_or_else(|| "—".to_string()),
                        n.ram_used_mb.map(|r| format!("{:.1}GB", r as f32 / 1024.0)).unwrap_or_else(|| "—".to_string())
                    );
                }
            }
        },
        Commands::Git { cmd } => match cmd {
            GitCommands::List => {
                let repos = client.git_repos().await?;
                println!("{:<20} {:<40}", "NAME", "URL");
                for r in repos {
                    println!("{:<20} {:<40}", r.name, r.clone_url);
                }
            }
            GitCommands::Create { name } => {
                let repo = client.create_repo(&name).await?;
                println!("Created repository: {} ({})", repo.name, repo.clone_url);
            }
        },
        Commands::Vault { cmd } => match cmd {
            VaultCommands::Configs => {
                let configs = client.vault_configs_list().await?;
                println!("{:<40} {:<20} {:<20}", "ID", "NAME", "CREATED AT");
                for c in configs {
                    println!("{:<40} {:<20} {}", c.id, c.name, c.created_at);
                }
            }
            VaultCommands::CreateConfig { name } => {
                let config = client.vault_config_create(&name).await?;
                println!("Created vault config: {} ({})", config.name, config.id);
            }
            VaultCommands::List { config } => {
                let keys = client.vault_list(&config).await?;
                for k in keys {
                    println!("{}", k);
                }
            }
            VaultCommands::Get { key, config } => {
                let val = client.vault_get(&config, &key).await?;
                println!("{}", val);
            }
            VaultCommands::Set { key, value, config } => {
                client.vault_set(&config, &key, &value).await?;
                println!("Secret set: {} in {}", key, config);
            }
            VaultCommands::Delete { key, config } => {
                client.vault_delete(&config, &key).await?;
                println!("Secret deleted: {} from {}", key, config);
            }
        },
        Commands::Rbac { cmd } => match cmd {
            RbacCommands::List => {
                let tokens = client.tokens_list().await?;
                println!("{:<36} {:<15}", "ID", "USER");
                for t in tokens {
                    println!("{:<36} {:<15}", t.id, t.username);
                }
            }
            RbacCommands::CreateToken { username, verbs, resources, resource_names } => {
                use r4a_client::{Verb, Resource};
                let verbs: Vec<Verb> = verbs.iter().map(|v| match v.as_str() {
                    "get" | "Get" => Verb::Get,
                    "list" | "List" => Verb::List,
                    "create" | "Create" => Verb::Create,
                    "update" | "Update" => Verb::Update,
                    "delete" | "Delete" => Verb::Delete,
                    _ => Verb::All,
                }).collect();
                let resources: Vec<Resource> = resources.iter().map(|r| match r.as_str() {
                    "nodes" | "Nodes" => Resource::Nodes,
                    "manifests" | "Manifests" => Resource::Manifests,
                    "vault" | "Vault" => Resource::Vault,
                    "git" | "GitRepos" => Resource::GitRepos,
                    "tokens" | "Tokens" => Resource::Tokens,
                    "policies" | "Policies" => Resource::Policies,
                    "bindings" | "Bindings" => Resource::Bindings,
                    _ => Resource::All,
                }).collect();
                let token = client.token_create(&username, verbs, resources, resource_names).await?;
                println!("Created token: {}", token.id);
            }
            RbacCommands::DeleteToken { id } => {
                client.token_delete(&id).await?;
                println!("Token deleted: {}", id);
            }
        },
        Commands::Update { cmd } => match cmd {
            UpdateCommands::Status => {
                let status = client.update_status().await?;
                println!("Master checksum: {}", status.master_checksum.as_deref().unwrap_or("—"));
                println!("Update pending: {}", status.update_pending);
                println!("\nAgents:");
                for (name, info) in status.agents {
                    println!("  {:<20} {:<15} {}", name, info.status, info.checksum.as_deref().unwrap_or("—"));
                }
            }
            UpdateCommands::Test => {
                let resp = client.update_test().await?;
                println!("OK: {}", resp.ok);
                println!("Message: {}", resp.message);
                if let Some(cs) = resp.checksum {
                    println!("Checksum: {}", cs);
                }
            }
            UpdateCommands::Run => {
                client.update_trigger().await?;
                println!("Update triggered for agents.");
                client.server_update_trigger().await?;
                println!("Update triggered for server.");
            }
        },
        Commands::Manifests { cmd } => match cmd {
            ManifestCommands::List { node } => {
                let manifests = client.manifests(node.as_deref()).await?;
                println!("{:<20} {:<20} {:<20}", "APP", "NODE", "TYPE");
                for (name, m) in manifests {
                    let kind = if m.container.is_some() { "Docker" } else if m.systemd.is_some() { "Systemd" } else { "Other" };
                    println!("{:<20} {:<20} {:<20}", name, m.app.node_selector, kind);
                }
            }
        },
        Commands::Connect { cmd } => match cmd {
            ConnectCommands::Up { label, wg_endpoint } => {
                // Bring down old WG interface if any, but do NOT delete the server-side
                // connection — let server evict by label and reuse the same VPN IP.
                if let Ok(old) = load_connection_state() {
                    let _ = std::process::Command::new("wg-quick").args(["down", "wg0"]).status();
                    for host in &old.added_hosts {
                        let _ = r4a_vpn::dns::set_hosts_entries(&[], host);
                    }
                    remove_connection_state();
                }

                let keypair = r4a_vpn::wireguard::generate_keypair()
                    .context("Failed to generate WireGuard keypair (is wg installed?)")?;

                let resp = client.connection_create(&keypair.public, label.as_deref()).await?;

                // Derive WG endpoint: use --wg-endpoint if given, otherwise replace
                // the host in master_endpoint with the host from --master URL so the
                // tunnel goes through the same address the user used for the API.
                let derived_endpoint = wg_endpoint.unwrap_or_else(|| {
                    let master_host = cli.master
                        .trim_start_matches("http://")
                        .trim_start_matches("https://")
                        .split(':').next()
                        .unwrap_or("10.42.0.1");
                    let wg_port = resp.master_endpoint.rsplit(':').next().unwrap_or("51820");
                    format!("{}:{}", master_host, wg_port)
                });
                let endpoint = derived_endpoint.as_str();
                r4a_vpn::wireguard::setup_agent(
                    &keypair.private,
                    &resp.vpn_ip,
                    &resp.master_pubkey,
                    endpoint,
                ).context("Failed to configure WireGuard interface")?;

                let mut added_hosts: Vec<String> = Vec::new();

                // master.r4a.local → 10.42.0.1
                match r4a_vpn::dns::set_hosts_entries(&["10.42.0.1"], "master.r4a.local") {
                    Ok(_) => added_hosts.push("master.r4a.local".to_string()),
                    Err(e) => eprintln!("Warning: could not update /etc/hosts: {}", e),
                }

                // <label>.r4a.local → own VPN IP
                if let Some(ref lbl) = label {
                    let label_host = format!("{}.r4a.local", lbl);
                    match r4a_vpn::dns::set_hosts_entries(&[resp.vpn_ip.as_str()], &label_host) {
                        Ok(_) => added_hosts.push(label_host),
                        Err(e) => eprintln!("Warning: could not update /etc/hosts for label: {}", e),
                    }
                }

                // <node_name>.r4a.local → node VPN IP
                match client.nodes().await {
                    Ok(nodes) => {
                        for node in &nodes {
                            let node_host = format!("{}.r4a.local", node.name);
                            match r4a_vpn::dns::set_hosts_entries(&[node.ip.as_str()], &node_host) {
                                Ok(_) => added_hosts.push(node_host),
                                Err(e) => eprintln!("Warning: /etc/hosts for {}: {}", node.name, e),
                            }
                        }
                    }
                    Err(e) => eprintln!("Warning: could not fetch nodes for DNS setup: {}", e),
                }

                save_connection_state(&ConnectionState {
                    id: resp.id.clone(),
                    master: cli.master.clone(),
                    vpn_ip: resp.vpn_ip.clone(),
                    master_pubkey: resp.master_pubkey.clone(),
                    master_endpoint: resp.master_endpoint.clone(),
                    label: label.clone(),
                    added_hosts: added_hosts.clone(),
                })?;

                println!("Connected!");
                println!("  VPN IP:        {}", resp.vpn_ip);
                println!("  Connection ID: {}", resp.id);
                println!("  WG endpoint:   {}", endpoint);
                println!("  DNS entries added:");
                for h in &added_hosts {
                    println!("    {}", h);
                }
                println!("  Ingress:       http://master.r4a.local:8000");
                println!("  Heartbeat:     every {}s", resp.heartbeat_interval_secs);
                println!();

                // Quick reachability check
                let check = std::process::Command::new("ping")
                    .args(["-c", "1", "-W", "2", "10.42.0.1"])
                    .output();
                match check {
                    Ok(o) if o.status.success() => println!("  ✓ 10.42.0.1 reachable"),
                    _ => eprintln!("  ✗ Warning: 10.42.0.1 not reachable — tunnel may not be up"),
                }
                println!();
                println!("Sending heartbeats — press Ctrl-C to disconnect.");

                let interval = std::time::Duration::from_secs(resp.heartbeat_interval_secs);
                let mut ticker = tokio::time::interval(interval);
                ticker.tick().await; // skip immediate first tick
                loop {
                    tokio::select! {
                        _ = ticker.tick() => {
                            let _ = client.connection_heartbeat(&resp.id).await;
                        }
                        _ = tokio::signal::ctrl_c() => {
                            println!("\nDisconnecting...");
                            let _ = client.connection_delete(&resp.id).await;
                            let _ = std::process::Command::new("wg-quick").args(["down", "wg0"]).status();
                            for host in &added_hosts {
                                let _ = r4a_vpn::dns::set_hosts_entries(&[], host);
                            }
                            remove_connection_state();
                            println!("Disconnected.");
                            break;
                        }
                    }
                }
            }
            ConnectCommands::Down => {
                let state = load_connection_state()?;
                let _ = client.connection_delete(&state.id).await;
                let _ = std::process::Command::new("wg-quick").args(["down", "wg0"]).status();
                for host in &state.added_hosts {
                    let _ = r4a_vpn::dns::set_hosts_entries(&[], host);
                }
                remove_connection_state();
                println!("Disconnected.");
            }
            ConnectCommands::Status => {
                match load_connection_state() {
                    Ok(s) => {
                        println!("Connected to:  {}", s.master);
                        println!("VPN IP:        {}", s.vpn_ip);
                        println!("Connection ID: {}", s.id);
                        println!("Master WG:     {}", s.master_endpoint);
                    }
                    Err(e) => println!("Not connected: {}", e),
                }
            }
            ConnectCommands::List => {
                let conns = client.connections_list().await?;
                println!("{:<36} {:<15} {:<20} {}", "ID", "VPN IP", "LABEL", "LAST SEEN");
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                for c in conns {
                    let age = now.saturating_sub(c.last_seen);
                    let age_str = if age < 60 { format!("{}s ago", age) }
                        else if age < 3600 { format!("{}m ago", age / 60) }
                        else { format!("{}h ago", age / 3600) };
                    println!("{:<36} {:<15} {:<20} {}",
                        &c.id[..8.min(c.id.len())],
                        c.vpn_ip,
                        c.label.as_deref().unwrap_or("—"),
                        age_str);
                }
            }
        },
    }

    Ok(())
}
