use r4a_client::RepoInfo;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

pub fn render(
    f: &mut Frame,
    area: Rect,
    repos: Option<&[RepoInfo]>,
    error: Option<&str>,
    input: Option<&str>,
    message: Option<&str>,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(area);

    let block = Block::default()
        .title(" Git Repositories ")
        .borders(Borders::ALL);

    let items: Vec<ListItem> = match (repos, error) {
        (_, Some(e)) => vec![ListItem::new(Line::from(Span::styled(
            format!("error: {e}"),
            Style::default().fg(Color::Red),
        )))],
        (None, _) => vec![ListItem::new(Line::from(Span::styled(
            "Loading...",
            Style::default().fg(Color::DarkGray),
        )))],
        (Some(r), _) if r.is_empty() => vec![ListItem::new(Line::from(Span::styled(
            "No repositories found",
            Style::default().fg(Color::DarkGray),
        )))],
        (Some(r), _) => r
            .iter()
            .map(|repo| {
                ListItem::new(vec![
                    Line::from(Span::styled(
                        repo.name.clone(),
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(Span::styled(
                        format!("  git clone {}", repo.clone_url),
                        Style::default().fg(Color::DarkGray),
                    )),
                    Line::raw(""),
                ])
            })
            .collect(),
    };

    let list = List::new(items).block(block);
    f.render_widget(list, chunks[0]);

    // Bottom bar: input or hint
    let bottom = if let Some(inp) = input {
        let title = " New repository name (Enter=confirm, Esc=cancel) ";
        Paragraph::new(inp)
            .block(Block::default().title(title).borders(Borders::ALL))
            .style(Style::default().fg(Color::Yellow))
    } else {
        let hint = message.unwrap_or("[n] New repo");
        let style = if message.is_some() {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        Paragraph::new(hint)
            .block(Block::default().borders(Borders::ALL))
            .style(style)
    };
    f.render_widget(bottom, chunks[1]);
}
