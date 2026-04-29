use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::Sparkline,
    Frame,
};

/// Render a 1-line sparkline showing log-line density per time bucket.
/// `histogram` holds counts per bucket; the sparkline scales to its max.
pub fn render(frame: &mut Frame, area: Rect, histogram: &[u16]) {
    if histogram.is_empty() {
        return;
    }
    let data: Vec<u64> = histogram.iter().map(|&v| v as u64).collect();
    let sparkline = Sparkline::default()
        .data(&data)
        .style(Style::default().fg(Color::Cyan).bg(Color::Rgb(20, 22, 28)));
    frame.render_widget(sparkline, area);
}
