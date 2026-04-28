use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use crossbeam_channel::{bounded, Receiver, Sender};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::layout::{Constraint, Direction, Layout};

use crate::buffer::{Buffer, LineBytes, MMAP_THRESHOLD};
use crate::buffer::chunked::ChunkedBuffer;
use crate::buffer::mmap::MmapBuffer;
use crate::buffer::ring::{RingBuffer, DEFAULT_RING_CAPACITY};
use crate::filter::view::{FilterMsg, FilterSource, FilterWorker, WorkerCmd};
use crate::filter::{FilterState, LevelRegistry};
use crate::index::builder::{IndexBuilder, IndexMsg};
use crate::parser::{detect_format, parse_line, FormatHint, LogLine};
use crate::poller::{FilePollEvent, FilePoller};
use crate::search::query::SearchQuery;
use crate::search::SearchEngine;
use crate::ui::{restore_terminal, setup_terminal, TerminalGuard, Tui};
use crate::ui::searchbar::InputMode;
use crate::ui::target_popup::TargetPopup;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum KeyState {
    Normal,
    PendingG,
}

pub enum BufferKind {
    Mmap(MmapBuffer),
    Chunked(ChunkedBuffer),
    Ring(RingBuffer),
}

impl Buffer for BufferKind {
    fn read_line(&self, line_no: u64) -> Option<LineBytes> {
        match self {
            BufferKind::Mmap(b)    => b.read_line(line_no),
            BufferKind::Chunked(b) => b.read_line(line_no),
            BufferKind::Ring(b)    => b.read_line(line_no),
        }
    }
    fn line_count(&self) -> u64 {
        match self {
            BufferKind::Mmap(b)    => b.line_count(),
            BufferKind::Chunked(b) => b.line_count(),
            BufferKind::Ring(b)    => b.line_count(),
        }
    }
    fn byte_offset(&self, line_no: u64) -> u64 {
        match self {
            BufferKind::Mmap(b)    => b.byte_offset(line_no),
            BufferKind::Chunked(b) => b.byte_offset(line_no),
            BufferKind::Ring(b)    => b.byte_offset(line_no),
        }
    }
}

pub struct AppState {
    pub buffer: BufferKind,
    pub format: FormatHint,
    pub file_path: Option<PathBuf>,
    pub file_len: u64,

    pub index_done: bool,
    pub index_progress: f64,

    pub viewport_top: u64,
    pub viewport_height: u16,
    pub selected: usize,
    pub follow_mode: bool,

    pub filter_state: FilterState,
    pub registry: LevelRegistry,
    pub filter_view: Vec<u32>,
    pub filter_computing: bool,
    pub filter_generation: u64,

    pub search_query: Option<SearchQuery>,
    /// (line_no, byte_start, byte_end) per match
    pub search_matches: Vec<(u64, usize, usize)>,
    pub search_cursor: usize,
    pub search_truncated: bool, // >SEARCH_LIMIT matches found

    pub key_state: KeyState,
    pub input_mode: InputMode,
    pub input_buf: String,

    pub target_popup: Option<TargetPopup>,

    pub worker_cmd_tx: Sender<WorkerCmd>,
    pub filter_rx: Receiver<FilterMsg>,
    pub index_rx: Receiver<IndexMsg>,
    pub tail_index_rx: Option<Receiver<IndexMsg>>,
    pub poller_rx: Option<Receiver<FilePollEvent>>,
    pub stdin_rx: Option<Receiver<Vec<u8>>>,
}

impl AppState {
    fn visible_line_count(&self) -> u64 {
        if self.filter_view.is_empty() {
            self.buffer.line_count()
        } else {
            self.filter_view.len() as u64
        }
    }

    fn logical_to_physical(&self, logical: u64) -> u64 {
        if self.filter_view.is_empty() {
            logical
        } else {
            self.filter_view.get(logical as usize).copied().unwrap_or(0) as u64
        }
    }

    fn ensure_viewport_valid(&mut self) {
        let total = self.visible_line_count();
        if total == 0 { self.viewport_top = 0; return; }
        let max_top = total.saturating_sub(self.viewport_height as u64);
        if self.viewport_top > max_top { self.viewport_top = max_top; }
    }

    fn scroll_down(&mut self, n: u64) {
        let max_top = self.visible_line_count().saturating_sub(self.viewport_height as u64);
        self.viewport_top = (self.viewport_top + n).min(max_top);
    }

    fn scroll_up(&mut self, n: u64) {
        self.viewport_top = self.viewport_top.saturating_sub(n);
    }

