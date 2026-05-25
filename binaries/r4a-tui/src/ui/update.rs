use r4a_client::UpdateStatus;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

pub fn render(f: &mut Frame, area: Rect, status: Option<&UpdateStatus>, message: Option<&str>) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(area);

    let block = Block::default().title(" Update ").borders(Borders::ALL);

    let mut items: Vec<ListItem> = vec![];

    if let Some(s) = status {
        let master_cs = s.master_checksum.as_deref().unwrap_or("not found");
        let master_short = short_cs(master_cs);
        let master_ok = s.master_checksum.is_some();
        let master_color = if master_ok { Color::Green } else { Color::Red };
        items.push(ListItem::new(Line::from(vec![
            Span::styled("master  ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(format!("sha256:{master_short}"), Style::default().fg(master_color)),
            if s.update_pending {
                Span::styled("  [UPDATE PENDING]", Style::default().fg(Color::Yellow))
            } else {
                Span::raw("")
            },
        ])));

        items.push(ListItem::new(Line::raw("")));

        for (ip, agent) in &s.agents {
            let cs = agent.checksum.as_deref().unwrap_or("unknown");
            let cs_short = short_cs(cs);
            let up_to_date = agent.checksum.as_deref() == s.master_checksum.as_deref();
            let status_color = match agent.status.as_str() {
                "updated" => Color::Green,
                "updating" => Color::Yellow,
                "failed" => Color::Red,
                _ if up_to_date => Color::Green,
                _ => Color::DarkGray,
            };
            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("{ip:<15} "), Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(format!("sha256:{cs_short}"), Style::default().fg(status_color)),
                Span::raw(format!("  [{}]", agent.status)),
            ])));
        }

        if s.agents.is_empty() {
            items.push(ListItem::new(Line::from(Span::styled(
                "No agents connected",
                Style::default().fg(Color::DarkGray),
            ))));
        }
    } else {
        items.push(ListItem::new(Line::from(Span::styled(
            "Loading...",
            Style::default().fg(Color::DarkGray),
        ))));
    }

    let list = List::new(items).block(block);
    f.render_widget(list, chunks[0]);

    // Bottom hint / message bar
    let hint_text = if let Some(msg) = message {
        msg.to_string()
    } else {
        " [t] test binary on master   [u] trigger update to all agents ".to_string()
    };
    let hint = Paragraph::new(hint_text)
        .style(Style::default().fg(Color::Cyan))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(hint, chunks[1]);
}

fn short_cs(cs: &str) -> &str {
    if cs.len() >= 12 { &cs[..12] } else { cs }
}
