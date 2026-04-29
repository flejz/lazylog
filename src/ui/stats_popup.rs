use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

#[derive(Debug, Clone)]
pub struct StatsPopup {
    pub lines: Vec<String>,
}

pub fn render(frame: &mut Frame, area: Rect, popup: &StatsPopup) {
    if area.height < 6 || area.width < 20 {
        return;
    }
    let popup_area = centered_rect(60, 70, area);
    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .title(" Stats (Esc close) ")
        .title_style(Style::default().fg(Color::Rgb(160, 160, 175)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let body: Vec<Line> = popup
        .lines
        .iter()
        .map(|s| Line::from(Span::styled(s.clone(), Style::default().fg(Color::Rgb(210, 210, 225)))))
        .collect();

    let para = Paragraph::new(body).block(block);
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