    fn goto_bottom(&mut self) {
        let total = self.visible_line_count();
        self.viewport_top = total.saturating_sub(self.viewport_height as u64);
    }

    fn goto_top(&mut self) { self.viewport_top = 0; }
    fn half_page(&self) -> u64 { (self.viewport_height as u64 / 2).max(1) }
    fn full_page(&self) -> u64 { (self.viewport_height as u64).saturating_sub(1).max(1) }

    fn search_next(&mut self) {
        if self.search_matches.is_empty() { return; }
        self.search_cursor = (self.search_cursor + 1) % self.search_matches.len();
        let (line_no, _, _) = self.search_matches[self.search_cursor];
        self.jump_to_line(line_no);
    }

    fn search_prev(&mut self) {
        if self.search_matches.is_empty() { return; }
        if self.search_cursor == 0 {
            self.search_cursor = self.search_matches.len() - 1;
        } else {
            self.search_cursor -= 1;
        }
        let (line_no, _, _) = self.search_matches[self.search_cursor];
        self.jump_to_line(line_no);
    }

    fn jump_to_line(&mut self, phys_line: u64) {
        let logical = if self.filter_view.is_empty() {
            phys_line
        } else {
            self.filter_view.iter()
                .position(|&l| l as u64 >= phys_line)
                .map(|i| i as u64)
                .unwrap_or(0)
        };
        let half = self.half_page();
        self.viewport_top = logical.saturating_sub(half);
        self.ensure_viewport_valid();
    }

    const SEARCH_LIMIT: usize = 9_999;

    fn run_search(&mut self, pattern: String, forward: bool) {
        let Ok(query) = SearchQuery::new(pattern, false) else { return };
        let from = self.logical_to_physical(self.viewport_top);
        let result = if forward {
            SearchEngine::search_forward(&self.buffer, from + 1, &query)
        } else {
            SearchEngine::search_backward(&self.buffer, from.saturating_sub(1), &query)
        };
        let total = self.buffer.line_count();
        let (matches, truncated) = SearchEngine::collect_matches(&self.buffer, &query, 0, total, Self::SEARCH_LIMIT);
        if let Some((found_line, _, _)) = result {
            self.search_cursor = matches.iter().position(|&(ln, _, _)| ln == found_line).unwrap_or(0);
        }
        self.search_truncated = truncated;
        self.search_matches = matches;
        self.search_query = Some(query);
        if let Some(&(line_no, _, _)) = self.search_matches.get(self.search_cursor) {
            self.jump_to_line(line_no);
        }
    }

    fn clear_search(&mut self) {
        self.search_query = None;
        self.search_matches.clear();
        self.search_cursor = 0;
        self.search_truncated = false;
    }

    /// Returns the active match (line_no, byte_start, byte_end) if any.
    pub fn active_match(&self) -> Option<(u64, usize, usize)> {
        self.search_matches.get(self.search_cursor).copied()
    }

    fn trigger_filter_recompute(&mut self) {
        self.filter_generation += 1;
        self.filter_computing = true;
        self.filter_view.clear();

        let source = match (&self.buffer, &self.file_path) {
            (BufferKind::Ring(rb), _) => {
                let lines: Vec<Vec<u8>> = (0..rb.line_count())
                    .filter_map(|i| rb.read_line(i))
                    .collect();
                FilterSource::Lines(lines)
            }
            (_, Some(path)) => FilterSource::File(path.clone()),
            _ => return,
        };

        let _ = self.worker_cmd_tx.send(WorkerCmd::RecomputeFilter {
            generation: self.filter_generation,
            filter: self.filter_state.clone(),
            format: self.format,
            source,
        });
    }

    fn visible_lines(&self) -> Vec<LogLine> {
        let height = self.viewport_height as u64;
        let start = self.viewport_top;
        let end = (start + height).min(self.visible_line_count());
        (start..end)
            .filter_map(|logical| {
                let phys = self.logical_to_physical(logical);
                let bytes = self.buffer.read_line(phys)?;
                Some(parse_line(&bytes, phys, self.format))
            })
            .collect()
    }

    /// Parse first N lines to discover non-standard log levels.
    fn discover_levels(&mut self) {
        let sample = self.buffer.line_count().min(1000);
        for ln in 0..sample {
            if let Some(bytes) = self.buffer.read_line(ln) {
                let log_line = parse_line(&bytes, ln, self.format);
                if let Some(level) = log_line.level {
                    self.registry.discover(level);
                }
            }
        }
    }
}

