use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

#[derive(Debug, Clone)]
pub struct JsonPopup {
    pub content: String, // pretty-printed JSON
    pub scroll: u16,
}

impl JsonPopup {
    pub fn new(raw: &[u8]) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_slice(raw).ok()?;
        let pretty = serde_json::to_string_pretty(&v).ok()?;
        Some(JsonPopup { content: pretty, scroll: 0 })
    }

    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }
}

pub fn render(frame: &mut Frame, area: Rect, popup: &JsonPopup) {
    if area.height < 6 || area.width < 20 {
        return;
    }
    let popup_area = centered_rect(80, 80, area);
    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .title(" JSON Detail (j/k scroll, Esc close) ")
        .title_style(Style::default().fg(Color::Rgb(160, 160, 175)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let para = Paragraph::new(popup.content.as_str())
        .block(block)
        .scroll((popup.scroll, 0));
    frame.render_widget(para, popup_area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
