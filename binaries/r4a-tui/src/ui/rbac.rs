use r4a_client::Token;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

pub fn render(
    f: &mut Frame,
    area: Rect,
    tokens: Option<&[Token]>,
    error: Option<&str>,
    message: Option<&str>,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(area);

    let block = Block::default().title(" RBAC Tokens ").borders(Borders::ALL);

    let items: Vec<ListItem> = match (tokens, error) {
        (_, Some(e)) => vec![ListItem::new(Line::from(Span::styled(
            format!("error: {e}"),
            Style::default().fg(Color::Red),
        )))],
        (None, _) => vec![ListItem::new(Line::from(Span::styled(
            "Loading...",
            Style::default().fg(Color::DarkGray),
        )))],
        (Some(t), _) if t.is_empty() => vec![ListItem::new(Line::from(Span::styled(
            "No tokens found",
            Style::default().fg(Color::DarkGray),
        )))],
        (Some(t), _) => t
            .iter()
            .map(|token| {
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(
                            format!("{:<8}", token.username),
                            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("{}", token.id),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]),
                    Line::raw(""),
                ])
            })
            .collect(),
    };

    let list = List::new(items).block(block);
    f.render_widget(list, chunks[0]);

    let bottom = {
        let hint = message.unwrap_or("[d] Delete selected token");
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
