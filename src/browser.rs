use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers, KeyEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

use crate::app;
use crate::ui::{restore_terminal, setup_terminal, TerminalGuard, Tui};

pub struct Args {
    pub start_dir: PathBuf,
    pub follow: bool,
    pub config_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    ParentDir,
    Dir,
    LogFile,
    OtherFile,
}

struct Entry {
    name: String,
    path: PathBuf,
    kind: EntryKind,
    size: u64,
    modified: Option<SystemTime>,
    hidden: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Normal,
    Filter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChordState {
    None,
    PendingG,
}

struct State {
    cwd: PathBuf,
    entries: Vec<Entry>,
    filtered: Vec<usize>,
    selected: usize,
    viewport_top: usize,
    viewport_height: u16,
    filter_buf: String,
    input_mode: InputMode,
    show_hidden: bool,
    follow: bool,
    marks: BTreeSet<PathBuf>,
    help_open: bool,
    help_scroll: u16,
    status: Option<String>,
    chord: ChordState,
    config_path: Option<PathBuf>,
}

const LOG_EXTS: &[&str] = &["log", "txt", "json", "jsonl", "ndjson", "out", "err", "gz", "trace"];

fn is_log_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| LOG_EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)))
        .unwrap_or(false)
}

#[cfg(windows)]
fn is_hidden_meta(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    (meta.file_attributes() & FILE_ATTRIBUTE_HIDDEN) != 0
}

#[cfg(not(windows))]
fn is_hidden_meta(_meta: &std::fs::Metadata) -> bool {
    false
}

