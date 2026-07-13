use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use r4a_client::ApiClient;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "r4a-cli", about = "r4a cluster management CLI")]
struct Cli {
    /// Master node API URL
    #[arg(long, env = "R4A_MASTER", default_value = "http://master.r4a.local:3501")]
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
    /// Manually remove all r4a network leftovers (DNS, hosts, certs) if connect down failed
    Cleanup,
    /// Install or remove the auto-connect background service (systemd on Linux, launchd on macOS)
    Service {
        #[command(subcommand)]
        cmd: ServiceCommands,
    },
}

#[derive(Subcommand)]
enum ServiceCommands {
    /// Install and enable the auto-connect service (systemd on Linux, launchd on macOS)
    Install {
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        wg_endpoint: Option<String>,
        /// Linux only: "user" (~/.config/systemd/user/, no root) or "system" (/etc/systemd/system/, requires root)
        #[arg(long, default_value = "user")]
        scope: String,
        /// Remove existing service before installing
        #[arg(long)]
        reinstall: bool,
    },
    /// Remove the auto-connect service
    Uninstall {
        /// Linux only: "user" or "system"
        #[arg(long, default_value = "user")]
        scope: String,
    },
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
    #[serde(default)]
    resolver_domain: Option<String>,
    #[serde(default)]
    ca_cert_path: Option<String>,
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

/// Download CA cert from master and install it into the system trust store.
/// Returns the path where the cert was written (for cleanup on disconnect), or None on failure.
async fn install_ca_cert(client: &r4a_client::ApiClient) -> Option<String> {
    let pem = match client.ca_cert().await {
        Ok(p) => p,
        Err(e) => { eprintln!("Warning: could not download CA cert: {}", e); return None; }
    };

    // Determine OS-specific cert path
    // update_cmd используется только в linux-ветке ниже; на macOS глушим warning
    #[cfg_attr(target_os = "macos", allow(unused_variables))]
    let (cert_path, update_cmd): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("/tmp/r4a-ca.crt", &[])  // macOS uses security command directly
    } else if std::path::Path::new("/usr/sbin/update-ca-certificates").exists()
        || std::path::Path::new("/usr/bin/update-ca-certificates").exists()
    {
        ("/usr/local/share/ca-certificates/r4a-ca.crt", &["update-ca-certificates"])
    } else if std::path::Path::new("/usr/bin/update-ca-trust").exists() {
        ("/etc/pki/ca-trust/source/anchors/r4a-ca.crt", &["update-ca-trust", "extract"])
    } else {
        eprintln!("Warning: unknown system, skipping CA cert install. Save manually:\n{}", pem);
        return None;
    };

    if let Err(e) = std::fs::write(cert_path, pem.as_bytes()) {
        eprintln!("Warning: could not write CA cert to {}: {}", cert_path, e);
        return None;
    }

    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("security")
            .args(["add-trusted-cert", "-d", "-r", "trustRoot",
                   "-k", "/Library/Keychains/System.keychain", cert_path])
            .status();
        match status {
            Ok(s) if s.success() => println!("  CA cert:       installed to macOS keychain"),
            _ => eprintln!("Warning: could not install CA cert (try: sudo security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain {})", cert_path),
        }
    }

    #[cfg(not(target_os = "macos"))]
    if !update_cmd.is_empty() {
        let status = std::process::Command::new(update_cmd[0])
            .args(&update_cmd[1..])
            .status();
        match status {
            Ok(s) if s.success() => println!("  CA cert:       installed to system trust store ({})", cert_path),
            _ => eprintln!("Warning: could not run {}. Cert saved to {}. Run manually.", update_cmd[0], cert_path),
        }
    }

    Some(cert_path.to_string())
}

