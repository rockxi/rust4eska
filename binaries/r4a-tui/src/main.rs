mod ui;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use r4a_client::{ApiClient, Manifest, NodeInfo, RepoInfo, Token, UpdateStatus};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Tabs},
    Terminal,
};
use sha2::{Digest, Sha256};
use std::{io, time::Duration};
use ui::Screen;

const POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Parser)]
#[command(name = "r4a-tui", about = "r4a cluster dashboard")]
struct Cli {
    /// Master node API URL
    #[arg(
        long,
        env = "R4A_MASTER",
        default_value = "http://master.r4a.local:3501"
    )]
    master: String,
    #[arg(long, env = "R4A_SECRET")]
    secret: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    Update,
}

struct App {
    screen: Screen,
    nodes: Vec<NodeInfo>,
    fetch_error: Option<String>,
    git_repos: Option<Vec<RepoInfo>>,
    git_error: Option<String>,
    git_input: Option<String>,
    git_message: Option<String>,
    update_status: Option<UpdateStatus>,
    update_message: Option<String>,
    vault_configs: Option<Vec<r4a_client::VaultConfig>>,
    vault_config_idx: usize,
    vault_keys: Option<Vec<String>>,
    vault_input: Option<String>,
    vault_message: Option<String>,
    vault_revealed: Option<String>,
    vault_selected_idx: usize,
    vault_editing_key: Option<String>,
    vault_grant_input: Option<String>,
    rbac_tokens: Option<Vec<Token>>,
    rbac_selected_idx: usize,
    rbac_message: Option<String>,
    manifests: Option<Vec<Manifest>>,
    manifests_selected_idx: usize,
    manifests_input: Option<String>,
    manifests_message: Option<String>,
    logs_containers: Option<Vec<(String, String)>>,
    logs_selected_idx: usize,
    logs_entries: Option<Vec<r4a_client::LogEntry>>,
    logs_message: Option<String>,
    client: ApiClient,
}

impl App {
    fn new(master_url: &str, secret: Option<String>) -> Self {
        Self {
            screen: Screen::Dashboard,
            nodes: vec![],
            fetch_error: None,
            git_repos: None,
            git_error: None,
            git_input: None,
            git_message: None,
            update_status: None,
            update_message: None,
            vault_configs: None,
            vault_config_idx: 0,
            vault_keys: None,
            vault_input: None,
            vault_message: None,
            vault_revealed: None,
            vault_selected_idx: 0,
            vault_editing_key: None,
            vault_grant_input: None,
            rbac_tokens: None,
            rbac_selected_idx: 0,
            rbac_message: None,
            manifests: None,
            manifests_selected_idx: 0,
            manifests_input: None,
            manifests_message: None,
            logs_containers: None,
            logs_selected_idx: 0,
            logs_entries: None,
            logs_message: None,
            client: ApiClient::new(master_url, secret),
        }
    }

