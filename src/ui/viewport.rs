use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem},
    Frame,
};
use crate::parser::{LogLevel, LogLine};
use crate::search::query::SearchQuery;

// Active match: bright amber on dark bg
const MATCH_ACTIVE_BG: Color = Color::Rgb(190, 145, 0);
const MATCH_ACTIVE_FG: Color = Color::White;
// Other matches in viewport: dim
const MATCH_DIM_BG: Color = Color::Rgb(70, 52, 0);
const MATCH_DIM_FG: Color = Color::Rgb(200, 170, 90);

pub fn render(
    frame: &mut Frame,
    area: Rect,
    lines: &[LogLine],
    selected_idx: Option<usize>,
    query: Option<&SearchQuery>,
    active_match: Option<(u64, usize, usize)>,
    context_lines: &std::collections::HashSet<u64>,
) {
    let items: Vec<ListItem> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| render_line(line, i == selected_idx.unwrap_or(usize::MAX), query, active_match, context_lines))
        .collect();

    let list = List::new(items).block(Block::default());
    frame.render_widget(list, area);
}

fn render_line(
    line: &LogLine,
    selected: bool,
    query: Option<&SearchQuery>,
    active_match: Option<(u64, usize, usize)>,
    context_lines: &std::collections::HashSet<u64>,
) -> ListItem<'static> {
    let is_active_line = active_match.map(|(ln, _, _)| ln == line.line_no).unwrap_or(false);
    let is_context_line = !context_lines.is_empty() && context_lines.contains(&line.line_no);
    let bg = if selected {
        Color::Rgb(45, 45, 55)
    } else if is_context_line {
        Color::Rgb(18, 28, 18) // very faint green — context line
    } else if is_active_line {
        Color::Rgb(35, 28, 10) // very subtle warm tint for active match row
    } else {
        Color::Reset
    };

    let mut spans: Vec<Span> = Vec::new();

    // Context line marker — constant width to avoid alignment breaks
    let marker = if is_context_line {
        Span::styled("▏ ", Style::default().fg(Color::Rgb(40, 80, 40)).bg(bg))
    } else {
        Span::styled("  ", Style::default().bg(bg))
    };
    spans.push(marker);

    // Timestamp (HH:MM:SS)
    let ts = line.timestamp.as_deref()
        .map(|t| {
            let time_part = t.find('T').or_else(|| t.find(' '))
                .map(|i| &t[i+1..])
                .unwrap_or(t);
            &time_part[..time_part.len().min(8)]
        })
        .unwrap_or("");
    spans.push(Span::styled(
        format!("{:8} ", ts),
        Style::default().fg(Color::Rgb(90, 90, 100)).bg(bg),
    ));

    // Level badge
    let (level_str, level_color) = match line.level {
        Some(LogLevel::Error)     => ("ERROR", Color::Red),
        Some(LogLevel::Warn)      => ("WARN ", Color::Yellow),
        Some(LogLevel::Info)      => ("INFO ", Color::Green),
        Some(LogLevel::Debug)     => ("DEBUG", Color::Cyan),
        Some(LogLevel::Trace)     => ("TRACE", Color::Gray),
        Some(LogLevel::Custom(_)) => ("CUST ", Color::Magenta),
        None                      => ("     ", Color::Reset),
    };
    spans.push(Span::styled(
        format!("{} ", level_str),
        Style::default().fg(level_color).add_modifier(Modifier::BOLD).bg(bg),
    ));

    // Target/crate
    if let Some(ref target) = line.target {
        let t = if target.len() > 24 {
            format!("…{} ", &target[target.len()-23..])
        } else {
            format!("{:24} ", target)
        };
        spans.push(Span::styled(t, Style::default().fg(Color::Rgb(90, 90, 100)).bg(bg)));
    } else {
        spans.push(Span::styled(format!("{:25}", ""), Style::default().bg(bg)));
    }

    // Message with search highlighting
    let msg = line.display_message();
    let msg_str = if msg.len() > 200 {
        format!("{}…", &msg[..199])
    } else {
        msg.into_owned()
    };

    if let Some(q) = query {
        let is_active_line = active_match.map(|(ln, _, _)| ln == line.line_no).unwrap_or(false);
        let all_matches = q.find_all(msg_str.as_bytes());

        if all_matches.is_empty() {
            spans.push(Span::styled(msg_str, Style::default().bg(bg)));
        } else {
            // Active line: bright highlight; other match lines: dim
            let (match_bg, match_fg) = if is_active_line {
                (MATCH_ACTIVE_BG, MATCH_ACTIVE_FG)
            } else {
                (MATCH_DIM_BG, MATCH_DIM_FG)
            };
            let mut pos = 0usize;
            for (start, end) in all_matches {
                if pos < start {
                    spans.push(Span::styled(msg_str[pos..start].to_owned(), Style::default().bg(bg)));
                }
                spans.push(Span::styled(
                    msg_str[start..end].to_owned(),
                    Style::default().fg(match_fg).bg(match_bg),
                ));
                pos = end;
            }
            if pos < msg_str.len() {
                spans.push(Span::styled(msg_str[pos..].to_owned(), Style::default().bg(bg)));
            }
        }
    } else {
        spans.push(Span::styled(msg_str, Style::default().bg(bg)));
    }

    ListItem::new(Line::from(spans))
}
