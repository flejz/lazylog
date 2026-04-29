use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

#[derive(Debug, Clone, PartialEq)]
pub enum TimeField {
    From,
    To,
}

#[derive(Debug, Clone)]
pub struct TimePopup {
    pub from_input: String,
    pub to_input: String,
    pub focused: TimeField,
}

impl TimePopup {
    /// Create a new TimePopup, optionally pre-filling from existing filter state.
    pub fn new(from: Option<String>, to: Option<String>) -> Self {
        Self {
            from_input: from.unwrap_or_default(),
            to_input: to.unwrap_or_default(),
            focused: TimeField::From,
        }
    }

    /// Toggle focus between From and To fields.
    pub fn toggle_focus(&mut self) {
        self.focused = match self.focused {
            TimeField::From => TimeField::To,
            TimeField::To => TimeField::From,
        };
    }

    /// Append a character to the currently focused field.
    pub fn push_char(&mut self, c: char) {
        match self.focused {
            TimeField::From => self.from_input.push(c),
            TimeField::To => self.to_input.push(c),
        }
    }

    /// Delete the last character from the currently focused field.
    pub fn pop_char(&mut self) {
        match self.focused {
            TimeField::From => { self.from_input.pop(); }
            TimeField::To => { self.to_input.pop(); }
        }
    }
}

pub fn render(frame: &mut Frame, full_area: Rect, popup: &TimePopup) {
    if full_area.height < 8 || full_area.width < 30 {
        return;
    }

    let pw: u16 = 64.min(full_area.width.saturating_sub(4));
    let ph: u16 = 9.min(full_area.height.saturating_sub(2));
    let area = centered(pw, ph, full_area);

    frame.render_widget(Clear, area);

    let title = " Time Filter  Tab:switch  Enter:apply  Esc:cancel ";
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(Color::Rgb(160, 160, 175)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(60, 65, 90)));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Layout: hint row, blank, from row, blank, to row, blank, hint row
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // hint
            Constraint::Length(1), // blank
            Constraint::Length(1), // from
            Constraint::Length(1), // blank
            Constraint::Length(1), // to
            Constraint::Min(0),    // rest
        ])
        .split(inner);

    // Hint line
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Formats: ", Style::default().fg(Color::Rgb(100, 100, 115))),
            Span::styled("YYYY-MM-DDTHH:MM:SS", Style::default().fg(Color::Rgb(140, 140, 160))),
            Span::styled("  HH:MM:SS", Style::default().fg(Color::Rgb(140, 140, 160))),
            Span::styled("  -1h  -30m  -2d", Style::default().fg(Color::Rgb(140, 140, 160))),
        ])),
        rows[0],
    );

    // From field
    let from_focused = popup.focused == TimeField::From;
    let from_label_style = if from_focused {
        Style::default().fg(Color::Rgb(80, 190, 230)).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Rgb(100, 100, 120))
    };
    let from_val_style = if from_focused {
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Rgb(160, 160, 175))
    };
    let from_cursor = if from_focused { "▌" } else { "" };
    let from_display = if popup.from_input.is_empty() && !from_focused {
        "(none — no lower bound)".to_string()
    } else {
        format!("{}{}", popup.from_input, from_cursor)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("From: ", from_label_style),
            Span::styled(from_display, from_val_style),
        ])),
        rows[2],
    );

    // To field
    let to_focused = popup.focused == TimeField::To;
    let to_label_style = if to_focused {
        Style::default().fg(Color::Rgb(80, 190, 230)).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Rgb(100, 100, 120))
    };
    let to_val_style = if to_focused {
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Rgb(160, 160, 175))
    };
    let to_cursor = if to_focused { "▌" } else { "" };
    let to_display = if popup.to_input.is_empty() && !to_focused {
        "(none — no upper bound)".to_string()
    } else {
        format!("{}{}", popup.to_input, to_cursor)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("To:   ", to_label_style),
            Span::styled(to_display, to_val_style),
        ])),
        rows[4],
    );
}

fn centered(w: u16, h: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    Rect { x, y, width: w.min(area.width), height: h.min(area.height) }
}