fn load_dir(dir: &Path) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    if dir.parent().is_some() {
        out.push(Entry {
            name: "..".into(),
            path: dir.parent().unwrap().to_path_buf(),
            kind: EntryKind::ParentDir,
            size: 0,
            modified: None,
            hidden: false,
        });
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    let mut tmp: Vec<Entry> = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(|s| s.to_string()) else { continue };
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let hidden_dot = name.starts_with('.');
        let hidden_attr = is_hidden_meta(&meta);
        let hidden = hidden_dot || hidden_attr;
        let kind = if meta.is_dir() {
            EntryKind::Dir
        } else if is_log_ext(&path) {
            EntryKind::LogFile
        } else {
            EntryKind::OtherFile
        };
        let size = if meta.is_file() { meta.len() } else { 0 };
        let modified = meta.modified().ok();
        tmp.push(Entry { name, path, kind, size, modified, hidden });
    }
    tmp.sort_by(|a, b| {
        let ad = a.kind == EntryKind::Dir;
        let bd = b.kind == EntryKind::Dir;
        bd.cmp(&ad).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out.extend(tmp);
    // Cap at 5000 entries to keep things responsive
    out.truncate(5000);
    out
}

fn fuzzy_match(needle: &str, hay: &str) -> bool {
    if needle.is_empty() { return true; }
    let n = needle.to_lowercase();
    let h = hay.to_lowercase();
    let mut hi = h.chars();
    for c in n.chars() {
        loop {
            match hi.next() {
                Some(hc) if hc == c => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

fn humanize_size(n: u64) -> String {
    const UNITS: &[&str] = &["B", "K", "M", "G", "T"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{}{}", n, UNITS[0]) }
    else if v >= 100.0 { format!("{:.0}{}", v, UNITS[u]) }
    else { format!("{:.1}{}", v, UNITS[u]) }
}

fn humanize_age(t: SystemTime) -> String {
    let elapsed = SystemTime::now().duration_since(t).unwrap_or_default().as_secs();
    if elapsed < 60 { format!("{}s", elapsed) }
    else if elapsed < 3600 { format!("{}m", elapsed / 60) }
    else if elapsed < 86400 { format!("{}h", elapsed / 3600) }
    else if elapsed < 86400 * 365 { format!("{}d", elapsed / 86400) }
    else { format!("{}y", elapsed / (86400 * 365)) }
}

impl State {
    fn refresh(&mut self) {
        self.entries = load_dir(&self.cwd);
        self.recompute_filter();
        self.clamp_selection();
    }

    fn recompute_filter(&mut self) {
        self.filtered = self.entries.iter().enumerate()
            .filter(|(_, e)| {
                if !self.show_hidden && e.hidden && e.kind != EntryKind::ParentDir {
                    return false;
                }
                fuzzy_match(&self.filter_buf, &e.name)
            })
            .map(|(i, _)| i)
            .collect();
    }

    fn clamp_selection(&mut self) {
        if self.filtered.is_empty() {
            self.selected = 0;
            self.viewport_top = 0;
            return;
        }
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len() - 1;
        }
        let h = self.viewport_height as usize;
        if h == 0 { return; }
        if self.selected < self.viewport_top {
            self.viewport_top = self.selected;
        } else if self.selected >= self.viewport_top + h {
            self.viewport_top = self.selected + 1 - h;
        }
    }

    fn move_sel(&mut self, delta: i64) {
        if self.filtered.is_empty() { return; }
        let len = self.filtered.len() as i64;
        let new = (self.selected as i64 + delta).clamp(0, len - 1) as usize;
        self.selected = new;
        self.clamp_selection();
    }

    fn current(&self) -> Option<&Entry> {
        self.filtered.get(self.selected).and_then(|&i| self.entries.get(i))
    }

    fn cd(&mut self, target: PathBuf) {
        if let Ok(canon) = std::fs::canonicalize(&target) {
            self.cwd = canon;
        } else {
            self.cwd = target;
        }
        self.filter_buf.clear();
        self.input_mode = InputMode::Normal;
        self.selected = 0;
        self.viewport_top = 0;
        self.refresh();
    }

    fn cd_up(&mut self) {
        if let Some(parent) = self.cwd.parent() {
            let parent = parent.to_path_buf();
            self.cd(parent);
        }
    }

    fn cd_home(&mut self) {
        if let Some(home) = dirs::home_dir() {
            self.cd(home);
        }
    }

    fn toggle_mark(&mut self) {
        let Some(e) = self.current() else { return };
        if matches!(e.kind, EntryKind::Dir | EntryKind::ParentDir) { return; }
        let p = e.path.clone();
        if !self.marks.remove(&p) {
            self.marks.insert(p);
        }
    }

    fn collect_attach_paths(&self) -> Vec<PathBuf> {
        if !self.marks.is_empty() {
            self.marks.iter().cloned().collect()
        } else if let Some(e) = self.current() {
            if matches!(e.kind, EntryKind::LogFile | EntryKind::OtherFile) {
                vec![e.path.clone()]
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    }
}

pub fn run(args: Args) -> Result<()> {
    app::install_panic_hook();
    let _guard = TerminalGuard;
    let mut terminal = setup_terminal()?;

    let mut state = State {
        cwd: args.start_dir.canonicalize().unwrap_or(args.start_dir),
        entries: Vec::new(),
        filtered: Vec::new(),
        selected: 0,
        viewport_top: 0,
        viewport_height: 20,
        filter_buf: String::new(),
        input_mode: InputMode::Normal,
        show_hidden: false,
        follow: args.follow,
        marks: BTreeSet::new(),
        help_open: false,
        help_scroll: 0,
        status: None,
        chord: ChordState::None,
        config_path: args.config_path,
    };
    state.refresh();

    let res = event_loop(&mut state, &mut terminal);
    restore_terminal(&mut terminal)?;
    res
}

fn event_loop(state: &mut State, terminal: &mut Tui) -> Result<()> {
    let tick = Duration::from_millis(50);
    loop {
        terminal.draw(|frame| render(frame, frame.area(), state))?;

        if event::poll(tick)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    match handle_key(state, key)? {
                        Action::None => {}
                        Action::Quit => return Ok(()),
                        Action::Attach(paths) => {
                            if attach(state, terminal, paths)? == app::ExitReason::Quit {
                                return Ok(());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

enum Action {
    None,
    Quit,
    Attach(Vec<PathBuf>),
}

fn handle_key(state: &mut State, key: KeyEvent) -> Result<Action> {
    if state.help_open {
        match key.code {
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Char('q') => {
                state.help_open = false;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(Action::Quit);
            }
            KeyCode::Char('j') | KeyCode::Down => {
                state.help_scroll = state.help_scroll.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                state.help_scroll = state.help_scroll.saturating_sub(1);
            }
            _ => {}
        }
        return Ok(Action::None);
    }

    if state.input_mode == InputMode::Filter {
        match key.code {
            KeyCode::Esc => {
                state.filter_buf.clear();
                state.input_mode = InputMode::Normal;
                state.recompute_filter();
                state.clamp_selection();
            }
            KeyCode::Enter => {
                state.input_mode = InputMode::Normal;
            }
            KeyCode::Backspace => {
                state.filter_buf.pop();
                state.recompute_filter();
                state.selected = 0;
                state.viewport_top = 0;
            }
            KeyCode::Char(c) => {
                state.filter_buf.push(c);
                state.recompute_filter();
                state.selected = 0;
                state.viewport_top = 0;
            }
            _ => {}
        }
        return Ok(Action::None);
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    if state.chord == ChordState::PendingG {
        if let KeyCode::Char('g') = key.code {
            state.chord = ChordState::None;
            state.selected = 0;
            state.viewport_top = 0;
            return Ok(Action::None);
        }
        state.chord = ChordState::None;
    }

    let page = state.viewport_height.saturating_sub(1).max(1) as i64;
    let half = (state.viewport_height as i64 / 2).max(1);

    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(Action::Quit),
        KeyCode::Char('c') if ctrl => return Ok(Action::Quit),
        KeyCode::Esc => {
            state.marks.clear();
            state.status = None;
        }

        KeyCode::Char('j') | KeyCode::Down => state.move_sel(1),
        KeyCode::Char('k') | KeyCode::Up   => state.move_sel(-1),
        KeyCode::Char('d') if ctrl => state.move_sel(half),
        KeyCode::Char('u') if ctrl => state.move_sel(-half),
        KeyCode::Char('f') if ctrl => state.move_sel(page),
        KeyCode::Char('b') if ctrl => state.move_sel(-page),
        KeyCode::PageDown => state.move_sel(page),
        KeyCode::PageUp   => state.move_sel(-page),

        KeyCode::Char('g') => state.chord = ChordState::PendingG,
        KeyCode::Char('G') => {
            if !state.filtered.is_empty() {
                state.selected = state.filtered.len() - 1;
                state.clamp_selection();
            }
        }

        KeyCode::Enter => {
            let paths = state.collect_attach_paths();
            if !paths.is_empty() {
                return Ok(Action::Attach(paths));
            }
            // Otherwise, descend into directory
            if let Some(e) = state.current() {
                match e.kind {
                    EntryKind::Dir | EntryKind::ParentDir => {
                        let target = e.path.clone();
                        state.cd(target);
                    }
                    _ => {}
                }
            }
        }

        KeyCode::Backspace | KeyCode::Left => state.cd_up(),
        KeyCode::Right => {
            if let Some(e) = state.current() {
                if matches!(e.kind, EntryKind::Dir) {
                    let target = e.path.clone();
                    state.cd(target);
                }
            }
        }

        KeyCode::Char('H') => state.cd_home(),

        KeyCode::Char('/') | KeyCode::Char('?') => {
            state.input_mode = InputMode::Filter;
            state.filter_buf.clear();
            state.recompute_filter();
        }

        KeyCode::Char('h') => {
            state.help_open = !state.help_open;
            state.help_scroll = 0;
        }

        KeyCode::Char(' ') => state.toggle_mark(),
        KeyCode::Char('a') => {
            // mark all visible files
            for &i in &state.filtered {
                let e = &state.entries[i];
                if matches!(e.kind, EntryKind::LogFile | EntryKind::OtherFile) {
                    state.marks.insert(e.path.clone());
                }
            }
        }
        KeyCode::Char('A') => state.marks.clear(),

        KeyCode::Char('.') => {
            state.show_hidden = !state.show_hidden;
            state.recompute_filter();
            state.clamp_selection();
        }

        KeyCode::Char('r') => state.refresh(),

        KeyCode::Char('f') => {
            state.follow = !state.follow;
            state.status = Some(format!("follow: {}", if state.follow { "on" } else { "off" }));
        }

        _ => {}
    }

    Ok(Action::None)
}

fn attach(state: &mut State, terminal: &mut Tui, paths: Vec<PathBuf>) -> Result<app::ExitReason> {
    let args = app::Args {
        file_paths: paths,
        follow: state.follow,
        stdin_mode: false,
        config_path: state.config_path.clone(),
        embedded: true,
    };
    let _ = terminal.clear();
    let reason = app::run_attached(args, terminal)?;
    let _ = terminal.clear();
    state.marks.clear();
    if reason == app::ExitReason::Detach {
        state.refresh();
    }
    Ok(reason)
}

fn render(frame: &mut ratatui::Frame, area: Rect, state: &mut State) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    state.viewport_height = chunks[1].height;
    state.clamp_selection();

    render_header(frame, chunks[0], state);
    render_list(frame, chunks[1], state);
    render_footer(frame, chunks[2], state);

    if state.help_open {
        render_help(frame, area, state.help_scroll);
    }
}

fn render_header(frame: &mut ratatui::Frame, area: Rect, state: &State) {
    let cwd = state.cwd.display().to_string();
    let mut spans = vec![
        Span::styled(" lazylog ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(cwd, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
    ];
    if state.follow {
        spans.push(Span::raw("  "));
        spans.push(Span::styled("[follow]", Style::default().fg(Color::Green)));
    }
    if !state.marks.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("[{} marked]", state.marks.len()),
            Style::default().fg(Color::Yellow),
        ));
    }
    if state.show_hidden {
        spans.push(Span::raw("  "));
        spans.push(Span::styled("[hidden]", Style::default().fg(Color::Magenta)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn entry_line<'a>(e: &'a Entry, marked: bool, selected: bool, width: u16) -> ListItem<'a> {
    let (icon, base_style) = match e.kind {
        EntryKind::ParentDir => ("..", Style::default().fg(Color::Gray)),
        EntryKind::Dir => ("[d]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        EntryKind::LogFile => ("log", Style::default().fg(Color::Green)),
        EntryKind::OtherFile => ("   ", Style::default().fg(Color::Gray)),
    };
    let mark = if marked { "*" } else { " " };
    let name = if e.kind == EntryKind::Dir { format!("{}/", e.name) } else { e.name.clone() };
    let mut size_age = String::new();
    if matches!(e.kind, EntryKind::LogFile | EntryKind::OtherFile) {
        size_age.push_str(&humanize_size(e.size));
        if let Some(t) = e.modified {
            size_age.push(' ');
            size_age.push_str(&humanize_age(t));
        }
    }

    // Compose: " * [d] name ............ 12K 3h"
    let pad_target = (width as usize).saturating_sub(2 + 1 + 3 + 1 + size_age.len() + 2);
    let visible_name: String = if name.chars().count() > pad_target {
        name.chars().take(pad_target.saturating_sub(1)).collect::<String>() + "…"
    } else {
        name
    };
    let pad = pad_target.saturating_sub(visible_name.chars().count());

    let mut style = base_style;
    if selected {
        style = style.bg(Color::Rgb(40, 50, 80)).add_modifier(Modifier::BOLD);
    }

    let line = Line::from(vec![
        Span::styled(format!(" {} ", mark), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{} ", icon), style),
        Span::styled(visible_name, style),
        Span::raw(" ".repeat(pad + 1)),
        Span::styled(size_age, Style::default().fg(Color::DarkGray)),
    ]);
    ListItem::new(line)
}

fn render_list(frame: &mut ratatui::Frame, area: Rect, state: &State) {
    let h = area.height as usize;
    let start = state.viewport_top;
    let end = (start + h).min(state.filtered.len());
    let items: Vec<ListItem> = state.filtered[start..end].iter().enumerate().map(|(off, &i)| {
        let e = &state.entries[i];
        let marked = state.marks.contains(&e.path);
        let selected = (start + off) == state.selected;
        entry_line(e, marked, selected, area.width)
    }).collect();

    if items.is_empty() {
        let msg = if state.entries.is_empty() {
            "(empty directory)"
        } else {
            "(no matches)"
        };
        let p = Paragraph::new(Line::from(Span::styled(msg, Style::default().fg(Color::DarkGray))));
        frame.render_widget(p, area);
        return;
    }
    frame.render_widget(List::new(items), area);
}

fn render_footer(frame: &mut ratatui::Frame, area: Rect, state: &State) {
    let line = if state.input_mode == InputMode::Filter {
        Line::from(vec![
            Span::styled("/", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(state.filter_buf.clone()),
            Span::styled("█", Style::default().fg(Color::White)),
        ])
    } else if let Some(ref s) = state.status {
        Line::from(Span::styled(s.clone(), Style::default().fg(Color::Yellow)))
    } else {
        let count = format!("{}/{}", state.selected.saturating_add(1).min(state.filtered.len()), state.filtered.len());
        let hint = "/ filter · h help · Enter open · Space mark · Bksp up · . hidden · f follow · q quit";
        Line::from(vec![
            Span::styled(format!(" {}  ", count), Style::default().fg(Color::Cyan)),
            Span::styled(hint, Style::default().fg(Color::DarkGray)),
        ])
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
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

fn render_help(frame: &mut ratatui::Frame, full_area: Rect, scroll: u16) {
    if full_area.height < 8 || full_area.width < 40 { return; }
    let area = centered_rect(70, 80, full_area);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Browser Help — h/Esc close ")
        .title_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(60, 65, 90)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = browser_help_lines();
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);
}

fn browser_help_lines() -> Vec<Line<'static>> {
    let sections: &[(&str, &[(&str, &str)])] = &[
        ("Navigation", &[
            ("j / k / ↑↓",     "move down / up"),
            ("Ctrl+d / u",     "half page down / up"),
            ("Ctrl+f / b",     "full page down / up"),
            ("PgDn / PgUp",    "full page down / up"),
            ("gg / G",         "top / bottom"),
            ("Enter / →",      "open dir or attach file(s)"),
            ("Backspace / ←",  "parent directory"),
            ("H",              "home directory"),
        ]),
        ("Filter & Marks", &[
            ("/",              "fuzzy filter entries"),
            ("Esc",            "clear filter / clear marks"),
            ("Space",          "toggle mark on file (multi-attach)"),
            ("a / A",          "mark all visible / clear marks"),
            (".",              "toggle hidden files"),
            ("r",              "refresh listing"),
        ]),
        ("Attach", &[
            ("Enter",          "attach selected file (or marks merged)"),
            ("f",              "toggle follow mode for next attach"),
        ]),
        ("Inside viewer", &[
            ("q",              "detach back to browser"),
            ("Q",              "detach back to browser"),
            ("Ctrl+C",         "quit lazylog entirely"),
            ("h",              "viewer help"),
        ]),
        ("Quit", &[
            ("q / Q / Ctrl+C", "quit lazylog"),
            ("? / h",          "toggle this help"),
        ]),
    ];

    let mut out: Vec<Line> = Vec::new();
    for (header, rows) in sections {
        out.push(Line::from(Span::styled(
            (*header).to_string(),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )));
        for (k, d) in *rows {
            out.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{:<18}", k), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled((*d).to_string(), Style::default().fg(Color::Gray)),
            ]));
        }
        out.push(Line::from(""));
    }
    out
}
