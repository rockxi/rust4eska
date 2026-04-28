use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

pub fn render(f: &mut Frame, area: Rect, title: &str) {
    let block = Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL);
    let text = Paragraph::new("Not implemented")
        .style(Style::default().fg(Color::DarkGray))
        .block(block);
    f.render_widget(text, area);
}
