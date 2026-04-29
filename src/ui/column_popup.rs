use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

#[derive(Debug, Clone)]
pub struct ColumnPopup {
    pub fields: Vec<String>,   // all available fields (from discovery)
    pub selected: Vec<String>, // ordered — position = column order
    pub cursor: usize,
    pub scroll: usize,
}

impl ColumnPopup {
    pub fn new(fields: &[String], current: &[String]) -> Self {
        Self {
            fields: fields.to_vec(),
            selected: current.to_vec(),
            cursor: 0,
            scroll: 0,
        }
    }

    pub fn move_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            if self.cursor < self.scroll { self.scroll = self.cursor; }
        }
    }

    pub fn move_down(&mut self, visible_rows: usize) {
        if !self.fields.is_empty() {
            self.cursor = (self.cursor + 1).min(self.fields.len() - 1);
            if self.cursor >= self.scroll + visible_rows {
                self.scroll = self.cursor + 1 - visible_rows;
            }
        }
    }

    pub fn toggle_cursor(&mut self) {
        let Some(field) = self.fields.get(self.cursor).cloned() else { return };
        if let Some(pos) = self.selected.iter().position(|s| s == &field) {
            self.selected.remove(pos);
        } else {
            self.selected.push(field);
        }
    }

    pub fn applied(&self) -> Vec<String> {
        self.selected.clone()
    }
}

pub fn render(frame: &mut Frame, full_area: Rect, popup: &ColumnPopup) {
    if full_area.height < 6 || full_area.width < 20 { return; }

    let pw = ((full_area.width as u32 * 2 / 3) as u16).max(44).min(72);
    let list_rows = (popup.fields.len() as u16).max(3).min(20);
    let ph = (list_rows + 4).min(full_area.height.saturating_sub(2)).max(6);
    let area = centered(pw, ph, full_area);

    frame.render_widget(Clear, area);

    let title = " Columns  Spc:toggle  Enter:apply  Esc:cancel ";
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(Color::Rgb(160, 160, 175)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(60, 65, 90)));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if popup.fields.is_empty() {
        let msg = Paragraph::new("No JSON fields discovered yet.\nNavigate the log to populate.")
            .style(Style::default().fg(Color::Rgb(100, 100, 115)));
        frame.render_widget(msg, inner);
        return;
    }

    let visible = inner.height as usize;
    let items: Vec<ListItem> = popup.fields
        .iter()
        .enumerate()
        .skip(popup.scroll)
        .take(visible)
        .map(|(i, field)| {
            let pos = popup.selected.iter().position(|s| s == field);
            let is_cur = i == popup.cursor;
            let bg = if is_cur { Color::Rgb(38, 42, 62) } else { Color::Reset };

            let (checkbox, check_color) = match pos {
                Some(idx) => (
                    format!("[{}]", idx + 1),
                    Color::Rgb(80, 210, 110),
                ),
                None => (
                    "[ ]".to_string(),
                    Color::Rgb(70, 70, 90),
                ),
            };

            let text_color = if is_cur {
                Color::White
            } else if pos.is_some() {
                Color::Rgb(200, 210, 200)
            } else {
                Color::Rgb(140, 140, 155)
            };

            let mut spans = vec![
                Span::styled(
                    format!("{} ", checkbox),
                    Style::default().fg(check_color).add_modifier(Modifier::BOLD).bg(bg),
                ),
                Span::styled(field.clone(), Style::default().fg(text_color).bg(bg)),
            ];

            if is_cur {
                spans.push(Span::styled(" ◀", Style::default().fg(Color::Rgb(80, 90, 150)).bg(bg)));
            }

            ListItem::new(Line::from(spans))
        })
        .collect();

    frame.render_widget(List::new(items), inner);

    // Scroll indicator
    if popup.fields.len() > visible {
        let pct = popup.scroll * 100 / (popup.fields.len().saturating_sub(visible)).max(1);
        let indicator = format!("{}/{} {}%", popup.cursor + 1, popup.fields.len(), pct);
        let ind_area = Rect {
            x: area.x + area.width.saturating_sub(indicator.len() as u16 + 2),
            y: area.y + area.height.saturating_sub(1),
            width: indicator.len() as u16 + 2,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(format!(" {} ", indicator))
                .style(Style::default().fg(Color::Rgb(100, 100, 120)).bg(Color::Rgb(28, 30, 36))),
            ind_area,
        );
    }
}

fn centered(w: u16, h: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    Rect { x, y, width: w.min(area.width), height: h.min(area.height) }
}
