use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::presets::FilterPreset;

/// Centered popup that lists presets and highlights the cursor row.
/// Enter applies, Esc cancels (handled by the caller).
pub fn render(frame: &mut Frame, area: Rect, presets: &[FilterPreset], cursor: usize) {
    let popup = centered_rect(60, 60, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Load preset (Enter=apply  Esc=cancel  j/k=move) ")
        .style(Style::default().bg(Color::Rgb(20, 22, 28)));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    if presets.is_empty() {
        let p = Paragraph::new("No presets saved yet. Use Ctrl+S in normal mode to save one.")
            .style(Style::default().fg(Color::Rgb(170, 170, 175)));
        frame.render_widget(p, inner);
        return;
    }

    let items: Vec<ListItem> = presets
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let style = if i == cursor {
                Style::default().fg(Color::Black).bg(Color::Rgb(220, 200, 80)).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(220, 220, 230))
            };
            let summary = preset_summary(p);
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {:20} ", p.name), style),
                Span::styled(summary, Style::default().fg(Color::Rgb(120, 120, 140))),
            ]))
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

fn preset_summary(p: &FilterPreset) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !p.filter.show_all_levels {
        parts.push(format!("levels=0x{:x}", p.filter.level_mask));
    }
    if !p.filter.crate_prefixes.is_empty() {
        parts.push(format!("targets={}", p.filter.crate_prefixes.len()));
    }
    if p.filter.time_from.is_some() || p.filter.time_to.is_some() {
        parts.push("time-range".to_string());
    }
    if parts.is_empty() {
        " (no filters)".to_string()
    } else {
        format!(" {}", parts.join(", "))
    }
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
