use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

#[derive(Debug, Clone, Default)]
pub struct HelpPopup {
    pub scroll: u16,
}

impl HelpPopup {
    pub fn new() -> Self {
        HelpPopup { scroll: 0 }
    }
    pub fn scroll_down(&mut self) { self.scroll = self.scroll.saturating_add(1); }
    pub fn scroll_up(&mut self)   { self.scroll = self.scroll.saturating_sub(1); }
}

/// Two-column listing of all keybindings, grouped by category.
fn sections() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    vec![
        ("Navigation", vec![
            ("j / k / ↑↓",      "scroll up / down"),
            ("Ctrl+d / u",      "half page down / up"),
            ("Ctrl+f / b",      "full page down / up"),
            ("PgDn / PgUp",     "full page down / up"),
            ("gg",              "go to top"),
            ("G",               "go to bottom"),
            (":N <Enter>",      "jump to line number N"),
            ("← / →",           "horizontal scroll"),
            ("Mouse wheel",     "scroll 3 lines"),
            ("Mouse click",     "select row"),
        ]),
        ("Search", vec![
            ("/",               "search forward (regex)"),
            ("?",               "search backward (regex)"),
            ("n / N",           "next / previous match"),
            ("F3 / Shift+F3",   "next / previous match"),
            ("↑ / ↓ in search", "cycle search history"),
            ("c",               "toggle context lines around matches"),
            ("+ / -",           "increase / decrease context size"),
            ("Esc",             "clear search / close popup"),
        ]),
        ("Filters", vec![
            ("1 - 9",           "toggle log level filter"),
            ("t",               "target / crate filter popup"),
            ("T",               "time range filter popup"),
            ("F",               "JSON column picker (JSON only)"),
            ("D",               "toggle dedup repeated lines"),
            ("Ctrl+S",          "save current filter as named preset"),
            ("Ctrl+R",          "load a saved filter preset"),
        ]),
        ("Bookmarks", vec![
            ("m",               "toggle bookmark on selected line"),
            ("[",               "jump to previous bookmark"),
            ("]",               "jump to next bookmark"),
        ]),
        ("View", vec![
            ("f",               "toggle follow mode (tail -f)"),
            ("l",               "toggle line number gutter"),
            ("w",               "toggle word wrap"),
            ("h",               "toggle this help overlay"),
        ]),
        ("Actions", vec![
            ("y",               "copy selected line to clipboard"),
            ("Y",               "copy all visible lines to clipboard"),
            ("e",               "export filtered view to file"),
            ("p",               "JSON detail popup (JSON only)"),
            ("s",               "stats popup (level counts)"),
        ]),
        ("Quit", vec![
            ("q / Q / Ctrl+C",  "quit"),
        ]),
    ]
}

fn build_lines() -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (header, rows) in sections() {
        lines.push(Line::from(Span::styled(
            header.to_string(),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )));
        for (key, desc) in rows {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{:<18}", key),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    desc.to_string(),
                    Style::default().fg(Color::Gray),
                ),
            ]));
        }
        lines.push(Line::from(""));
    }
    lines
}

pub fn render(frame: &mut Frame, full_area: Rect, popup: &HelpPopup) {
    if full_area.height < 8 || full_area.width < 40 {
        return;
    }

    let area = centered_rect(70, 85, full_area);
    frame.render_widget(Clear, area);

    let title = " Help — j/k scroll · h or Esc close ";
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(60, 65, 90)));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let para = Paragraph::new(build_lines())
        .scroll((popup.scroll, 0));
    frame.render_widget(para, inner);
}

/// Returns a Rect centered inside `area`, sized as a percentage of `area`.
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(v[1])[1]
}