pub struct Args {
    pub file_path: Option<PathBuf>,
    pub follow: bool,
    pub stdin_mode: bool,
}

pub fn run(args: Args) -> Result<()> {
    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
        );
        orig_hook(info);
    }));
    let _guard = TerminalGuard;

    // Spawn stdin reader BEFORE enabling raw mode (avoids crossterm conflict)
    let stdin_rx = if args.stdin_mode {
        let (tx, rx) = bounded::<Vec<u8>>(4096);
        std::thread::spawn(move || {
            use std::io::BufRead;
            let stdin = std::io::stdin();
            let locked = stdin.lock();
            let mut reader = std::io::BufReader::new(locked);
            let mut line = Vec::new();
            loop {
                line.clear();
                match reader.read_until(b'\n', &mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if line.last() == Some(&b'\n') { line.pop(); }
                        if line.last() == Some(&b'\r') { line.pop(); }
                        if tx.send(line.clone()).is_err() { break; }
                    }
                    Err(_) => break,
                }
            }
        });
        Some(rx)
    } else {
        None
    };

    let (buffer, format, file_len) = build_buffer(&args)?;
    let needs_index = matches!(buffer, BufferKind::Chunked(_));

    let (worker_cmd_tx, worker_cmd_rx) = bounded::<WorkerCmd>(16);
    let (filter_tx, filter_rx) = bounded::<FilterMsg>(128);
    let (index_tx, index_rx) = bounded::<IndexMsg>(128);

    FilterWorker::spawn(worker_cmd_rx, filter_tx);

    if needs_index {
        if let Some(ref path) = args.file_path {
            IndexBuilder::spawn(path.clone(), 0, 0, index_tx);
        }
    }

    let poller_rx = if args.follow {
        if let Some(ref path) = args.file_path {
            let (ptx, prx) = bounded::<FilePollEvent>(32);
            FilePoller::spawn(path.clone(), ptx, 150);
            Some(prx)
        } else { None }
    } else { None };

    let mut app = AppState {
        buffer,
        format,
        file_path: args.file_path,
        file_len,
        index_done: !needs_index,
        index_progress: if needs_index { 0.0 } else { 1.0 },
        viewport_top: 0,
        viewport_height: 20,
        selected: 0,
        follow_mode: args.follow,
        filter_state: FilterState::new(),
        registry: LevelRegistry::new(),
        filter_view: Vec::new(),
        filter_computing: false,
        filter_generation: 0,
        search_query: None,
        search_matches: Vec::new(),
        search_cursor: 0,
        search_truncated: false,
        target_popup: None,
        key_state: KeyState::Normal,
        input_mode: InputMode::Normal,
        input_buf: String::new(),
        worker_cmd_tx,
        filter_rx,
        index_rx,
        tail_index_rx: None,
        poller_rx,
        stdin_rx,
    };

    // Discover non-standard levels from first sample of lines
    app.discover_levels();

    let mut terminal = setup_terminal()?;
    event_loop(&mut app, &mut terminal)?;
    restore_terminal(&mut terminal)?;
    Ok(())
}

fn build_buffer(args: &Args) -> Result<(BufferKind, FormatHint, u64)> {
    if args.stdin_mode {
        return Ok((BufferKind::Ring(RingBuffer::new(DEFAULT_RING_CAPACITY)), FormatHint::Text, 0));
    }
    let path = args.file_path.as_ref()
        .ok_or_else(|| anyhow::anyhow!("no file path"))?;
    let meta = std::fs::metadata(path)?;
    let file_len = meta.len();

    if file_len <= MMAP_THRESHOLD && !args.follow {
        match MmapBuffer::open(path) {
            Ok(buf) => {
                let hint = buf.read_line(0).map(|b| detect_format(&b)).unwrap_or(FormatHint::Text);
                return Ok((BufferKind::Mmap(buf), hint, file_len));
            }
            Err(_) => {}
        }
    }

    let hint = peek_format(path);
    let buf = ChunkedBuffer::open(path.clone())?;
    Ok((BufferKind::Chunked(buf), hint, file_len))
}

fn peek_format(path: &PathBuf) -> FormatHint {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else { return FormatHint::Text };
    let mut buf = vec![0u8; 4096];
    let n = f.read(&mut buf).unwrap_or(0);
    detect_format(&buf[..n])
}

