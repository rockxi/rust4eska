use r4a_client::NodeInfo;
use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table},
};

pub fn render(f: &mut Frame, area: Rect, nodes: &[NodeInfo], error: Option<&str>) {
    let title = if let Some(err) = error {
        format!(" Dashboard — {err} ")
    } else {
        " Dashboard ".to_string()
    };

    let header = Row::new(vec![
        Cell::from("Status").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Name").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("IP").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Role").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("CPU %").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("RAM used").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("RAM total").style(Style::default().add_modifier(Modifier::BOLD)),
    ])
    .style(Style::default().fg(Color::Yellow))
    .height(1);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let rows: Vec<Row> = nodes
        .iter()
        .map(|n| {
            let fmt_mb = |v: Option<u64>| v.map(|x| format!("{x} MB")).unwrap_or_else(|| "—".to_string());

            let role_color = if n.role == "master" { Color::Cyan } else { Color::Green };
            
            let (status_text, status_style) = match n.last_seen {
                Some(ls) if now - ls < 20 => (" ONLINE ", Style::default().bg(Color::Green).fg(Color::Black)),
                _ => (" OFFLINE ", Style::default().bg(Color::Red).fg(Color::White)),
            };

            Row::new(vec![
                Cell::from(status_text).style(status_style),
                Cell::from(n.name.as_str()),
                Cell::from(n.ip.as_str()),
                Cell::from(n.role.as_str()).style(Style::default().fg(role_color)),
                Cell::from(n.cpu_percent.map(|c| format!("{c:.1}%")).unwrap_or_else(|| "—".to_string())),
                Cell::from(fmt_mb(n.ram_used_mb)),
                Cell::from(fmt_mb(n.ram_total_mb)),
            ])
        })
        .collect();

    let table = Table::new(rows, [
        Constraint::Length(10),
        Constraint::Length(25),
        Constraint::Length(15),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(12),
        Constraint::Length(12),
    ])
    .header(header)
    .block(Block::default().title(title).borders(Borders::ALL))
    .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    f.render_widget(table, area);
}
