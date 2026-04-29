use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

#[derive(Debug, Clone, PartialEq)]
pub enum InputMode {
    Normal,
    SearchForward,
    SearchBackward,
    CommandLine,
    FilterCrate,
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    mode: &InputMode,
    input: &str,
    active_pattern: &str,   // current committed search pattern (may be empty)
    match_count: usize,
    current_match: usize,
    truncated: bool,         // true = matches capped, show "n/many"
    context_mode: bool,
) {
    let content = match mode {
        InputMode::Normal => {
            if !active_pattern.is_empty() {
                // Persistent search indicator
                let counter = if match_count == 0 {
                    "no matches".to_owned()
                } else if truncated {
                    format!("{}/many", current_match + 1)
                } else {
                    format!("{}/{}", current_match + 1, match_count)
                };
                Line::from(vec![
                    Span::styled("/", Style::default().fg(Color::Rgb(160, 130, 50))),
                    Span::styled(
                        active_pattern.to_owned(),
                        Style::default().fg(Color::Rgb(220, 190, 100)),
                    ),
                    Span::styled(
                        format!("  [{}]", counter),
                        Style::default().fg(Color::Rgb(120, 100, 50)),
                    ),
                    Span::styled(
                        if context_mode {
                            "  n:next  N:prev  c:context  +/-:ctx-size  Esc:clear".to_owned()
                        } else {
                            "  n:next  N:prev  c:context  Esc:clear".to_owned()
                        },
                        Style::default().fg(Color::Rgb(70, 70, 80)),
                    ),
                ])
            } else {
                Line::from(Span::styled(
                    " q:quit  /:search  f:follow  1-9:levels  t:target  gg/G:top/bot",
                    Style::default().fg(Color::Rgb(70, 70, 80)),
                ))
            }
        }

        InputMode::SearchForward | InputMode::SearchBackward => {
            let prefix = if *mode == InputMode::SearchForward { "/" } else { "?" };
            let counter = if match_count == 0 && !input.is_empty() {
                Span::styled("  no matches", Style::default().fg(Color::Red))
            } else if match_count > 0 {
                let s = if truncated {
                    format!("  [{}/many]  n:next  N:prev", current_match + 1)
                } else {
                    format!("  [{}/{}]  n:next  N:prev", current_match + 1, match_count)
                };
                Span::styled(s, Style::default().fg(Color::Rgb(120, 100, 50)))
            } else {
                Span::raw("")
            };
            Line::from(vec![
                Span::styled(prefix, Style::default().fg(Color::Rgb(220, 190, 100))),
                Span::styled(input.to_owned(), Style::default().fg(Color::White)),
                Span::styled("_", Style::default().fg(Color::Rgb(220, 190, 100))),
                counter,
            ])
        }

        InputMode::CommandLine => {
            Line::from(vec![
                Span::styled(":", Style::default().fg(Color::White)),
                Span::raw(input.to_owned()),
                Span::styled("_", Style::default().fg(Color::White)),
            ])
        }

        InputMode::FilterCrate => {
            Line::from(vec![
                Span::styled("target: ", Style::default().fg(Color::Cyan)),
                Span::raw(input.to_owned()),
                Span::styled("_", Style::default().fg(Color::Cyan)),
                Span::styled(
                    "   (Enter: apply  Esc: cancel)",
                    Style::default().fg(Color::Rgb(70, 70, 80)),
                ),
            ])
        }
    };

    frame.render_widget(Paragraph::new(content), area);
}