fn event_loop(app: &mut AppState, terminal: &mut Tui) -> Result<()> {
    let tick = Duration::from_millis(16);
    loop {
        drain_stdin(app);
        drain_index(app);
        drain_filter(app);
        drain_poller(app);

        terminal.draw(|frame| {
            let area = frame.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)])
                .split(area);

            app.viewport_height = chunks[1].height;

            let fname = app.file_path.as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("<stdin>");
            let prog = if app.index_done { None } else { Some(app.index_progress) };
            let cur_line = app.logical_to_physical(app.viewport_top);
            let total = app.buffer.line_count();

            crate::ui::statusbar::render(
                frame, chunks[0], fname, cur_line, total,
                app.follow_mode, prog, &app.filter_state, &app.registry,
            );

            let visible = app.visible_lines();
            let active_match = app.active_match();
            crate::ui::viewport::render(
                frame, chunks[1], &visible, Some(app.selected),
                app.search_query.as_ref(), active_match,
            );

            let search_pat = app.search_query.as_ref().map(|q| q.pattern.as_str()).unwrap_or("");
            crate::ui::searchbar::render(
                frame, chunks[2], &app.input_mode, &app.input_buf,
                search_pat,
                app.search_matches.len(), app.search_cursor, app.search_truncated,
            );

            // Popup overlay (rendered last, on top)
            if let Some(ref popup) = app.target_popup {
                crate::ui::target_popup::render(frame, area, popup);
            }
        })?;

        if event::poll(tick)? {
            match event::read()? {
                Event::Key(key) if key.kind == crossterm::event::KeyEventKind::Press => {
                    if handle_key(app, key) { return Ok(()); }
                }
                Event::Mouse(mouse) => handle_mouse(app, mouse),
                _ => {}
            }
        }
    }
}

fn handle_key(app: &mut AppState, key: KeyEvent) -> bool {
    // Popup takes priority — never propagates quit signal
    if app.target_popup.is_some() {
        handle_popup_key(app, key);
        return false;
    }

    // Handle input modes first
    match &app.input_mode {
        InputMode::SearchForward | InputMode::SearchBackward => {
            let forward = app.input_mode == InputMode::SearchForward;
            match key.code {
                KeyCode::Enter => {
                    let pattern = app.input_buf.drain(..).collect::<String>();
                    app.input_mode = InputMode::Normal;
                    if !pattern.is_empty() { app.run_search(pattern, forward); }
                }
                KeyCode::Esc       => { app.input_buf.clear(); app.input_mode = InputMode::Normal; }
                KeyCode::Backspace => { app.input_buf.pop(); }
                KeyCode::Char(c)   => { app.input_buf.push(c); }
                _ => {}
            }
            return false;
        }
        InputMode::CommandLine => {
            match key.code {
                KeyCode::Enter => {
                    let input = app.input_buf.drain(..).collect::<String>();
                    app.input_mode = InputMode::Normal;
                    if let Ok(n) = input.parse::<u64>() {
                        let target = n.saturating_sub(1);
                        app.viewport_top = target.min(app.visible_line_count().saturating_sub(1));
                        app.ensure_viewport_valid();
                    }
                }
                KeyCode::Esc       => { app.input_buf.clear(); app.input_mode = InputMode::Normal; }
                KeyCode::Backspace => { app.input_buf.pop(); }
                KeyCode::Char(c) if c.is_ascii_digit() => { app.input_buf.push(c); }
                _ => {}
            }
            return false;
        }
        InputMode::FilterCrate => {
            match key.code {
                KeyCode::Enter => {
                    let prefix = app.input_buf.drain(..).collect::<String>();
                    app.input_mode = InputMode::Normal;
                    if prefix.is_empty() {
                        app.filter_state.crate_prefixes.clear();
                    } else {
                        app.filter_state.crate_prefixes = vec![prefix];
                    }
                    if !app.filter_state.is_active() {
                        app.filter_view.clear();
                    } else {
                        app.trigger_filter_recompute();
                    }
                }
                KeyCode::Esc       => { app.input_buf.clear(); app.input_mode = InputMode::Normal; }
                KeyCode::Backspace => { app.input_buf.pop(); }
                KeyCode::Char(c)   => { app.input_buf.push(c); }
                _ => {}
            }
            return false;
        }
        InputMode::Normal => {}
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // Handle PendingG: any non-g key resets the chord
    if app.key_state == KeyState::PendingG {
        match key.code {
            KeyCode::Char('g') => {
                app.key_state = KeyState::Normal;
                app.goto_top();
                return false;
            }
            _ => {
                app.key_state = KeyState::Normal;
                // Fall through to process the key normally
            }
        }
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => return true,
        KeyCode::Char('c') if ctrl => return true,
        KeyCode::Esc => { app.clear_search(); }

        KeyCode::Char('j') | KeyCode::Down  => app.scroll_down(1),
        KeyCode::Char('k') | KeyCode::Up    => app.scroll_up(1),
        KeyCode::Char('d') if ctrl => { let h = app.half_page(); app.scroll_down(h); }
        KeyCode::Char('u') if ctrl => { let h = app.half_page(); app.scroll_up(h); }
        KeyCode::Char('f') if ctrl => { let p = app.full_page(); app.scroll_down(p); }
        KeyCode::Char('b') if ctrl => { let p = app.full_page(); app.scroll_up(p); }
        KeyCode::PageDown => { let p = app.full_page(); app.scroll_down(p); }
        KeyCode::PageUp   => { let p = app.full_page(); app.scroll_up(p); }

        KeyCode::Char('g') => { app.key_state = KeyState::PendingG; }
        KeyCode::Char('G') => app.goto_bottom(),

        KeyCode::Char('/') => { app.input_mode = InputMode::SearchForward;  app.input_buf.clear(); }
        KeyCode::Char('?') => { app.input_mode = InputMode::SearchBackward; app.input_buf.clear(); }
        KeyCode::Char('n') => app.search_next(),
        KeyCode::Char('N') => app.search_prev(),
        KeyCode::F(3) if key.modifiers.contains(KeyModifiers::SHIFT) => app.search_prev(),
        KeyCode::F(3) => app.search_next(),

        KeyCode::Char('f') => {
            app.follow_mode = !app.follow_mode;
            if app.follow_mode { app.goto_bottom(); }
        }

        KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
            let key_idx = (c as u8 - b'0') as usize;
            let registry = app.registry.clone();
            app.filter_state.toggle_key(key_idx, &registry);
            if !app.filter_state.is_active() {
                app.filter_view.clear();
            } else {
                app.trigger_filter_recompute();
            }
        }

        KeyCode::Char('t') => {
            open_target_popup(app);
        }

        KeyCode::Char(':') => {
            app.input_mode = InputMode::CommandLine;
            app.input_buf.clear();
        }

        _ => {}
    }

    false
}

