use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem},
    Frame,
};
use serde_json::Value;
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
    lines: &[(LogLine, usize)],
    selected_idx: Option<usize>,
    query: Option<&SearchQuery>,
    active_match: Option<(u64, usize, usize)>,
    context_lines: &std::collections::HashSet<u64>,
    json_columns: &[String],
    file_info: Option<(&[String], &[ratatui::style::Color])>,
) {
    let items: Vec<ListItem> = lines
        .iter()
        .enumerate()
        .map(|(i, (line, count))| render_line(line, *count, i == selected_idx.unwrap_or(usize::MAX), query, active_match, context_lines, json_columns, file_info))
        .collect();

    let list = List::new(items).block(Block::default());
    frame.render_widget(list, area);
}

fn truncate_col(s: &str) -> String {
    let max = 17usize;
    if s.chars().count() > max {
        let truncated: String = s.chars().take(max).collect();
        format!("{}…", truncated)
    } else {
        s.to_owned()
    }
}

fn render_target_message(
    spans: &mut Vec<Span<'static>>,
    line: &LogLine,
    bg: Color,
    query: Option<&SearchQuery>,
    active_match: Option<(u64, usize, usize)>,
) {
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
}

fn render_line(
    line: &LogLine,
    count: usize,
    selected: bool,
    query: Option<&SearchQuery>,
    active_match: Option<(u64, usize, usize)>,
    context_lines: &std::collections::HashSet<u64>,
    json_columns: &[String],
    file_info: Option<(&[String], &[ratatui::style::Color])>,
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

    // Dedup badge
    if count > 1 {
        spans.push(Span::styled(
            format!("[×{}] ", count),
            Style::default().fg(Color::Rgb(150, 100, 200)).bg(bg),
        ));
    }

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

    // Source file badge (multi-file mode)
    if let Some((names, colors)) = file_info {
        let idx = line.file_idx.min(colors.len().saturating_sub(1));
        let name = names.get(line.file_idx).map(|n| {
            if n.len() > 10 { format!("…{}", &n[n.len()-9..]) } else { format!("{:10}", n) }
        }).unwrap_or_else(|| format!("{:10}", "?"));
        spans.push(Span::styled(
            format!("{} ", name),
            Style::default().fg(colors[idx]).bg(bg),
        ));
    }

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

    if !json_columns.is_empty() && line.raw.first() == Some(&b'{') {
        // Try JSON extraction
        if let Ok(v) = serde_json::from_slice::<Value>(&line.raw) {
            if let Some(obj) = v.as_object() {
                for col in json_columns {
                    let val = match obj.get(col) {
                        Some(Value::String(s)) => truncate_col(s),
                        Some(other) => truncate_col(&other.to_string()),
                        None => truncate_col(""),
                    };
                    spans.push(Span::styled(
                        format!("{:18} ", val),
                        Style::default().fg(Color::Rgb(160, 180, 200)).bg(bg),
                    ));
                }
            } else {
                render_target_message(&mut spans, line, bg, query, active_match);
            }
        } else {
            render_target_message(&mut spans, line, bg, query, active_match);
        }
    } else {
        render_target_message(&mut spans, line, bg, query, active_match);
    }

    ListItem::new(Line::from(spans))
}
