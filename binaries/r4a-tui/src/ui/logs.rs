use r4a_client::LogEntry;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

fn fmt_ts(ts_ms: u64) -> String {
    let secs = ts_ms / 1000;
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    format!("{h:02}:{m:02}:{s:02}")
}

pub fn render(
    f: &mut Frame,
    area: Rect,
    containers: Option<&[(String, String)]>,
    selected_idx: usize,
    entries: Option<&[LogEntry]>,
    message: Option<&str>,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(34), Constraint::Min(0)])
        .split(area);

    // Левая панель: контейнеры, по которым есть логи
    let items: Vec<ListItem> = containers
        .unwrap_or_default()
        .iter()
        .map(|(node, container)| ListItem::new(format!("{node} / {container}")))
        .collect();
    let empty = items.is_empty();

    let list = List::new(items)
        .block(
            Block::default()
                .title(" Containers (j/k) ")
                .borders(Borders::ALL),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut list_state = ListState::default();
    if !empty {
        list_state.select(Some(selected_idx));
    }
    f.render_stateful_widget(list, chunks[0], &mut list_state);

    // Правая панель: tail лога выбранного контейнера
    let title = if let Some(msg) = message {
        format!(" Logs — {msg} ")
    } else {
        " Logs (live, poll 2s) ".to_string()
    };

    let lines: Vec<Line> = match entries {
        None if empty => vec![Line::from("No containers with logs yet.")],
        None => vec![Line::from("Loading...")],
        Some(entries) => {
            // Показываем последние строки, влезающие в панель
            let visible = chunks[1].height.saturating_sub(2) as usize;
            entries
                .iter()
                .skip(entries.len().saturating_sub(visible))
                .map(|e| {
                    let line_style = if e.stream == "stderr" {
                        Style::default().fg(Color::Red)
                    } else if e.line.to_lowercase().contains("error") {
                        Style::default().fg(Color::LightRed)
                    } else if e.line.to_lowercase().contains("warn") {
                        Style::default().fg(Color::Yellow)
                    } else {
                        Style::default()
                    };
                    Line::from(vec![
                        Span::styled(fmt_ts(e.ts_ms), Style::default().fg(Color::DarkGray)),
                        Span::raw(" "),
                        Span::styled(e.line.clone(), line_style),
                    ])
                })
                .collect()
        }
    };

    let logs = Paragraph::new(lines).block(Block::default().title(title).borders(Borders::ALL));
    f.render_widget(logs, chunks[1]);
}