fn handle_mouse(app: &mut AppState, mouse: crossterm::event::MouseEvent) {
    match mouse.kind {
        MouseEventKind::ScrollDown => app.scroll_down(3),
        MouseEventKind::ScrollUp   => app.scroll_up(3),
        MouseEventKind::Down(MouseButton::Left) => {
            app.selected = mouse.row.saturating_sub(1) as usize;
        }
        _ => {}
    }
}

fn open_target_popup(app: &mut AppState) {
    let existing = app.filter_state.crate_prefixes.clone();
    let current_depth = if existing.is_empty() { 2 } else {
        existing[0].split("::").count().max(1)
    };
    let targets = collect_targets(app);
    app.target_popup = Some(TargetPopup::new(targets, &existing, current_depth));
}

fn collect_targets(app: &AppState) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut set = BTreeSet::new();
    // Cap at 50k to keep the operation fast even on large files.
    // For chunked buffers line_count() only reflects what's been indexed so far.
    let limit = app.buffer.line_count().min(50_000);
    for ln in 0..limit {
        let Some(bytes) = app.buffer.read_line(ln) else { continue };
        if bytes.is_empty() { continue }
        let log_line = crate::parser::parse_line(&bytes, ln, app.format);
        if let Some(t) = log_line.target {
            set.insert(t);
        }
    }
    set.into_iter().collect()
}

/// Returns true if the key was consumed by the popup.
fn handle_popup_key(app: &mut AppState, key: crossterm::event::KeyEvent) -> bool {
    let Some(ref mut popup) = app.target_popup else { return false };
    let visible_rows = 20usize; // approximate; good enough

    match key.code {
        KeyCode::Esc => {
            app.target_popup = None;
        }
        KeyCode::Enter => {
            let applied = popup.applied();
            app.filter_state.crate_prefixes = applied;
            app.target_popup = None;
            app.viewport_top = 0;
            if !app.filter_state.is_active() {
                app.filter_view.clear();
            } else {
                app.trigger_filter_recompute();
            }
        }
        KeyCode::Char(' ') => { popup.toggle_cursor(); }
        KeyCode::Char('a') | KeyCode::Char('A') => { popup.select_all(); }
        KeyCode::Char('n') | KeyCode::Char('N') => { popup.select_none(); }
        KeyCode::Up   | KeyCode::Char('k') => { popup.move_up(); }
        KeyCode::Down | KeyCode::Char('j') => { popup.move_down(visible_rows); }
        KeyCode::Left  | KeyCode::Char('[') => { popup.depth_dec(); }
        KeyCode::Right | KeyCode::Char(']') => { popup.depth_inc(); }
        _ => {}
    }
    true
}

