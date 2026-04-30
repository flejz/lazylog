use std::collections::{BTreeSet, HashSet};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

#[derive(Debug, Clone)]
pub struct TargetPopup {
    /// All unique raw targets discovered from the buffer.
    pub all_targets: Vec<String>,
    /// Currently selected prefixes (shown at `depth`).
    pub selected: HashSet<String>,
    /// Popup cursor (row in display list).
    pub cursor: usize,
    /// Scroll offset for long lists.
    pub scroll: usize,
    /// How many `::` segments to use. 1 = top crate only, 2 = two levels, etc.
    pub depth: usize,
    /// Derived: deduplicated display list at current depth, filtered by `filter_input`.
    pub display: Vec<String>,
    /// Live fuzzy-filter substring (case-insensitive). Empty = no filter.
    pub filter_input: String,
}

impl TargetPopup {
    pub fn new(all_targets: Vec<String>, existing: &[String], depth: usize) -> Self {
        let display = make_display(&all_targets, depth, "");
        let selected: HashSet<String> = existing.iter().cloned().collect();
        Self {
            all_targets,
            selected,
            cursor: 0,
            scroll: 0,
            depth,
            display,
            filter_input: String::new(),
        }
    }

    pub fn depth_inc(&mut self) {
        self.set_depth(self.depth + 1);
    }

    pub fn depth_dec(&mut self) {
        if self.depth > 1 { self.set_depth(self.depth - 1); }
    }

    fn set_depth(&mut self, d: usize) {
        self.depth = d.max(1).min(8);
        self.refresh_display();
    }

    /// Append a character to the filter input and rebuild the display list.
    pub fn push_filter_char(&mut self, c: char) {
        self.filter_input.push(c);
        self.refresh_display();
    }

    /// Drop the last filter char (no-op if empty) and rebuild the display list.
    pub fn pop_filter_char(&mut self) {
        if self.filter_input.pop().is_some() {
            self.refresh_display();
        }
    }

    fn refresh_display(&mut self) {
        self.display = make_display(&self.all_targets, self.depth, &self.filter_input);
        if self.display.is_empty() {
            self.cursor = 0;
            self.scroll = 0;
        } else {
            self.cursor = self.cursor.min(self.display.len() - 1);
            self.scroll = self.scroll.min(self.cursor);
        }
    }

    pub fn move_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            if self.cursor < self.scroll { self.scroll = self.cursor; }
        }
    }

    pub fn move_down(&mut self, visible_rows: usize) {
        if !self.display.is_empty() {
            self.cursor = (self.cursor + 1).min(self.display.len() - 1);
            if self.cursor >= self.scroll + visible_rows {
                self.scroll = self.cursor + 1 - visible_rows;
            }
        }
    }

    pub fn toggle_cursor(&mut self) {
        if let Some(t) = self.display.get(self.cursor).cloned() {
            if self.selected.contains(&t) {
                self.selected.remove(&t);
            } else {
                self.selected.insert(t);
            }
        }
    }

    pub fn select_all(&mut self) {
        self.selected = self.display.iter().cloned().collect();
    }

    pub fn select_none(&mut self) {
        self.selected.clear();
    }

    /// Final list of prefixes to apply.
    pub fn applied(&self) -> Vec<String> {
        let mut v: Vec<String> = self.selected.iter().cloned().collect();
        v.sort();
        v
    }
}

fn make_display(all: &[String], depth: usize, filter: &str) -> Vec<String> {
    let needle = filter.to_lowercase();
    let mut set = BTreeSet::new();
    for t in all {
        let seg: String = t.split("::").take(depth).collect::<Vec<_>>().join("::");
        if needle.is_empty() || seg.to_lowercase().contains(&needle) {
            set.insert(seg);
        }
    }
    set.into_iter().collect()
}

pub fn render(frame: &mut Frame, full_area: Rect, popup: &TargetPopup) {
    if full_area.height < 6 || full_area.width < 20 { return; }

    let pw = ((full_area.width as u32 * 2 / 3) as u16).max(44).min(72);
    let list_rows = (popup.display.len() as u16).max(3).min(20);
    let ph = (list_rows + 4).min(full_area.height.saturating_sub(2)).max(6);
    let area = centered(pw, ph, full_area);

    frame.render_widget(Clear, area);

    let title = format!(
        " Targets  ←/→ depth:{}  Spc:toggle  A:all  N:none  Enter:apply  Esc:cancel ",
        popup.depth
    );
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(Color::Rgb(160, 160, 175)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(60, 65, 90)));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Filter row at top of popup body
    let filter_row = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: 1.min(inner.height),
    };
    let filter_text = format!(" Filter: {}|", popup.filter_input);
    frame.render_widget(
        Paragraph::new(filter_text)
            .style(Style::default().fg(Color::Rgb(180, 200, 220)).bg(Color::Rgb(28, 30, 36))),
        filter_row,
    );

    let list_area = Rect {
        x: inner.x,
        y: inner.y + filter_row.height,
        width: inner.width,
        height: inner.height.saturating_sub(filter_row.height),
    };

    if popup.display.is_empty() {
        let msg = Paragraph::new("No targets match the filter.")
            .style(Style::default().fg(Color::Rgb(100, 100, 115)));
        frame.render_widget(msg, list_area);
        return;
    }

    let inner = list_area;
    let visible = inner.height as usize;
    let items: Vec<ListItem> = popup.display
        .iter()
        .enumerate()
        .skip(popup.scroll)
        .take(visible)
        .map(|(i, t)| {
            let on = popup.selected.contains(t);
            let is_cur = i == popup.cursor;
            let bg = if is_cur { Color::Rgb(38, 42, 62) } else { Color::Reset };
            let check_color = if on { Color::Rgb(80, 210, 110) } else { Color::Rgb(70, 70, 90) };
            let text_color = if is_cur { Color::White } else if on { Color::Rgb(200, 210, 200) } else { Color::Rgb(140, 140, 155) };
            let checkbox = if on { "[✓] " } else { "[ ] " };

            // Dim segments beyond the previous depth for visual depth cue
            let segments: Vec<&str> = t.splitn(100, "::").collect();
            let mut spans = vec![
                Span::styled(checkbox, Style::default().fg(check_color).bg(bg)),
            ];
            for (si, seg) in segments.iter().enumerate() {
                if si > 0 {
                    spans.push(Span::styled("::", Style::default().fg(Color::Rgb(60, 65, 80)).bg(bg)));
                }
                let seg_color = if si + 1 == popup.depth {
                    text_color
                } else {
                    Color::Rgb(100, 105, 120)
                };
                spans.push(Span::styled(seg.to_string(), Style::default().fg(seg_color).bg(bg)));
            }
            if is_cur {
                spans.push(Span::styled(" ◀", Style::default().fg(Color::Rgb(80, 90, 150)).bg(bg)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    frame.render_widget(List::new(items), inner);

    // Scroll indicator
    if popup.display.len() > visible {
        let pct = popup.scroll * 100 / (popup.display.len().saturating_sub(visible)).max(1);
        let indicator = format!("{}/{} {}%", popup.cursor + 1, popup.display.len(), pct);
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
