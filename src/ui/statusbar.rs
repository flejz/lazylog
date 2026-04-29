use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use crate::filter::{FilterState, LevelRegistry};
use crate::parser::LogLevel;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    filename: &str,
    current_line: u64,
    total_lines: u64,
    follow_mode: bool,
    index_progress: Option<f64>, // None = done, Some(0.0..1.0) = indexing
    filter: &FilterState,
    registry: &LevelRegistry,
    context_mode: bool,
    context_size: usize,
) {
    let mut spans: Vec<Span> = Vec::new();

    // App name
    spans.push(Span::styled(
        " lazylog",
        Style::default().fg(Color::Rgb(180, 220, 220)).bg(Color::Rgb(0, 80, 90)).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw(" "));

    // Filename
    let fname = if filename.len() > 40 {
        format!("…{}", &filename[filename.len().saturating_sub(39)..])
    } else {
        filename.to_owned()
    };
    spans.push(Span::styled(fname, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)));
    spans.push(Span::raw(" │ "));

    // Line position
    spans.push(Span::styled(
        format!("{}/{}", current_line + 1, total_lines),
        Style::default().fg(Color::Gray),
    ));
    spans.push(Span::raw(" │ "));

    // Index progress or READY
    if let Some(pct) = index_progress {
        spans.push(Span::styled(
            format!("indexing {:.0}%", pct * 100.0),
            Style::default().fg(Color::Rgb(190, 150, 50)),
        ));
    } else {
        spans.push(Span::styled("READY", Style::default().fg(Color::Rgb(80, 170, 100))));
    }
    spans.push(Span::raw(" │ "));

    // Follow mode
    if follow_mode {
        spans.push(Span::styled(
            "FOLLOW",
            Style::default().fg(Color::White).bg(Color::Rgb(0, 110, 50)).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" │ "));
    }

    // Active level filters
    if !filter.show_all_levels {
        spans.push(Span::raw("lvl:"));
        for (i, level) in registry.levels.iter().enumerate().take(9) {
            let bit = (filter.level_mask >> i) & 1 == 1;
            let key = i + 1;
            let (abbrev, color) = level_display(level);
            if bit {
                spans.push(Span::styled(
                    format!("[{key}:{abbrev}]"),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(
                    format!(" {key}:{abbrev} "),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
        spans.push(Span::raw(" │ "));
    }

    // Crate filter
    if !filter.crate_prefixes.is_empty() {
        spans.push(Span::styled("target:", Style::default().fg(Color::DarkGray)));
        let label = if filter.crate_prefixes.len() == 1 {
            filter.crate_prefixes[0].clone()
        } else {
            format!("{} +{}", filter.crate_prefixes[0], filter.crate_prefixes.len() - 1)
        };
        spans.push(Span::styled(label, Style::default().fg(Color::Cyan)));
        spans.push(Span::raw(" │ "));
    }

    // Context mode indicator
    if context_mode {
        spans.push(Span::styled(
            format!("CTX:±{}", context_size),
            Style::default().fg(Color::Rgb(80, 160, 80)),
        ));
        spans.push(Span::raw(" │ "));
    }

    let line = Line::from(spans);
    let para = Paragraph::new(line)
        .style(Style::default().fg(Color::Rgb(170, 170, 175)).bg(Color::Rgb(28, 30, 36)));
    frame.render_widget(para, area);
}

fn level_display(level: &LogLevel) -> (&'static str, Color) {
    match level {
        LogLevel::Error  => ("ERR", Color::Red),
        LogLevel::Warn   => ("WRN", Color::Yellow),
        LogLevel::Info   => ("INF", Color::Green),
        LogLevel::Debug  => ("DBG", Color::Cyan),
        LogLevel::Trace  => ("TRC", Color::Gray),
        LogLevel::Custom(_) => ("???", Color::Magenta),
    }
}
