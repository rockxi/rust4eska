use r4a_client::Manifest;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

pub fn render(
    f: &mut Frame,
    area: Rect,
    manifests: Option<&[Manifest]>,
    selected_idx: usize,
    message: Option<&str>,
    input: Option<&str>,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(area);

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(chunks[0]);

    // Левая панель — список манифестов
    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(selected_idx));

    let items: Vec<ListItem> = match manifests {
        None => vec![ListItem::new(Line::from(Span::styled(
            "Loading...",
            Style::default().fg(Color::DarkGray),
        )))],
        Some(m) if m.is_empty() => vec![ListItem::new(Line::from(Span::styled(
            "No manifests. Press 'n' to create.",
            Style::default().fg(Color::DarkGray),
        )))],
        Some(m) => m
            .iter()
            .enumerate()
            .map(|(i, manifest)| {
                let selected = i == selected_idx;
                let style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };

                let kind = if manifest.container.is_some() {
                    "container"
                } else if manifest.systemd.is_some() {
                    "systemd"
                } else {
                    "unknown"
                };

                let line = Line::from(vec![
                    Span::styled(format!(" {}", manifest.app.name), style),
                    Span::styled(
                        format!(" [{}] @{}", kind, manifest.app.node_selector),
                        if selected {
                            Style::default().fg(Color::Black).bg(Color::Cyan)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        },
                    ),
                ]);
                ListItem::new(line)
            })
            .collect(),
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Manifests (j/k, d=delete) ")
                .borders(Borders::ALL),
        )
        .highlight_style(Style::default());

    f.render_stateful_widget(list, main_chunks[0], &mut list_state);

    // Правая панель — детали выбранного манифеста или форма создания
    if let Some(text) = input {
        // Режим ввода имени нового манифеста
        let content = format!(
            "New manifest name:\n> {}_\n\nEnter to confirm, Esc to cancel.\n\nFormat: app-name\n(will create a template you can edit via API/web)",
            text
        );
        let para = Paragraph::new(content)
            .block(
                Block::default()
                    .title(" Create Manifest ")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::Yellow));
        f.render_widget(para, main_chunks[1]);
    } else {
        let detail_text = match manifests {
            Some(m) if !m.is_empty() => {
                if let Some(manifest) = m.get(selected_idx) {
                    render_manifest_detail(manifest)
                } else {
                    "Select a manifest".to_string()
                }
            }
            _ => "No manifest selected.".to_string(),
        };

        let para = Paragraph::new(detail_text)
            .block(Block::default().title(" Details ").borders(Borders::ALL))
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::White));
        f.render_widget(para, main_chunks[1]);
    }

    // Строка статуса / сообщений
    let status_text = match message {
        Some(m) => m.to_string(),
        None => "n=new  d=delete  j/k=navigate".to_string(),
    };
    let status_style = if message.map(|m| m.starts_with("Error")).unwrap_or(false) {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let status = Paragraph::new(status_text)
        .block(Block::default().borders(Borders::ALL))
        .style(status_style);
    f.render_widget(status, chunks[1]);
}

fn render_manifest_detail(m: &Manifest) -> String {
    let mut out = String::new();

    out.push_str(&format!("Name:          {}\n", m.app.name));
    out.push_str(&format!("Node selector: {}\n", m.app.node_selector));

    if let Some(c) = &m.container {
        out.push_str("\n[container]\n");
        out.push_str(&format!("  image:   {}\n", c.image));
        out.push_str(&format!("  restart: {}\n", c.restart));
        if let Some(cmd) = &c.command {
            out.push_str(&format!("  command: {}\n", cmd.join(" ")));
        }
        if let Some(ports) = &c.ports {
            out.push_str(&format!("  ports:   {}\n", ports.join(", ")));
        }
    }

    if let Some(s) = &m.systemd {
        out.push_str("\n[systemd]\n");
        out.push_str(&format!("  exec: {}\n", s.exec));
        if let Some(u) = &s.user {
            out.push_str(&format!("  user: {}\n", u));
        }
    }

    if let Some(i) = &m.ingress {
        out.push_str("\n[ingress]\n");
        out.push_str(&format!("  domain: {}\n", i.domain));
        out.push_str(&format!("  port:   {}\n", i.container_port));
    }

    if !m.env.is_empty() {
        out.push_str("\n[env]\n");
        let mut env_pairs: Vec<_> = m.env.iter().collect();
        env_pairs.sort_by_key(|(k, _)| k.as_str());
        for (k, v) in env_pairs {
            let display_v = if v.starts_with("vault://") {
                v.clone()
            } else {
                "***".to_string()
            };
            out.push_str(&format!("  {} = {}\n", k, display_v));
        }
    }

    out
}