/// Remove CA cert installed by install_ca_cert.
fn remove_ca_cert(cert_path: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("security")
            .args(["remove-trusted-cert", "-d", cert_path])
            .status();
        let _ = std::fs::remove_file(cert_path);
    }

    #[cfg(not(target_os = "macos"))]
    {
        if std::fs::remove_file(cert_path).is_ok() {
            if std::path::Path::new("/usr/sbin/update-ca-certificates").exists()
                || std::path::Path::new("/usr/bin/update-ca-certificates").exists()
            {
                let _ = std::process::Command::new("update-ca-certificates").status();
            } else if std::path::Path::new("/usr/bin/update-ca-trust").exists() {
                let _ = std::process::Command::new("update-ca-trust").args(["extract"]).status();
            }
        }
    }
}

// ── Linux systemd ────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn linux_env_file_path(scope: &str) -> std::path::PathBuf {
    if scope == "system" {
        std::path::PathBuf::from("/etc/r4a-connect.env")
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        std::path::PathBuf::from(home).join(".r4a-connect.env")
    }
}

#[cfg(target_os = "linux")]
fn linux_service_path(scope: &str) -> std::path::PathBuf {
    if scope == "system" {
        std::path::PathBuf::from("/etc/systemd/system/r4a-connect.service")
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        std::path::PathBuf::from(home).join(".config/systemd/user/r4a-connect.service")
    }
}

#[cfg(target_os = "linux")]
fn install_systemd_service(
    bin: &str,
    up_args: &[String],
    master: &str,
    token: Option<&str>,
    is_bearer: bool,
    scope: &str,
) -> Result<()> {
    // Write credentials to env file (not visible in `ps aux`)
    let env_path = linux_env_file_path(scope);
    let mut env_content = format!("R4A_MASTER={}\n", master);
    if let Some(tok) = token {
        let key = if is_bearer { "R4A_TOKEN" } else { "R4A_SECRET" };
        env_content.push_str(&format!("{}={}\n", key, tok));
    }
    std::fs::write(&env_path, &env_content)
        .with_context(|| format!("Could not write env file to {}", env_path.display()))?;
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&env_path, std::fs::Permissions::from_mode(0o600));
    }

    // Write systemd unit file
    let service_path = linux_service_path(scope);
    if let Some(dir) = service_path.parent() {
        std::fs::create_dir_all(dir)?;
    }

    let exec_start = std::iter::once(bin.to_string())
        .chain(up_args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");

    let unit = if scope == "system" {
        let user = std::env::var("USER").unwrap_or_else(|_| "nobody".to_string());
        format!(
            "[Unit]\nDescription=r4a cluster VPN connection\nAfter=network-online.target\nWants=network-online.target\n\n\
             [Service]\nType=simple\nUser={user}\nEnvironmentFile={env}\nExecStart={exec}\nRestart=always\nRestartSec=10\n\n\
             [Install]\nWantedBy=multi-user.target\n",
            user = user,
            env = env_path.display(),
            exec = exec_start,
        )
    } else {
        format!(
            "[Unit]\nDescription=r4a cluster VPN connection\nAfter=network.target\n\n\
             [Service]\nType=simple\nEnvironmentFile={env}\nExecStart={exec}\nRestart=always\nRestartSec=10\n\n\
             [Install]\nWantedBy=default.target\n",
            env = env_path.display(),
            exec = exec_start,
        )
    };

    std::fs::write(&service_path, &unit)
        .with_context(|| format!("Could not write service file to {}", service_path.display()))?;

    let run_ctl = |args: &[&str]| {
        if scope == "user" {
            std::process::Command::new("systemctl").arg("--user").args(args).status()
        } else {
            std::process::Command::new("sudo").arg("systemctl").args(args).status()
        }
    };

    let _ = run_ctl(&["daemon-reload"]);
    let _ = run_ctl(&["enable", "r4a-connect"]);
    let flag = if scope == "user" { "--user " } else { "" };
    match run_ctl(&["start", "r4a-connect"]) {
        Ok(s) if s.success() => {
            println!("  Env file:  {}", env_path.display());
            println!("  Service:   {}", service_path.display());
            println!("  Status:    systemctl {}status r4a-connect", flag);
            println!("  Logs:      journalctl {}--unit r4a-connect -f", flag);
        }
        _ => {
            eprintln!("Warning: service start failed. Files written.");
            eprintln!("Run manually: systemctl {}enable --now r4a-connect", flag);
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn uninstall_systemd_service(scope: &str) -> Result<()> {
    let service_path = linux_service_path(scope);
    let env_path = linux_env_file_path(scope);

    let run_ctl = |args: &[&str]| {
        if scope == "user" {
            std::process::Command::new("systemctl").arg("--user").args(args).status()
        } else {
            std::process::Command::new("sudo").arg("systemctl").args(args).status()
        }
    };

    let _ = run_ctl(&["stop", "r4a-connect"]);
    let _ = run_ctl(&["disable", "r4a-connect"]);
    let _ = std::fs::remove_file(&service_path);
    let _ = std::fs::remove_file(&env_path);
    let _ = run_ctl(&["daemon-reload"]);
    println!("  Removed: {}", service_path.display());
    Ok(())
}

// ── macOS launchd ────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn launchd_plist_path() -> Result<PathBuf> {
    // When running as root, install as a system daemon so it can manage WireGuard interfaces
    if unsafe { libc::geteuid() } == 0 {
        return Ok(PathBuf::from("/Library/LaunchDaemons/com.r4a.connect.plist"));
    }
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join("Library/LaunchAgents/com.r4a.connect.plist"))
}

#[cfg(target_os = "macos")]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
}