fn drain_stdin(app: &mut AppState) {
    let Some(ref rx) = app.stdin_rx else { return };
    let mut pushed = false;
    loop {
        match rx.try_recv() {
            Ok(line) => {
                if let BufferKind::Ring(ref mut rb) = app.buffer {
                    rb.push(line);
                    pushed = true;
                }
            }
            Err(_) => break,
        }
    }
    if pushed && app.follow_mode {
        app.goto_bottom();
    }
}

fn drain_index(app: &mut AppState) {
    // Drain both primary and tail index receivers
    for pass in 0..2u8 {
        loop {
            let msg = if pass == 0 {
                app.index_rx.try_recv().ok()
            } else {
                app.tail_index_rx.as_ref().and_then(|rx| rx.try_recv().ok())
            };
            let Some(msg) = msg else { break };
            match msg {
                IndexMsg::Chunk { base_line, all_offsets, .. } => {
                    if let BufferKind::Chunked(ref mut buf) = app.buffer {
                        buf.append_to_index(base_line, &all_offsets);
                    }
                    // Progress: bytes indexed / total file size
                    if app.file_len > 0 {
                        let bytes_so_far = app.buffer.byte_offset(
                            (base_line + all_offsets.len() as u64).saturating_sub(1)
                        );
                        app.index_progress = (bytes_so_far as f64 / app.file_len as f64).min(1.0);
                    }
                }
                IndexMsg::Done { .. } => {
                    if pass == 0 {
                        app.index_done = true;
                        app.index_progress = 1.0;
                    }
                    // tail_index_rx done is fine; keep it
                }
                IndexMsg::TailChunk { base_line, all_offsets } => {
                    if let BufferKind::Chunked(ref mut buf) = app.buffer {
                        buf.append_to_index(base_line, &all_offsets);
                    }
                }
            }
        }
    }
}

fn drain_filter(app: &mut AppState) {
    loop {
        match app.filter_rx.try_recv() {
            Ok(FilterMsg::Chunk { generation, indices, .. }) => {
                if generation == app.filter_generation {
                    app.filter_view.extend_from_slice(&indices);
                }
            }
            Ok(FilterMsg::Done { generation, .. }) => {
                if generation == app.filter_generation {
                    app.filter_computing = false;
                    app.ensure_viewport_valid();
                }
            }
            Ok(FilterMsg::Cancelled { .. }) => { app.filter_computing = false; }
            Err(_) => break,
        }
    }
}

fn drain_poller(app: &mut AppState) {
    let events: Vec<FilePollEvent> = {
        let Some(ref rx) = app.poller_rx else { return };
        let mut evs = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(ev)  => evs.push(ev),
                Err(_) => break,
            }
        }
        evs
    };

    for ev in events {
        match ev {
            FilePollEvent::Grew { new_len } => {
                if let (BufferKind::Chunked(ref mut buf), Some(path)) =
                    (&mut app.buffer, app.file_path.clone())
                {
                    let old_line_count = buf.line_count();
                    let old_offset = buf.byte_offset(old_line_count.saturating_sub(1));
                    buf.set_file_len(new_len);
                    app.file_len = new_len;

                    // Spawn tail indexer and store receiver so drain_index picks it up
                    let (itx, irx) = bounded::<IndexMsg>(128);
                    IndexBuilder::spawn(path, old_offset, old_line_count, itx);
                    app.tail_index_rx = Some(irx);
                }
                if app.follow_mode { app.goto_bottom(); }
            }
            FilePollEvent::Rotated => {
                if let Some(path) = app.file_path.clone() {
                    if let Ok(new_buf) = ChunkedBuffer::open(path.clone()) {
                        app.buffer = BufferKind::Chunked(new_buf);
                        app.viewport_top = 0;
                        app.index_done = false;
                        let (itx, irx) = bounded::<IndexMsg>(128);
                        app.index_rx = irx;
                        IndexBuilder::spawn(path, 0, 0, itx);
                    }
                }
            }
        }
    }
}
