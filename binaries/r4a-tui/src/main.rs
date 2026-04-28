mod api_client;
mod ui;

use anyhow::Result;
use api_client::{ApiClient, NodeInfo, RepoInfo, UpdateStatus};
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Tabs},
};
use std::{io, time::Duration};
use ui::Screen;

const POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Parser)]
#[command(name = "r4a-tui", about = "r4a cluster dashboard")]
struct Cli {
    /// Master node API URL
    #[arg(long, default_value = "http://10.42.0.1:8080")]
    master: String,
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
    client: ApiClient,
}

impl App {
    fn new(master_url: &str) -> Self {
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
            client: ApiClient::new(master_url),
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
                Ok(r) => { self.git_repos = Some(r); self.git_error = None; }
                Err(e) => self.git_error = Some(e.to_string()),
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

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, &cli.master).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, master_url: &str) -> Result<()> {
    let mut app = App::new(master_url);
    app.refresh().await;

    let mut last_refresh = std::time::Instant::now();

    loop {
        terminal.draw(|f| draw(f, &app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                // Git input mode intercepts all keys
                if app.screen == Screen::Git && app.git_input.is_some() {
                    match key.code {
                        KeyCode::Esc => { app.git_input = None; }
                        KeyCode::Backspace => {
                            if let Some(ref mut s) = app.git_input { s.pop(); }
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
                            if let Some(ref mut s) = app.git_input { s.push(c); }
                        }
                        _ => {}
                    }
                    continue;
                }

                match (key.code, key.modifiers) {
                    (KeyCode::Char('q'), _)
                    | (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
                    (KeyCode::Tab, _) | (KeyCode::Right, _) => {
                        app.screen = app.screen.next();
                        app.update_message = None;
                    }
                    (KeyCode::BackTab, _) | (KeyCode::Left, _) => {
                        app.screen = app.screen.prev();
                        app.update_message = None;
                    }
                    (KeyCode::Char('n'), _) if app.screen == Screen::Git => {
                        app.git_input = Some(String::new());
                        app.git_message = None;
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
                                app.update_message = Some("Update triggered. Agents will update within 30s.".to_string());
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
        Screen::Rbac => {
            ui::not_implemented::render(f, chunks[1], Screen::Rbac.title());
        }
        Screen::Manifests => {
            ui::not_implemented::render(f, chunks[1], Screen::Manifests.title());
        }
        Screen::Observability => {
            ui::not_implemented::render(f, chunks[1], Screen::Observability.title());
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