    async fn refresh(&mut self) {
        match self.client.nodes().await {
            Ok(nodes) => {
                self.nodes = nodes;
                self.fetch_error = None;
            }
            Err(e) => {
                self.fetch_error = Some(format!("error: {e}"));
            }
        }

        if self.screen == Screen::Git {
            match self.client.git_repos().await {
                Ok(r) => {
                    self.git_repos = Some(r);
                    self.git_error = None;
                }
                Err(e) => self.git_error = Some(e.to_string()),
            }
        }

        if self.screen == Screen::Vault {
            match self.client.vault_configs_list().await {
                Ok(c) => {
                    self.vault_configs = Some(c);
                }
                Err(e) => self.vault_message = Some(format!("Error: {e}")),
            }

            let config_id = self
                .vault_configs
                .as_ref()
                .and_then(|c| c.get(self.vault_config_idx))
                .map(|c| c.id.as_str())
                .unwrap_or("default");

            match self.client.vault_list(config_id).await {
                Ok(k) => {
                    self.vault_keys = Some(k);
                }
                Err(e) => self.vault_message = Some(format!("Error: {e}")),
            }
            match self.client.tokens_list().await {
                Ok(t) => {
                    self.rbac_tokens = Some(t);
                }
                Err(_) => {}
            }
        }

        if self.screen == Screen::Manifests {
            match self.client.manifests(None).await {
                Ok(m) => {
                    let mut list: Vec<Manifest> = m.into_values().collect();
                    list.sort_by(|a, b| a.app.name.cmp(&b.app.name));
                    self.manifests = Some(list);
                    self.manifests_message = None;
                }
                Err(e) => self.manifests_message = Some(format!("Error: {e}")),
            }
        }

        if self.screen == Screen::Rbac {
            match self.client.tokens_list().await {
                Ok(t) => {
                    self.rbac_tokens = Some(t);
                }
                Err(e) => self.rbac_message = Some(format!("Error: {e}")),
            }
        }

        if self.screen == Screen::Logs {
            match self.client.logs_containers().await {
                Ok(c) => {
                    if self.logs_selected_idx >= c.len() {
                        self.logs_selected_idx = c.len().saturating_sub(1);
                    }
                    self.logs_containers = Some(c);
                    self.logs_message = None;
                }
                Err(e) => self.logs_message = Some(format!("Error: {e}")),
            }
            let selected = self
                .logs_containers
                .as_ref()
                .and_then(|c| c.get(self.logs_selected_idx))
                .cloned();
            if let Some((node, container)) = selected {
                match self.client.logs(&node, &container, 500).await {
                    Ok(entries) => self.logs_entries = Some(entries),
                    Err(e) => self.logs_message = Some(format!("Error: {e}")),
                }
            } else {
                self.logs_entries = None;
            }
        }

        if self.screen == Screen::Update {
            match self.client.update_status().await {
                Ok(s) => self.update_status = Some(s),
                Err(_) => {}
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(Commands::Update) = cli.command {
        return handle_update_command(&cli.master, cli.secret).await;
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, &cli.master, cli.secret).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn handle_update_command(master_url: &str, secret: Option<String>) -> Result<()> {
    let client = ApiClient::new(master_url, secret);

    println!("Checking for updates on {}...", master_url);

    print!("- Fetching latest release from GitHub... ");
    match client.fetch_github_release().await {
        Ok(v) => println!("OK (version {})", v),
        Err(e) => {
            println!("FAIL: {e}");
            return Err(e);
        }
    }

    print!("- Triggering agent updates... ");
    match client.update_trigger().await {
        Ok(()) => println!("OK (agents will update within 30s)"),
        Err(e) => println!("FAIL: {e}"),
    }

    print!("- Checking for r4a-tui update... ");
    match tui_self_update(&client).await {
        Ok(updated) => {
            if updated {
                println!("UPDATED (successfully replaced binary)");
            } else {
                println!("ALREADY UP TO DATE");
            }
        }
        Err(e) => println!("FAIL: {e}"),
    }

    print!("- Triggering r4a-server restart... ");
    match client.server_update_trigger().await {
        Ok(()) => println!("OK (server is restarting)"),
        Err(e) => println!("FAIL: {e}"),
    }

    println!("\nUpdate process finished.");
    Ok(())
}

async fn tui_self_update(client: &ApiClient) -> Result<bool> {
    let master_checksum = client.get_tui_checksum().await?;

    let self_path = std::env::current_exe().context("Failed to get current executable path")?;
    let data = std::fs::read(&self_path)?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let self_checksum = format!("{:x}", hasher.finalize());

    if self_checksum == master_checksum {
        return Ok(false);
    }

    let bytes = client.download_tui_binary().await?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let downloaded_checksum = format!("{:x}", hasher.finalize());

    if downloaded_checksum != master_checksum {
        anyhow::bail!("Checksum mismatch for tui binary");
    }

    let tmp_path = format!("{}.new", self_path.display());
    std::fs::write(&tmp_path, &bytes)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))?;
    }

    let check_status = std::process::Command::new(&tmp_path).arg("--help").status();
    if match check_status {
        Ok(status) => !status.success(),
        Err(_) => true,
    } {
        let _ = std::fs::remove_file(&tmp_path);
        anyhow::bail!("TUI Binary verification failed");
    }

    std::fs::rename(&tmp_path, &self_path)?;
    Ok(true)
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    master_url: &str,
    secret: Option<String>,
) -> Result<()> {
    let mut app = App::new(master_url, secret);
    app.refresh().await;

    let mut last_refresh = std::time::Instant::now();

    loop {
        terminal.draw(|f| draw(f, &app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                // Git input mode intercepts all keys
                if app.screen == Screen::Git && app.git_input.is_some() {
                    match key.code {
                        KeyCode::Esc => {
                            app.git_input = None;
                        }
                        KeyCode::Backspace => {
                            if let Some(ref mut s) = app.git_input {
                                s.pop();
                            }
                        }
                        KeyCode::Enter => {
                            let name = app.git_input.take().unwrap_or_default();
                            let name = name.trim().to_string();
                            if !name.is_empty() {
                                match app.client.create_repo(&name).await {
                                    Ok(repo) => {
                                        app.git_message = Some(format!("Created: {}", repo.name));
                                        if let Ok(r) = app.client.git_repos().await {
                                            app.git_repos = Some(r);
                                            app.git_error = None;
                                        }
                                    }
                                    Err(e) => app.git_message = Some(format!("Error: {e}")),
                                }
                            }
                        }
                        KeyCode::Char(c) => {
                            if let Some(ref mut s) = app.git_input {
                                s.push(c);
                            }
                        }
                        _ => {}
                    }
                    continue;
                }

                // Vault input mode
                if app.screen == Screen::Vault && app.vault_input.is_some() {
                    match key.code {
                        KeyCode::Esc => {
                            app.vault_input = None;
                            app.vault_editing_key = None;
                        }
                        KeyCode::Backspace => {
                            if let Some(ref mut s) = app.vault_input {
                                s.pop();
                            }
                        }
                        KeyCode::Enter => {
                            let input = app.vault_input.take().unwrap_or_default();
                            app.vault_editing_key = None;
                            let parts: Vec<&str> = input.splitn(2, '=').collect();
                            if parts.len() == 2 {
                                let key = parts[0].trim();
                                let val = parts[1].trim();
                                let config_id = app
                                    .vault_configs
                                    .as_ref()
                                    .and_then(|c| c.get(app.vault_config_idx))
                                    .map(|c| c.id.as_str())
                                    .unwrap_or("default");

                                match app.client.vault_set(config_id, key, val).await {
                                    Ok(()) => {
                                        app.vault_message = Some(format!("Set: {}", key));
                                        if let Ok(k) = app.client.vault_list(config_id).await {
                                            app.vault_keys = Some(k);
                                        }
                                    }
                                    Err(e) => app.vault_message = Some(format!("Error: {e}")),
                                }
                            } else {
                                app.vault_message = Some("Format: key=value".to_string());
                            }
                        }
                        KeyCode::Char(c) => {
                            if let Some(ref mut s) = app.vault_input {
                                s.push(c);
                            }
                        }
                        _ => {}
                    }
                    continue;
                }

                if app.screen == Screen::Vault && app.vault_grant_input.is_some() {
                    match key.code {
                        KeyCode::Esc => {
                            app.vault_grant_input = None;
                        }
                        KeyCode::Backspace => {
                            if let Some(ref mut s) = app.vault_grant_input {
                                s.pop();
                            }
                        }
                        KeyCode::Enter => {
                            let input = app
                                .vault_grant_input
                                .take()
                                .unwrap_or_default()
                                .trim()
                                .to_string();
                            if input.starts_with("CONFIG: ") {
                                let name = input.trim_start_matches("CONFIG: ").to_string();
                                if !name.is_empty() {
                                    match app.client.vault_config_create(&name).await {
                                        Ok(config) => {
                                            app.vault_message =
                                                Some(format!("Created config: {}", config.name));
                                            app.refresh().await;
                                        }
                                        Err(e) => app.vault_message = Some(format!("Error: {e}")),
                                    }
                                }
                            } else {
                                let username = input;
                                if !username.is_empty() {
                                    if let Some(ref keys) = app.vault_keys {
                                        if let Some(key) = keys.get(app.vault_selected_idx) {
                                            let config_id = app
                                                .vault_configs
                                                .as_ref()
                                                .and_then(|c| c.get(app.vault_config_idx))
                                                .map(|c| c.id.as_str())
                                                .unwrap_or("default");
                                            let full_key = format!("{}/{}", config_id, key);

                                            match app
                                                .client
                                                .token_create(
                                                    &username,
                                                    vec![r4a_core::models::Verb::Get],
                                                    vec![r4a_core::models::Resource::Vault],
                                                    Some(vec![full_key]),
                                                )
                                                .await
                                            {
                                                Ok(token) => {
                                                    app.vault_message = Some(format!(
                                                        "Created token for {}: {}",
                                                        username, token.id
                                                    ));
                                                    if let Ok(t) = app.client.tokens_list().await {
                                                        app.rbac_tokens = Some(t);
                                                    }
                                                }
                                                Err(e) => {
                                                    app.vault_message = Some(format!("Error: {e}"))
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        KeyCode::Char(c) => {
                            if let Some(ref mut s) = app.vault_grant_input {
                                s.push(c);
                            }
                        }
                        _ => {}
                    }
                    continue;
                }

                // Manifests input mode
                if app.screen == Screen::Manifests && app.manifests_input.is_some() {
                    match key.code {
                        KeyCode::Esc => {
                            app.manifests_input = None;
                        }
                        KeyCode::Backspace => {
                            if let Some(ref mut s) = app.manifests_input {
                                s.pop();
                            }
                        }
                        KeyCode::Enter => {
                            let name = app
                                .manifests_input
                                .take()
                                .unwrap_or_default()
                                .trim()
                                .to_string();
                            if !name.is_empty() {
                                let manifest = r4a_client::Manifest {
                                    app: r4a_client::AppConfig {
                                        name: name.clone(),
                                        node_selector: "all".to_string(),
                                    },
                                    container: Some(r4a_client::ContainerConfig {
                                        image: "alpine:latest".to_string(),
                                        restart: "always".to_string(),
                                        command: None,
                                        ports: None,
                                        volumes: None,
                                    }),
                                    systemd: None,
                                    ingress: None,
                                    env: Default::default(),
                                };
                                match app.client.manifest_upsert(&manifest).await {
                                    Ok(()) => {
                                        app.manifests_message = Some(format!(
                                            "Created: {} (edit via API/web to configure)",
                                            name
                                        ));
                                        app.refresh().await;
                                    }
                                    Err(e) => app.manifests_message = Some(format!("Error: {e}")),
                                }
                            }
                        }
                        KeyCode::Char(c) => {
                            if let Some(ref mut s) = app.manifests_input {
                                s.push(c);
                            }
                        }
                        _ => {}
                    }
                    continue;
                }

                // If revealed value is shown, any key hides it
                if app.vault_revealed.is_some() {
                    app.vault_revealed = None;
                    continue;
                }

                match (key.code, key.modifiers) {
                    (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
                    (KeyCode::Tab, _) | (KeyCode::Right, _) | (KeyCode::Char('l'), _) => {
                        app.screen = app.screen.next();
                        app.update_message = None;
                    }
                    (KeyCode::BackTab, _) | (KeyCode::Left, _) | (KeyCode::Char('h'), _) => {
                        app.screen = app.screen.prev();
                        app.update_message = None;
                    }
                    (KeyCode::Char('n'), _) if app.screen == Screen::Manifests => {
                        app.manifests_input = Some(String::new());
                        app.manifests_message = None;
                    }
                    (KeyCode::Char('d'), _) if app.screen == Screen::Manifests => {
                        if let Some(ref list) = app.manifests {
                            if let Some(m) = list.get(app.manifests_selected_idx) {
                                let name = m.app.name.clone();
                                match app.client.manifest_delete(&name).await {
                                    Ok(()) => {
                                        app.manifests_message = Some(format!("Deleted: {}", name));
                                        app.manifests_selected_idx = 0;
                                        app.refresh().await;
                                    }
                                    Err(e) => app.manifests_message = Some(format!("Error: {e}")),
                                }
                            }
                        }
                    }
                    (KeyCode::Up, _) | (KeyCode::Char('k'), _)
                        if app.screen == Screen::Manifests =>
                    {
                        if app.manifests_selected_idx > 0 {
                            app.manifests_selected_idx -= 1;
                        }
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _)
                        if app.screen == Screen::Manifests =>
                    {
                        if let Some(ref list) = app.manifests {
                            if app.manifests_selected_idx < list.len().saturating_sub(1) {
                                app.manifests_selected_idx += 1;
                            }
                        }
                    }
                    (KeyCode::Char('g'), _) if app.screen == Screen::Manifests => {
                        app.manifests_selected_idx = 0;
                    }
                    (KeyCode::Char('G'), _) if app.screen == Screen::Manifests => {
                        if let Some(ref list) = app.manifests {
                            app.manifests_selected_idx = list.len().saturating_sub(1);
                        }
                    }
                    (KeyCode::Char('n'), _) if app.screen == Screen::Git => {
                        app.git_input = Some(String::new());
                        app.git_message = None;
                    }
                    (KeyCode::Char('n'), _) if app.screen == Screen::Vault => {
                        app.vault_input = Some(String::new());
                        app.vault_editing_key = None;
                        app.vault_message = None;
                    }
                    (KeyCode::Char('e'), _) if app.screen == Screen::Vault => {
                        if let Some(ref keys) = app.vault_keys {
                            if let Some(key) = keys.get(app.vault_selected_idx) {
                                let config_id = app
                                    .vault_configs
                                    .as_ref()
                                    .and_then(|c| c.get(app.vault_config_idx))
                                    .map(|c| c.id.as_str())
                                    .unwrap_or("default");

                                match app.client.vault_get(config_id, key).await {
                                    Ok(val) => {
                                        app.vault_input = Some(format!("{}={}", key, val));
                                        app.vault_editing_key = Some(key.clone());
                                        app.vault_message = None;
                                    }
                                    Err(e) => app.vault_message = Some(format!("Error: {e}")),
                                }
                            }
                        }
                    }
                    (KeyCode::Char('d'), _) if app.screen == Screen::Vault => {
                        if let Some(ref keys) = app.vault_keys {
                            if let Some(key) = keys.get(app.vault_selected_idx) {
                                let config_id = app
                                    .vault_configs
                                    .as_ref()
                                    .and_then(|c| c.get(app.vault_config_idx))
                                    .map(|c| c.id.as_str())
                                    .unwrap_or("default");

                                match app.client.vault_delete(config_id, key).await {
                                    Ok(()) => {
                                        app.vault_message = Some(format!("Deleted: {}", key));
                                        if let Ok(k) = app.client.vault_list(config_id).await {
                                            app.vault_keys = Some(k);
                                        }
                                    }
                                    Err(e) => app.vault_message = Some(format!("Error: {e}")),
                                }
                            }
                        }
                    }
                    (KeyCode::Char('a'), _) if app.screen == Screen::Vault => {
                        app.vault_grant_input = Some(String::new());
                        app.vault_message = None;
                    }
                    (KeyCode::Char('v'), _) if app.screen == Screen::Vault => {
                        if let Some(ref keys) = app.vault_keys {
                            if let Some(key) = keys.get(app.vault_selected_idx) {
                                let config_id = app
                                    .vault_configs
                                    .as_ref()
                                    .and_then(|c| c.get(app.vault_config_idx))
                                    .map(|c| c.id.as_str())
                                    .unwrap_or("default");

                                match app.client.vault_get(config_id, key).await {
                                    Ok(val) => app.vault_revealed = Some(val),
                                    Err(e) => app.vault_message = Some(format!("Error: {e}")),
                                }
                            }
                        }
                    }
                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) if app.screen == Screen::Vault => {
                        if app.vault_selected_idx > 0 {
                            app.vault_selected_idx -= 1;
                        }
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) if app.screen == Screen::Vault => {
                        if let Some(ref keys) = app.vault_keys {
                            if app.vault_selected_idx < keys.len().saturating_sub(1) {
                                app.vault_selected_idx += 1;
                            }
                        }
                    }
                    (KeyCode::Char('g'), _) if app.screen == Screen::Vault => {
                        app.vault_selected_idx = 0;
                    }
                    (KeyCode::Char('G'), _) if app.screen == Screen::Vault => {
                        if let Some(ref keys) = app.vault_keys {
                            app.vault_selected_idx = keys.len().saturating_sub(1);
                        }
                    }
                    (KeyCode::Char('['), _) if app.screen == Screen::Vault => {
                        if app.vault_config_idx > 0 {
                            app.vault_config_idx -= 1;
                            app.vault_selected_idx = 0;
                            app.refresh().await;
                        }
                    }
                    (KeyCode::Char(']'), _) if app.screen == Screen::Vault => {
                        if let Some(ref configs) = app.vault_configs {
                            if app.vault_config_idx < configs.len().saturating_sub(1) {
                                app.vault_config_idx += 1;
                                app.vault_selected_idx = 0;
                                app.refresh().await;
                            }
                        }
                    }
                    (KeyCode::Char('C'), _) if app.screen == Screen::Vault => {
                        app.vault_grant_input = Some("CONFIG: ".to_string());
                        app.vault_message = Some("Enter new config name".to_string());
                    }
                    (KeyCode::Char('d'), _) if app.screen == Screen::Rbac => {
                        if let Some(ref tokens) = app.rbac_tokens {
                            if let Some(token) = tokens.get(app.rbac_selected_idx) {
                                match app.client.token_delete(&token.id).await {
                                    Ok(()) => {
                                        app.rbac_message = Some(format!("Deleted: {}", token.id));
                                        if let Ok(t) = app.client.tokens_list().await {
                                            app.rbac_tokens = Some(t);
                                        }
                                    }
                                    Err(e) => app.rbac_message = Some(format!("Error: {e}")),
                                }
                            }
                        }
                    }
                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) if app.screen == Screen::Rbac => {
                        if app.rbac_selected_idx > 0 {
                            app.rbac_selected_idx -= 1;
                        }
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) if app.screen == Screen::Rbac => {
                        if let Some(ref tokens) = app.rbac_tokens {
                            if app.rbac_selected_idx < tokens.len().saturating_sub(1) {
                                app.rbac_selected_idx += 1;
                            }
                        }
                    }
                    (KeyCode::Char('g'), _) if app.screen == Screen::Rbac => {
                        app.rbac_selected_idx = 0;
                    }
                    (KeyCode::Char('G'), _) if app.screen == Screen::Rbac => {
                        if let Some(ref tokens) = app.rbac_tokens {
                            app.rbac_selected_idx = tokens.len().saturating_sub(1);
                        }
                    }
                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) if app.screen == Screen::Logs => {
                        if app.logs_selected_idx > 0 {
                            app.logs_selected_idx -= 1;
                            app.logs_entries = None;
                            app.refresh().await;
                        }
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) if app.screen == Screen::Logs => {
                        if let Some(ref list) = app.logs_containers {
                            if app.logs_selected_idx < list.len().saturating_sub(1) {
                                app.logs_selected_idx += 1;
                                app.logs_entries = None;
                                app.refresh().await;
                            }
                        }
                    }
                    (KeyCode::Char('t'), _) if app.screen == Screen::Update => {
                        match app.client.update_test().await {
                            Ok(r) => {
                                let cs = r.checksum.as_deref().unwrap_or("—");
                                app.update_message = Some(format!(
                                    "Test: {} | {} | sha256:{}",
                                    if r.ok { "OK" } else { "FAIL" },
                                    r.message,
                                    if cs.len() >= 12 { &cs[..12] } else { cs },
                                ));
                            }
                            Err(e) => app.update_message = Some(format!("Test error: {e}")),
                        }
                    }
                    (KeyCode::Char('u'), _) if app.screen == Screen::Update => {
                        match app.client.update_trigger().await {
                            Ok(()) => {
                                app.update_message = Some(
                                    "Update triggered. Agents will update within 30s.".to_string(),
                                );
                                if let Ok(s) = app.client.update_status().await {
                                    app.update_status = Some(s);
                                }
                            }
                            Err(e) => app.update_message = Some(format!("Trigger error: {e}")),
                        }
                    }
                    _ => {}
                }
            }
        }

        if last_refresh.elapsed() >= POLL_INTERVAL {
            app.refresh().await;
            last_refresh = std::time::Instant::now();
        }
    }

    Ok(())
}

fn draw(f: &mut ratatui::Frame, app: &App) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    // Tabs
    let tab_titles: Vec<Line> = Screen::ALL
        .iter()
        .map(|s| {
            let style = if *s == app.screen {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            Line::from(Span::styled(s.title(), style))
        })
        .collect();

    let selected = Screen::ALL
        .iter()
        .position(|s| *s == app.screen)
        .unwrap_or(0);

    let tabs = Tabs::new(tab_titles)
        .block(Block::default().borders(Borders::ALL).title(" r4a "))
        .select(selected)
        .highlight_style(Style::default().fg(Color::Yellow));

    f.render_widget(tabs, chunks[0]);

    // Content
    match app.screen {
        Screen::Dashboard => {
            ui::dashboard::render(f, chunks[1], &app.nodes, app.fetch_error.as_deref());
        }
        Screen::Manifests => {
            ui::manifests::render(
                f,
                chunks[1],
                app.manifests.as_deref(),
                app.manifests_selected_idx,
                app.manifests_message.as_deref(),
                app.manifests_input.as_deref(),
            );
        }
        Screen::Git => {
            ui::git::render(
                f,
                chunks[1],
                app.git_repos.as_deref(),
                app.git_error.as_deref(),
                app.git_input.as_deref(),
                app.git_message.as_deref(),
            );
        }
        Screen::Vault => {
            ui::vault::render(
                f,
                chunks[1],
                app.vault_configs.as_deref(),
                app.vault_config_idx,
                app.vault_keys.as_deref(),
                app.vault_selected_idx,
                app.rbac_tokens.as_deref(),
                app.vault_input.as_deref(),
                app.vault_grant_input.as_deref(),
                app.vault_message.as_deref(),
                app.vault_revealed.as_deref(),
            );
        }
        Screen::Rbac => {
            ui::rbac::render(
                f,
                chunks[1],
                app.rbac_tokens.as_deref(),
                None,
                app.rbac_message.as_deref(),
            );
        }
        Screen::Logs => {
            ui::logs::render(
                f,
                chunks[1],
                app.logs_containers.as_deref(),
                app.logs_selected_idx,
                app.logs_entries.as_deref(),
                app.logs_message.as_deref(),
            );
        }
        Screen::Update => {
            ui::update::render(
                f,
                chunks[1],
                app.update_status.as_ref(),
                app.update_message.as_deref(),
            );
        }
    }
}
