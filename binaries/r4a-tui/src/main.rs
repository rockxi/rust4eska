mod api_client;
mod ui;

use anyhow::Result;
use api_client::{ApiClient, NodeInfo};
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
    client: ApiClient,
}

impl App {
    fn new(master_url: &str) -> Self {
        Self {
            screen: Screen::Dashboard,
            nodes: vec![],
            fetch_error: None,
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
                match (key.code, key.modifiers) {
                    (KeyCode::Char('q'), _)
                    | (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
                    (KeyCode::Tab, _) | (KeyCode::Right, _) => {
                        app.screen = app.screen.next();
                    }
                    (KeyCode::BackTab, _) | (KeyCode::Left, _) => {
                        app.screen = app.screen.prev();
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
        Screen::Rbac => {
            ui::not_implemented::render(f, chunks[1], Screen::Rbac.title());
        }
        Screen::Manifests => {
            ui::not_implemented::render(f, chunks[1], Screen::Manifests.title());
        }
        Screen::Observability => {
            ui::not_implemented::render(f, chunks[1], Screen::Observability.title());
        }
    }
}
