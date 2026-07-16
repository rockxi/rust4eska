use r4a_client::Token;
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
    configs: Option<&[r4a_client::VaultConfig]>,
    config_idx: usize,
    secrets: Option<&[String]>,
    selected_idx: usize,
    tokens: Option<&[Token]>,
    input: Option<&str>,
    grant_input: Option<&str>,
    message: Option<&str>,
    revealed_value: Option<&str>,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(area);

    let config_names = match configs {
        Some(c) => c.iter().map(|cfg| cfg.name.clone()).collect::<Vec<_>>(),
        None => vec!["Loading...".to_string()],
    };

    let config_tabs = ratatui::widgets::Tabs::new(
        config_names
            .iter()
            .map(|s| Line::from(s.as_str()))
            .collect::<Vec<_>>(),
    )
    .block(
        Block::default()
            .title(" Vault Config ( [ / ] to switch, Shift+C to new ) ")
            .borders(Borders::ALL),
    )
    .select(config_idx)
    .highlight_style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    f.render_widget(config_tabs, chunks[0]);

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[1]);

    let block = Block::default()
        .title(" Vault Secrets ")
        .borders(Borders::ALL);

    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(Some(selected_idx));

    let items: Vec<ListItem> = match secrets {
        None => vec![ListItem::new(Line::from(Span::styled(
            "Loading...",
            Style::default().fg(Color::DarkGray),
        )))],
        Some(s) if s.is_empty() => vec![ListItem::new(Line::from(Span::styled(
            "No secrets found",
            Style::default().fg(Color::DarkGray),
        )))],
        Some(s) => s
            .iter()
            .enumerate()
            .map(|(i, key)| {
                let style = if i == selected_idx {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Cyan)
                };
                ListItem::new(vec![
                    Line::from(Span::styled(key.clone(), style)),
                    Line::from(Span::styled(
                        "  ********",
                        Style::default().fg(Color::DarkGray),
                    )),
                    Line::raw(""),
                ])
            })
            .collect(),
    };

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().bg(Color::DarkGray));
    f.render_stateful_widget(list, main_chunks[0], &mut list_state);

    let selected_key = secrets.and_then(|s| s.get(selected_idx));
    let access_block = Block::default()
        .title(" Access Control ")
        .borders(Borders::ALL);

    let mut access_items = vec![];
    if let Some(key) = selected_key {
        access_items.push(ListItem::new(Line::from(vec![
            Span::raw("Tokens with access to "),
            Span::styled(key, Style::default().fg(Color::Yellow)),
            Span::raw(":"),
        ])));
        access_items.push(ListItem::new(Line::raw("")));

        if let Some(toks) = tokens {
            for token in toks {
                access_items.push(ListItem::new(Line::from(vec![Span::styled(
                    format!("  {:<10}", token.username),
                    Style::default().fg(Color::Green),
                )])));
            }
        }
    } else {
        access_items.push(ListItem::new(Line::from(Span::styled(
            "Select a secret to see access",
            Style::default().fg(Color::DarkGray),
        ))));
    }

    let access_list = List::new(access_items).block(access_block);
    f.render_widget(access_list, main_chunks[1]);

    let bottom = if let Some(inp) = input {
        let title = " Set secret key=value (Enter=confirm, Esc=cancel) ";
        Paragraph::new(inp)
            .block(Block::default().title(title).borders(Borders::ALL))
            .style(Style::default().fg(Color::Yellow))
    } else if let Some(inp) = grant_input {
        let title = if inp.starts_with("CONFIG: ") {
            " Create new Vault config (Enter=confirm, Esc=cancel) "
        } else {
            " Grant access to username (Enter=confirm, Esc=cancel) "
        };
        Paragraph::new(inp)
            .block(Block::default().title(title).borders(Borders::ALL))
            .style(Style::default().fg(Color::Magenta))
    } else if let Some(val) = revealed_value {
        Paragraph::new(format!(" Value: {}", val))
            .block(
                Block::default()
                    .title(" Revealed Secret (Press any key to hide) ")
                    .borders(Borders::ALL),
            )
            .style(Style::default().fg(Color::Green))
    } else {
        let hint =
            message.unwrap_or("[n] New | [e] Edit | [d] Delete | [v] View | [a] Grant Access");
        let style = if message.is_some() {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        Paragraph::new(hint)
            .block(Block::default().borders(Borders::ALL))
            .style(style)
    };
    f.render_widget(bottom, chunks[2]);
}