#[cfg(target_os = "macos")]
fn install_launchd_service(
    bin: &str,
    up_args: &[String],
    master: &str,
    token: Option<&str>,
    is_bearer: bool,
) -> Result<()> {
    // System daemons must run from a system path — copy binary to /usr/local/bin
    let system_bin = "/usr/local/bin/r4a-cli";
    if bin != system_bin {
        std::fs::copy(bin, system_bin)
            .with_context(|| format!("copy {} to {} (run as root?)", bin, system_bin))?;
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(system_bin, std::fs::Permissions::from_mode(0o755));
    }
    let bin = system_bin;

    let plist_path = launchd_plist_path()?;
    if let Some(dir) = plist_path.parent() {
        std::fs::create_dir_all(dir)?;
    }

    let args_xml: String = std::iter::once(bin.to_string())
        .chain(up_args.iter().cloned())
        .map(|a| format!("        <string>{}</string>", xml_escape(&a)))
        .collect::<Vec<_>>()
        .join("\n");

    // Credentials go into EnvironmentVariables (not ProgramArguments, so not visible in ps)
    let mut env_entries = format!(
        "        <key>PATH</key>\n        <string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string>\n\
         <key>R4A_MASTER</key>\n        <string>{}</string>",
        xml_escape(master)
    );
    if let Some(tok) = token {
        let key = if is_bearer { "R4A_TOKEN" } else { "R4A_SECRET" };
        env_entries.push_str(&format!(
            "\n        <key>{}</key>\n        <string>{}</string>",
            key,
            xml_escape(tok)
        ));
    }

    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
             <key>Label</key>\n\
             <string>com.r4a.connect</string>\n\
             <key>ProgramArguments</key>\n\
             <array>\n\
         {args}\n\
             </array>\n\
             <key>EnvironmentVariables</key>\n\
             <dict>\n\
         {env}\n\
             </dict>\n\
             <key>RunAtLoad</key>\n\
             <true/>\n\
             <key>KeepAlive</key>\n\
             <true/>\n\
             <key>StandardOutPath</key>\n\
             <string>/tmp/r4a-connect.log</string>\n\
             <key>StandardErrorPath</key>\n\
             <string>/tmp/r4a-connect.log</string>\n\
         </dict>\n\
         </plist>\n",
        args = args_xml,
        env = env_entries,
    );

    std::fs::write(&plist_path, plist.as_bytes())
        .with_context(|| format!("Could not write plist to {}", plist_path.display()))?;
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&plist_path, std::fs::Permissions::from_mode(0o600));
    }

    let path_str = plist_path.to_string_lossy().to_string();
    let is_system = unsafe { libc::geteuid() } == 0;
    let domain = if is_system {
        "system".to_string()
    } else {
        format!("gui/{}", unsafe { libc::getuid() })
    };
    let label = "com.r4a.connect";

    let service_target = format!("{}/{}", domain, label);

    // Stop any running instance first
    let _ = std::process::Command::new("launchctl")
        .args(["bootout", &service_target])
        .output();

    // Enable + kickstart: works reliably for both LaunchAgents and LaunchDaemons
    let _ = std::process::Command::new("launchctl")
        .args(["enable", &service_target])
        .output();
    let load_ok = std::process::Command::new("launchctl")
        .args(["kickstart", "-k", &service_target])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    // Fallback: try old-style load if kickstart failed
    if !load_ok {
        let _ = std::process::Command::new("launchctl")
            .args(["load", "-w", &path_str])
            .output();
    }

    println!("  Plist:   {}", plist_path.display());
    println!("  Logs:    tail -f /tmp/r4a-connect.log");
    println!("  Stop:    launchctl bootout {}/{}", domain, label);

    // Wait up to 15s for the WireGuard interface to come up
    print!("Waiting for VPN...");
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let wg_bin = if std::path::Path::new("/opt/homebrew/bin/wg").exists() {
        "/opt/homebrew/bin/wg"
    } else {
        "wg"
    };
    let connected = (0..15).any(|_| {
        std::thread::sleep(std::time::Duration::from_secs(1));
        print!(".");
        let _ = std::io::stdout().flush();
        std::process::Command::new(wg_bin)
            .args(["show", "wg0"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    });
    println!();
    if connected {
        println!("VPN connected.");
    } else {
        eprintln!("Warning: VPN did not come up in 15s. Check logs: tail -f /tmp/r4a-connect.log");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn uninstall_launchd_service() -> Result<()> {
    let plist_path = launchd_plist_path()?;
    let is_system = unsafe { libc::geteuid() } == 0;
    let domain = if is_system { "system".to_string() } else {
        format!("gui/{}", unsafe { libc::getuid() })
    };
    let _ = std::process::Command::new("launchctl")
        .args(["bootout", &format!("{}/com.r4a.connect", domain)])
        .output();
    let _ = std::fs::remove_file(&plist_path);
    if plist_path.exists() {
        // fallback: try old unload
        let _ = std::process::Command::new("launchctl")
            .args(["unload", "-w", &plist_path.to_string_lossy()])
            .output();
        let _ = std::fs::remove_file(&plist_path);
    }
    println!("  Removed: {}", plist_path.display());
    Ok(())
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
                    let _ = std::process::Command::new(r4a_vpn::wireguard::wg_quick_bin()).args(["down", r4a_vpn::wireguard::wg_conf_path()]).status();
                    for host in &old.added_hosts {
                        let _ = r4a_vpn::dns::set_hosts_entries(&[], host);
                    }
                    if let Some(ref domain) = old.resolver_domain {
                        let _ = r4a_vpn::dns::remove_resolver_domain(domain);
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
                let mut resolver_domain: Option<String> = None;

                // master.r4a.local → 10.42.0.1 (fallback /etc/hosts, works before DNS tunnel)
                match r4a_vpn::dns::set_hosts_entries(&["10.42.0.1"], "master.r4a.local") {
                    Ok(_) => added_hosts.push("master.r4a.local".to_string()),
                    Err(e) => eprintln!("Warning: could not update /etc/hosts: {}", e),
                }

                // <label>.r4a.local → own VPN IP (self-reference, useful even without DNS)
                if let Some(ref lbl) = label {
                    let label_host = format!("{}.r4a.local", lbl);
                    match r4a_vpn::dns::set_hosts_entries(&[resp.vpn_ip.as_str()], &label_host) {
                        Ok(_) => added_hosts.push(label_host),
                        Err(e) => eprintln!("Warning: could not update /etc/hosts for label: {}", e),
                    }
                }

                // Configure system DNS resolver: *.r4a.local → 10.42.0.1:53
                match r4a_vpn::dns::set_resolver_domain("r4a.local", "10.42.0.1") {
                    Ok(_) => resolver_domain = Some("r4a.local".to_string()),
                    Err(e) => eprintln!("Warning: could not configure DNS resolver: {}", e),
                }

                // Download and install CA cert into system trust store
                let ca_cert_path = install_ca_cert(&client).await;

                save_connection_state(&ConnectionState {
                    id: resp.id.clone(),
                    master: cli.master.clone(),
                    vpn_ip: resp.vpn_ip.clone(),
                    master_pubkey: resp.master_pubkey.clone(),
                    master_endpoint: resp.master_endpoint.clone(),
                    label: label.clone(),
                    added_hosts: added_hosts.clone(),
                    resolver_domain: resolver_domain.clone(),
                    ca_cert_path: ca_cert_path.clone(),
                })?;

                println!("Connected!");
                println!("  VPN IP:        {}", resp.vpn_ip);
                println!("  Connection ID: {}", resp.id);
                println!("  WG endpoint:   {}", endpoint);
                if resolver_domain.is_some() {
                    println!("  DNS resolver:  *.r4a.local → 10.42.0.1:53");
                }
                println!("  Hosts entries: {}", added_hosts.join(", "));
                println!("  Ingress:       https://master.r4a.local");
                println!("  Web UI:        https://web.master.r4a.local");
                println!("  API:           https://api.master.r4a.local");
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

                #[cfg(unix)]
                let mut term_signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
                #[cfg(not(unix))]
                let mut term_signal = futures_util::future::pending::<()>(); // Dummy for non-unix

                loop {
                    tokio::select! {
                        _ = ticker.tick() => {
                            let _ = client.connection_heartbeat(&resp.id).await;
                        }
                        _ = tokio::signal::ctrl_c() => {
                            println!("\nReceived SIGINT, disconnecting...");
                            break;
                        }
                        _ = term_signal.recv() => {
                            println!("\nReceived SIGTERM, disconnecting...");
                            break;
                        }
                    }
                }

                // Cleanup logic (shared for both signals)
                let _ = client.connection_delete(&resp.id).await;
                let _ = std::process::Command::new(r4a_vpn::wireguard::wg_quick_bin()).args(["down", r4a_vpn::wireguard::wg_conf_path()]).status();
                for host in &added_hosts {
                    let _ = r4a_vpn::dns::set_hosts_entries(&[], host);
                }
                if let Some(ref domain) = resolver_domain {
                    let _ = r4a_vpn::dns::remove_resolver_domain(domain);
                }
                if let Some(ref path) = ca_cert_path {
                    remove_ca_cert(path);
                }
                remove_connection_state();
                println!("Disconnected.");
            }
            ConnectCommands::Down => {
                let state = load_connection_state()?;
                let _ = client.connection_delete(&state.id).await;
                let _ = std::process::Command::new(r4a_vpn::wireguard::wg_quick_bin()).args(["down", r4a_vpn::wireguard::wg_conf_path()]).status();
                for host in &state.added_hosts {
                    let _ = r4a_vpn::dns::set_hosts_entries(&[], host);
                }
                if let Some(ref domain) = state.resolver_domain {
                    let _ = r4a_vpn::dns::remove_resolver_domain(domain);
                }
                if let Some(ref path) = state.ca_cert_path {
                    remove_ca_cert(path);
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
            ConnectCommands::Cleanup => {
                println!("Performing thorough cleanup of r4a network leftovers...");
                
                // 1. WireGuard down
                let _ = std::process::Command::new(r4a_vpn::wireguard::wg_quick_bin())
                    .args(["down", r4a_vpn::wireguard::wg_conf_path()])
                    .status();
                
                // 2. Remove /etc/resolver/r4a.local
                let _ = r4a_vpn::dns::remove_resolver_domain("r4a.local");
                
                // 3. Clean /etc/hosts (brute force search for managed entries)
                let _ = r4a_vpn::dns::set_hosts_entries(&[], "master.r4a.local");
                // Also try to clean up any labelled hosts if we can find them
                if let Ok(hosts) = std::fs::read_to_string("/etc/hosts") {
                    for line in hosts.lines() {
                        if line.contains("# r4a-managed") {
                            if let Some(host) = line.split_whitespace().nth(1) {
                                let _ = r4a_vpn::dns::set_hosts_entries(&[], host);
                            }
                        }
                    }
                }

                // 4. Remove CA certs from keychain
                #[cfg(target_os = "macos")]
                {
                    let output = std::process::Command::new("security")
                        .args(["find-certificate", "-a", "-c", "r4a Local CA", "-Z", "/Library/Keychains/System.keychain"])
                        .output();
                    if let Ok(out) = output {
                        let out_str = String::from_utf8_lossy(&out.stdout);
                        for line in out_str.lines() {
                            if line.starts_with("SHA-1 hash:") {
                                if let Some(hash) = line.split(':').nth(1) {
                                    let hash = hash.trim();
                                    let _ = std::process::Command::new("security")
                                        .args(["delete-certificate", "-Z", hash, "/Library/Keychains/System.keychain"])
                                        .status();
                                }
                            }
                        }
                    }
                    let _ = std::fs::remove_file("/tmp/r4a-ca.crt");
                }

                // 5. Remove state file
                remove_connection_state();
                
                println!("Cleanup complete.");
            }
            ConnectCommands::Service { cmd } => match cmd {
                ServiceCommands::Install { label, wg_endpoint, scope, reinstall } => {
                    let bin = std::env::current_exe()
                        .context("Could not determine binary path")?
                        .to_string_lossy()
                        .to_string();
                    let token = cli.token.as_deref().or(cli.secret.as_deref());
                    let is_bearer = cli.token.is_some();

                    // Non-sensitive connect args
                    let mut up_args = vec!["connect".to_string(), "up".to_string()];
                    if let Some(ref l) = label {
                        up_args.extend_from_slice(&["--label".to_string(), l.clone()]);
                    }
                    if let Some(ref e) = wg_endpoint {
                        up_args.extend_from_slice(&["--wg-endpoint".to_string(), e.clone()]);
                    }

                    if reinstall {
                        #[cfg(target_os = "linux")]
                        let _ = uninstall_systemd_service(&scope);
                        #[cfg(target_os = "macos")]
                        let _ = uninstall_launchd_service();
                    }

                    println!("Installing r4a-connect service...");

                    #[cfg(target_os = "linux")]
                    install_systemd_service(&bin, &up_args, &cli.master, token, is_bearer, &scope)?;

                    #[cfg(target_os = "macos")]
                    { let _ = &scope; install_launchd_service(&bin, &up_args, &cli.master, token, is_bearer)?; }

                    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
                    { let _ = (scope, bin, up_args, token, is_bearer); eprintln!("Service management is not supported on this platform."); }

                    println!("Done. Connection will start automatically on next login/boot.");
                }
                ServiceCommands::Uninstall { scope } => {
                    println!("Removing r4a-connect service...");

                    #[cfg(target_os = "linux")]
                    uninstall_systemd_service(&scope)?;

                    #[cfg(target_os = "macos")]
                    { let _ = &scope; uninstall_launchd_service()?; }

                    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
                    { let _ = scope; eprintln!("Service management is not supported on this platform."); }

                    println!("Service removed.");
                }
            },
        },
    }

    Ok(())
}
