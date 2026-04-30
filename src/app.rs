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
use crate::ui::column_popup::ColumnPopup;
use crate::ui::time_popup::TimePopup;
use crate::ui::help_popup::HelpPopup;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum KeyState {
    Normal,
    PendingG,
}

pub enum BufferKind {
    Mmap(MmapBuffer),
    Chunked(ChunkedBuffer),
    Ring(RingBuffer),
    Multi(crate::buffer::multi::MultiBuffer),
}

impl Buffer for BufferKind {
    fn read_line(&self, line_no: u64) -> Option<LineBytes> {
        match self {
            BufferKind::Mmap(b)    => b.read_line(line_no),
            BufferKind::Chunked(b) => b.read_line(line_no),
            BufferKind::Ring(b)    => b.read_line(line_no),
            BufferKind::Multi(b)   => b.read_line(line_no),
        }
    }
    fn line_count(&self) -> u64 {
        match self {
            BufferKind::Mmap(b)    => b.line_count(),
            BufferKind::Chunked(b) => b.line_count(),
            BufferKind::Ring(b)    => b.line_count(),
            BufferKind::Multi(b)   => b.line_count(),
        }
    }
    fn byte_offset(&self, line_no: u64) -> u64 {
        match self {
            BufferKind::Mmap(b)    => b.byte_offset(line_no),
            BufferKind::Chunked(b) => b.byte_offset(line_no),
            BufferKind::Ring(b)    => b.byte_offset(line_no),
            BufferKind::Multi(b)   => b.byte_offset(line_no),
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

    pub context_mode: bool,
    pub context_size: usize, // default 5

    pub key_state: KeyState,
    pub input_mode: InputMode,
    pub input_buf: String,

    pub dedup_enabled: bool,
    pub file_names: Vec<String>,
    pub file_colors: Vec<ratatui::style::Color>,

    pub target_popup: Option<TargetPopup>,
    pub time_popup: Option<TimePopup>,

    pub json_fields: Vec<String>,    // discovered non-standard field names
    pub json_columns: Vec<String>,   // active column selection (ordered)
    pub column_popup: Option<ColumnPopup>,

    pub json_popup: Option<crate::ui::json_popup::JsonPopup>,
    pub stats_popup: Option<crate::ui::stats_popup::StatsPopup>,

    pub bookmarks: std::collections::BTreeSet<u64>,

    pub search_history: std::collections::VecDeque<String>,
    pub search_history_idx: Option<usize>,

    pub h_scroll: u16,

    pub help_open: bool,
    pub help_popup: Option<HelpPopup>,
    pub show_line_numbers: bool,
    pub word_wrap: bool,

    pub histogram: Vec<u16>,
    pub histogram_dirty: bool,

    pub config: crate::config::AppConfig,

    pub preset_name_input: String,
    pub preset_load_popup: Option<Vec<crate::presets::FilterPreset>>,
    pub preset_load_cursor: usize,

    pub worker_cmd_tx: Sender<WorkerCmd>,
    pub filter_rx: Receiver<FilterMsg>,
    pub index_rx: Receiver<IndexMsg>,
    pub tail_index_rx: Option<Receiver<IndexMsg>>,
    pub poller_rx: Option<Receiver<FilePollEvent>>,
    pub stdin_rx: Option<Receiver<Vec<u8>>>,
}

impl AppState {
    pub fn visible_line_count(&self) -> u64 {
        if self.filter_view.is_empty() {
            self.buffer.line_count()
        } else {
            self.filter_view.len() as u64
        }
    }

    pub fn logical_to_physical(&self, logical: u64) -> u64 {
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
        self.h_scroll = 0;
    }

    fn goto_top(&mut self) {
        self.viewport_top = 0;
        self.h_scroll = 0;
    }
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
        self.h_scroll = 0;
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

    fn toggle_bookmark(&mut self) {
        let phys = self.logical_to_physical(self.viewport_top);
        if !self.bookmarks.remove(&phys) {
            self.bookmarks.insert(phys);
        }
    }

    fn bookmark_prev(&mut self) {
        let cur = self.logical_to_physical(self.viewport_top);
        if let Some(&phys) = self.bookmarks.range(..cur).next_back() {
            self.jump_to_line(phys);
        }
    }

    fn bookmark_next(&mut self) {
        let cur = self.logical_to_physical(self.viewport_top);
        if let Some(&phys) = self.bookmarks.range(cur + 1..).next() {
            self.jump_to_line(phys);
        }
    }

    fn clear_search(&mut self) {
        self.search_query = None;
        self.search_matches.clear();
        self.search_cursor = 0;
        self.search_truncated = false;
        self.context_mode = false;
    }

    /// Returns the active match (line_no, byte_start, byte_end) if any.
    pub fn active_match(&self) -> Option<(u64, usize, usize)> {
        self.search_matches.get(self.search_cursor).copied()
    }

    fn trigger_filter_recompute(&mut self) {
        self.filter_generation += 1;
        self.filter_computing = true;
        self.filter_view.clear();
        self.h_scroll = 0;

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

    fn visible_lines(&self) -> Vec<(LogLine, usize)> {
        let height = self.viewport_height as u64;
        let start = self.viewport_top;
        let end = (start + height).min(self.visible_line_count());
        let lines: Vec<LogLine> = (start..end)
            .filter_map(|logical| {
                let phys = self.logical_to_physical(logical);
                let bytes = self.buffer.read_line(phys)?;
                let mut log_line = parse_line(&bytes, phys, self.format);
                if let BufferKind::Multi(ref mb) = self.buffer {
                    log_line.file_idx = mb.file_idx_for(phys);
                }
                Some(log_line)
            })
            .collect();

        if !self.dedup_enabled {
            lines.into_iter().map(|l| (l, 1)).collect()
        } else {
            let mut result: Vec<(LogLine, usize)> = Vec::new();
            for line in lines {
                if let Some(last) = result.last_mut() {
                    if last.0.raw == line.raw {
                        last.1 += 1;
                        continue;
                    }
                }
                result.push((line, 1));
            }
            result
        }
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
    pub file_paths: Vec<PathBuf>,
    pub follow: bool,
    pub stdin_mode: bool,
    pub config_path: Option<PathBuf>,
}

/// Convert "YYYY-MM-DDTHH:MM:SS" key to a pseudo-epoch in seconds.
/// Uses fixed 31-day months — good enough for relative bucketing within one log.
fn ts_key_to_secs(s: &str) -> i64 {
    if s.len() < 19 { return 0; }
    let y: i64 = s[0..4].parse().unwrap_or(1970);
    let mo: i64 = s[5..7].parse().unwrap_or(1);
    let d: i64 = s[8..10].parse().unwrap_or(1);
    let h: i64 = s[11..13].parse().unwrap_or(0);
    let mi: i64 = s[14..16].parse().unwrap_or(0);
    let se: i64 = s[17..19].parse().unwrap_or(0);
    let days = y * 365 + (mo - 1) * 31 + (d - 1);
    days * 86400 + h * 3600 + mi * 60 + se
}

/// Scan up to 50k lines, parse timestamps and bucket counts into `width` slots.
/// Lines without a parseable full timestamp are skipped.
pub fn recompute_histogram(app: &mut AppState, width: usize) {
    if width == 0 {
        app.histogram.clear();
        app.histogram_dirty = false;
        return;
    }
    let total = app.buffer.line_count();
    if total == 0 {
        app.histogram = vec![0u16; width];
        app.histogram_dirty = false;
        return;
    }

    let limit = total.min(50_000);
    let mut secs_vec: Vec<i64> = Vec::with_capacity((limit / 2) as usize);
    for ln in 0..limit {
        let Some(bytes) = app.buffer.read_line(ln) else { continue };
        let log_line = parse_line(&bytes, ln, app.format);
        let Some(ts) = log_line.timestamp else { continue };
        if let Some(key) = crate::time_parse::parse_ts_key(&ts) {
            secs_vec.push(ts_key_to_secs(&key));
        }
    }

    if secs_vec.is_empty() {
        app.histogram = vec![0u16; width];
        app.histogram_dirty = false;
        return;
    }

    let min_s = *secs_vec.iter().min().unwrap();
    let max_s = *secs_vec.iter().max().unwrap();
    let range = (max_s - min_s).max(1);

    let mut counts = vec![0u32; width];
    for s in &secs_vec {
        let bucket = (((s - min_s) as i128 * (width as i128 - 1)) / range as i128) as usize;
        let bucket = bucket.min(width - 1);
        counts[bucket] = counts[bucket].saturating_add(1);
    }

    app.histogram = counts.into_iter().map(|c| c.min(u16::MAX as u32) as u16).collect();
    app.histogram_dirty = false;
}

const FILE_COLORS: &[ratatui::style::Color] = &[
    ratatui::style::Color::Cyan,
    ratatui::style::Color::Yellow,
    ratatui::style::Color::Green,
    ratatui::style::Color::Magenta,
    ratatui::style::Color::Red,
    ratatui::style::Color::Blue,
    ratatui::style::Color::Rgb(255, 165, 0),
    ratatui::style::Color::Rgb(0, 200, 100),
];

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

    // For single-file mode, extract the one path; multi-file sets file_path to None
    let single_path: Option<PathBuf> = if args.file_paths.len() == 1 {
        args.file_paths.first().cloned()
    } else {
        None
    };

    let (buffer, format, file_len) = if args.file_paths.len() > 1 {
        build_multi_buffer(&args.file_paths)?
    } else {
        build_buffer(args.stdin_mode, single_path.as_ref(), args.follow)?
    };
    let needs_index = matches!(buffer, BufferKind::Chunked(_));

    let (worker_cmd_tx, worker_cmd_rx) = bounded::<WorkerCmd>(16);
    let (filter_tx, filter_rx) = bounded::<FilterMsg>(128);
    let (index_tx, index_rx) = bounded::<IndexMsg>(128);

    FilterWorker::spawn(worker_cmd_rx, filter_tx);

    if needs_index {
        if let Some(ref path) = single_path {
            IndexBuilder::spawn(path.clone(), 0, 0, index_tx);
        }
    }

    let poller_rx = if args.follow {
        if let Some(ref path) = single_path {
            let (ptx, prx) = bounded::<FilePollEvent>(32);
            FilePoller::spawn(path.clone(), ptx, 150);
            Some(prx)
        } else { None }
    } else { None };

    let mut app = AppState {
        buffer,
        format,
        file_path: single_path,
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
        dedup_enabled: false,
        context_mode: false,
        context_size: 5,
        file_names: Vec::new(),
        file_colors: Vec::new(),
        target_popup: None,
        time_popup: None,
        json_fields: Vec::new(),
        json_columns: Vec::new(),
        column_popup: None,
        json_popup: None,
        stats_popup: None,
        bookmarks: std::collections::BTreeSet::new(),
        search_history: std::collections::VecDeque::new(),
        search_history_idx: None,
        h_scroll: 0,
        help_open: false,
        help_popup: None,
        show_line_numbers: false,
        word_wrap: false,
        histogram: Vec::new(),
        histogram_dirty: true,
        config: crate::config::AppConfig::load(args.config_path.as_deref()),
        preset_name_input: String::new(),
        preset_load_popup: None,
        preset_load_cursor: 0,
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

    // Populate file names/colors for multi-file mode
    if let BufferKind::Multi(ref mb) = app.buffer {
        app.file_names = mb.file_names.clone();
        let n = mb.file_names.len().min(FILE_COLORS.len());
        app.file_colors = FILE_COLORS[..n].to_vec();
    }

    // Discover non-standard levels from first sample of lines
    app.discover_levels();

    let mut terminal = setup_terminal()?;
    event_loop(&mut app, &mut terminal)?;
    restore_terminal(&mut terminal)?;
    Ok(())
}

fn maybe_decompress(path: &PathBuf) -> Result<std::borrow::Cow<'_, PathBuf>> {
    if path.extension().and_then(|e| e.to_str()) == Some("gz") {
        use std::io::Read;
        let f = std::fs::File::open(path)?;
        let mut decoder = flate2::read::GzDecoder::new(f);
        let mut data = Vec::new();
        decoder.read_to_end(&mut data)?;
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let tmp = std::env::temp_dir().join(stem.as_ref());
        std::fs::write(&tmp, &data)?;
        Ok(std::borrow::Cow::Owned(tmp))
    } else {
        Ok(std::borrow::Cow::Borrowed(path))
    }
}

fn build_buffer(stdin_mode: bool, file_path: Option<&PathBuf>, follow: bool) -> Result<(BufferKind, FormatHint, u64)> {
    if stdin_mode {
        return Ok((BufferKind::Ring(RingBuffer::new(DEFAULT_RING_CAPACITY)), FormatHint::Text, 0));
    }
    let original = file_path
        .ok_or_else(|| anyhow::anyhow!("no file path"))?;
    let resolved = maybe_decompress(original)?;
    let path: &PathBuf = resolved.as_ref();
    let meta = std::fs::metadata(path)?;
    let file_len = meta.len();

    if file_len <= MMAP_THRESHOLD && !follow {
        if let Ok(buf) = MmapBuffer::open(path) {
            let hint = buf.read_line(0).map(|b| detect_format(&b)).unwrap_or(FormatHint::Text);
            return Ok((BufferKind::Mmap(buf), hint, file_len));
        }
    }

    let hint = peek_format(path);
    let buf = ChunkedBuffer::open(path.clone())?;
    Ok((BufferKind::Chunked(buf), hint, file_len))
}

fn build_multi_buffer(paths: &[PathBuf]) -> Result<(BufferKind, FormatHint, u64)> {
    use crate::buffer::multi::MultiBuffer;
    let buf = MultiBuffer::open(paths)?;
    let hint = buf.read_line(0).map(|b| detect_format(&b)).unwrap_or(FormatHint::Text);
    Ok((BufferKind::Multi(buf), hint, 0))
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

            let fname = if app.file_names.len() > 1 {
                format!("{} files", app.file_names.len())
            } else {
                app.file_path.as_ref()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("<stdin>")
                    .to_owned()
            };
            let prog = if app.index_done { None } else { Some(app.index_progress) };
            let cur_line = app.logical_to_physical(app.viewport_top);
            let total = app.buffer.line_count();

            crate::ui::statusbar::render(
                frame, chunks[0], &fname, cur_line, total,
                app.follow_mode, prog, &app.filter_state, &app.registry,
                app.dedup_enabled,
                app.context_mode, app.context_size, &app.json_columns,
                app.bookmarks.len(),
            );

            let context_lines: std::collections::HashSet<u64> = if app.context_mode {
                app.search_matches.iter()
                    .flat_map(|&(ln, _, _)| {
                        let lo = ln.saturating_sub(app.context_size as u64);
                        let hi = ln + app.context_size as u64;
                        lo..=hi
                    })
                    .collect()
            } else {
                std::collections::HashSet::new()
            };

            let visible = app.visible_lines();
            let active_match = app.active_match();
            let file_info = if app.file_names.is_empty() {
                None
            } else {
                Some((app.file_names.as_slice(), app.file_colors.as_slice()))
            };
            crate::ui::viewport::render(
                frame, chunks[1], &visible, Some(app.selected),
                app.search_query.as_ref(), active_match, &context_lines,
                &app.json_columns, file_info,
                &app.bookmarks,
            );

            let search_pat = app.search_query.as_ref().map(|q| q.pattern.as_str()).unwrap_or("");
            crate::ui::searchbar::render(
                frame, chunks[2], &app.input_mode, &app.input_buf,
                search_pat,
                app.search_matches.len(), app.search_cursor, app.search_truncated,
                app.context_mode,
            );

            // Popup overlays (rendered last, on top)
            if let Some(ref popup) = app.target_popup {
                crate::ui::target_popup::render(frame, area, popup);
            }
            if let Some(ref popup) = app.time_popup {
                crate::ui::time_popup::render(frame, area, popup);
            }
            if let Some(ref popup) = app.column_popup {
                crate::ui::column_popup::render(frame, area, popup);
            }
            if let Some(ref popup) = app.json_popup {
                crate::ui::json_popup::render(frame, area, popup);
            }
            if let Some(ref popup) = app.stats_popup {
                crate::ui::stats_popup::render(frame, area, popup);
            }
            if app.help_open {
                let popup = app.help_popup.clone().unwrap_or_else(HelpPopup::new);
                crate::ui::help_popup::render(frame, area, &popup);
            }
        })?;

        if event::poll(tick)? {
            match event::read()? {
                Event::Key(key) if key.kind == crossterm::event::KeyEventKind::Press
                    && handle_key(app, key) => { return Ok(()); }
                Event::Mouse(mouse) => handle_mouse(app, mouse),
                _ => {}
            }
        }
    }
}

fn handle_key(app: &mut AppState, key: KeyEvent) -> bool {
    // Help popup takes top priority — Esc/h/q close it; other keys ignored
    if app.help_open {
        match key.code {
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Char('q') => {
                app.help_open = false;
                app.help_popup = None;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return true;
            }
            _ => {}
        }
        return false;
    }

    if app.time_popup.is_some() {
        handle_time_popup_key(app, key);
        return false;
    }
    if app.column_popup.is_some() {
        handle_column_popup_key(app, key);
        return false;
    }

    // JSON detail popup — handled before other popups so its keys take priority
    if app.json_popup.is_some() {
        match key.code {
            KeyCode::Esc => { app.json_popup = None; }
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(ref mut p) = app.json_popup { p.scroll_down(); }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(ref mut p) = app.json_popup { p.scroll_up(); }
            }
            _ => {}
        }
        return false;
    }

    // Stats popup — Esc closes
    if app.stats_popup.is_some() {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('s') | KeyCode::Char('q')) {
            app.stats_popup = None;
        }
        return false;
    }

    // Target popup takes priority — never propagates quit signal
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
                    app.search_history_idx = None;
                    if !pattern.is_empty() {
                        // Push to front, dedup (remove existing copies first), cap at 50.
                        app.search_history.retain(|p| p != &pattern);
                        app.search_history.push_front(pattern.clone());
                        while app.search_history.len() > 50 { app.search_history.pop_back(); }
                        app.run_search(pattern, forward);
                    }
                }
                KeyCode::Esc => {
                    app.input_buf.clear();
                    app.input_mode = InputMode::Normal;
                    app.search_history_idx = None;
                }
                KeyCode::Up => {
                    if !app.search_history.is_empty() {
                        let new_idx = match app.search_history_idx {
                            None => 0,
                            Some(i) => (i + 1).min(app.search_history.len() - 1),
                        };
                        app.search_history_idx = Some(new_idx);
                        app.input_buf = app.search_history[new_idx].clone();
                    }
                }
                KeyCode::Down => {
                    match app.search_history_idx {
                        None => {}
                        Some(0) => {
                            app.search_history_idx = None;
                            app.input_buf.clear();
                        }
                        Some(i) => {
                            app.search_history_idx = Some(i - 1);
                            app.input_buf = app.search_history[i - 1].clone();
                        }
                    }
                }
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
        InputMode::ExportPath => {
            match key.code {
                KeyCode::Enter => {
                    let path = app.input_buf.drain(..).collect::<String>();
                    app.input_mode = InputMode::Normal;
                    export_filtered_view(app, &path);
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

        KeyCode::Char('c') => {
            if app.search_query.is_some() {
                app.context_mode = !app.context_mode;
            }
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            if app.search_query.is_some() {
                app.context_size = (app.context_size + 1).min(20);
            }
        }
        KeyCode::Char('-') => {
            if app.search_query.is_some() {
                app.context_size = app.context_size.saturating_sub(1).max(1);
            }
        }

        KeyCode::Char('j') | KeyCode::Down  => app.scroll_down(1),
        KeyCode::Char('k') | KeyCode::Up    => app.scroll_up(1),
        KeyCode::Char('d') if ctrl => { let h = app.half_page(); app.scroll_down(h); }
        KeyCode::Char('u') if ctrl => { let h = app.half_page(); app.scroll_up(h); }
        KeyCode::Char('f') if ctrl => { let p = app.full_page(); app.scroll_down(p); }
        KeyCode::Char('b') if ctrl => { let p = app.full_page(); app.scroll_up(p); }
        KeyCode::PageDown => { let p = app.full_page(); app.scroll_down(p); }
        KeyCode::PageUp   => { let p = app.full_page(); app.scroll_up(p); }

        KeyCode::Left  => { app.h_scroll = app.h_scroll.saturating_sub(8); }
        KeyCode::Right => { app.h_scroll = (app.h_scroll + 8).min(500); }

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

        KeyCode::Char('D') => {
            app.dedup_enabled = !app.dedup_enabled;
        }

        KeyCode::Char('y') => {
            let phys = app.logical_to_physical(app.viewport_top);
            if let Some(bytes) = app.buffer.read_line(phys) {
                if let Ok(text) = String::from_utf8(bytes) {
                    if let Ok(mut ctx) = arboard::Clipboard::new() {
                        let _ = ctx.set_text(text);
                    }
                }
            }
        }

        KeyCode::Char('Y') => {
            let total = app.visible_line_count();
            let limit = total.min(10_000);
            let mut out = String::new();
            for logical in 0..limit {
                let phys = app.logical_to_physical(logical);
                if let Some(bytes) = app.buffer.read_line(phys) {
                    if let Ok(s) = std::str::from_utf8(&bytes) {
                        out.push_str(s);
                        out.push('\n');
                    }
                }
            }
            if let Ok(mut ctx) = arboard::Clipboard::new() {
                let _ = ctx.set_text(out);
            }
        }

        KeyCode::Char('F') => {
            if app.format == FormatHint::Json {
                if app.json_fields.is_empty() {
                    app.json_fields = discover_json_fields(&app);
                }
                app.column_popup = Some(ColumnPopup::new(&app.json_fields, &app.json_columns));
            }
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

        KeyCode::Char('T') => {
            app.time_popup = Some(TimePopup::new(
                app.filter_state.time_from.clone(),
                app.filter_state.time_to.clone(),
            ));
        }

        KeyCode::Char(':') => {
            app.input_mode = InputMode::CommandLine;
            app.input_buf.clear();
        }

        KeyCode::Char('m') => app.toggle_bookmark(),
        KeyCode::Char('[') => app.bookmark_prev(),
        KeyCode::Char(']') => app.bookmark_next(),

        KeyCode::Char('h') => {
            app.help_open = !app.help_open;
            app.help_popup = if app.help_open { Some(HelpPopup::new()) } else { None };
        }
        KeyCode::Char('l') => {
            app.show_line_numbers = !app.show_line_numbers;
        }
        KeyCode::Char('w') => {
            app.word_wrap = !app.word_wrap;
        }

        KeyCode::Char('p') => {
            if app.format == FormatHint::Json {
                let phys = app.logical_to_physical(app.viewport_top);
                if let Some(bytes) = app.buffer.read_line(phys) {
                    app.json_popup = crate::ui::json_popup::JsonPopup::new(&bytes);
                }
            }
        }

        KeyCode::Char('s') => {
            let lines = compute_stats_lines(app);
            app.stats_popup = Some(crate::ui::stats_popup::StatsPopup { lines });
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

fn discover_json_fields(app: &AppState) -> Vec<String> {
    use std::collections::BTreeSet;
    const EXCLUDE: &[&str] = &[
        "level", "severity", "lvl", "msg", "message", "body",
        "timestamp", "time", "ts", "target", "module",
        "loglevel", "@timestamp", "datetime", "logger", "fields",
    ];
    let mut set = BTreeSet::new();
    let limit = app.buffer.line_count().min(500);
    for ln in 0..limit {
        let Some(bytes) = app.buffer.read_line(ln) else { continue };
        if bytes.first() != Some(&b'{') { continue }
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else { continue };
        if let Some(obj) = v.as_object() {
            for key in obj.keys() {
                if !EXCLUDE.contains(&key.as_str()) {
                    set.insert(key.clone());
                }
            }
        }
    }
    set.into_iter().collect()
}

fn handle_column_popup_key(app: &mut AppState, key: crossterm::event::KeyEvent) -> bool {
    let Some(ref mut popup) = app.column_popup else { return false };
    let visible_rows = 20usize;
    match key.code {
        KeyCode::Esc => { app.column_popup = None; }
        KeyCode::Enter => {
            let cols = popup.applied();
            app.json_columns = cols;
            app.column_popup = None;
        }
        KeyCode::Char(' ') => { popup.toggle_cursor(); }
        KeyCode::Up   | KeyCode::Char('k') => { popup.move_up(); }
        KeyCode::Down | KeyCode::Char('j') => { popup.move_down(visible_rows); }
        _ => {}
    }
    true
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
        KeyCode::Up   | KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => { popup.move_up(); }
        KeyCode::Down | KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => { popup.move_down(visible_rows); }
        KeyCode::Up   => { popup.move_up(); }
        KeyCode::Down => { popup.move_down(visible_rows); }
        KeyCode::Left  | KeyCode::Char('[') => { popup.depth_dec(); }
        KeyCode::Right | KeyCode::Char(']') => { popup.depth_inc(); }
        KeyCode::Backspace => { popup.pop_filter_char(); }
        // Printable chars feed the fuzzy filter (allow `:` and `_` for module paths).
        KeyCode::Char(c) if c.is_alphanumeric() || c == ':' || c == '_' || c == '-' || c == '.' => {
            popup.push_filter_char(c);
        }
        _ => {}
    }
    true
}

/// Scan last 100 buffer lines to find a recent parseable timestamp for relative offsets.
/// Falls back to system time if no timestamp is found.
fn get_now_key(app: &AppState) -> String {
    let total = app.buffer.line_count();
    let start = total.saturating_sub(100);
    for ln in (start..total).rev() {
        if let Some(bytes) = app.buffer.read_line(ln) {
            let log_line = crate::parser::parse_line(&bytes, ln, app.format);
            if let Some(ts) = log_line.timestamp {
                if let Some(key) = crate::time_parse::parse_ts_key(&ts) {
                    return key;
                }
            }
        }
    }
    crate::time_parse::now_key()
}

fn handle_time_popup_key(app: &mut AppState, key: crossterm::event::KeyEvent) {
    use crossterm::event::KeyModifiers;

    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    match key.code {
        KeyCode::Esc => {
            app.time_popup = None;
        }
        KeyCode::Enter => {
            let (from_input, to_input) = {
                let popup = app.time_popup.as_ref().unwrap();
                (popup.from_input.clone(), popup.to_input.clone())
            };
            app.time_popup = None;

            let now_key_str = get_now_key(app);
            app.filter_state.time_from =
                crate::time_parse::parse_user_input(&from_input, &now_key_str);
            app.filter_state.time_to =
                crate::time_parse::parse_user_input(&to_input, &now_key_str);

            app.viewport_top = 0;
            if !app.filter_state.is_active() {
                app.filter_view.clear();
            } else {
                app.trigger_filter_recompute();
            }
        }
        KeyCode::Tab => {
            if let Some(ref mut popup) = app.time_popup {
                popup.toggle_focus();
            }
        }
        KeyCode::BackTab => {
            if let Some(ref mut popup) = app.time_popup {
                popup.toggle_focus();
            }
        }
        KeyCode::Backspace => {
            if let Some(ref mut popup) = app.time_popup {
                popup.pop_char();
            }
        }
        KeyCode::Char(c) if !shift || c.is_ascii_punctuation() || c.is_alphabetic() || c.is_ascii_digit() => {
            if let Some(ref mut popup) = app.time_popup {
                popup.push_char(c);
            }
        }
        _ => {}
    }
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

/// Build the human-readable stats lines for the stats popup.
/// Scans up to 100k lines for performance on large logs.
fn compute_stats_lines(app: &AppState) -> Vec<String> {
    use std::collections::HashMap;
    let total = app.buffer.line_count();
    let scan_limit = total.min(100_000);

    let mut level_counts: HashMap<crate::parser::LogLevel, u64> = HashMap::new();
    let mut target_counts: HashMap<String, u64> = HashMap::new();

    for ln in 0..scan_limit {
        let Some(bytes) = app.buffer.read_line(ln) else { continue };
        if bytes.is_empty() { continue }
        let log_line = parse_line(&bytes, ln, app.format);
        if let Some(level) = log_line.level {
            *level_counts.entry(level).or_insert(0) += 1;
        }
        if let Some(target) = log_line.target {
            *target_counts.entry(target).or_insert(0) += 1;
        }
    }

    let filtered_count = if app.filter_view.is_empty() {
        total
    } else {
        app.filter_view.len() as u64
    };

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("Total lines:       {}", total));
    lines.push(format!("Visible lines:     {}", filtered_count));
    lines.push(format!(
        "Scanned for stats: {} ({}%)",
        scan_limit,
        if total > 0 { scan_limit * 100 / total } else { 0 }
    ));
    lines.push(String::new());
    lines.push("Levels:".to_string());

    for level in app.registry.levels.iter() {
        let count = level_counts.get(level).copied().unwrap_or(0);
        let key_label = app.registry.key_for(level)
            .map(|k| format!("[{}]", k))
            .unwrap_or_else(|| "   ".to_string());
        lines.push(format!("  {} {:6}  {}", key_label, level.as_str(), count));
    }

    lines.push(String::new());
    lines.push("Top 5 targets:".to_string());

    let mut tlist: Vec<(String, u64)> = target_counts.into_iter().collect();
    tlist.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    if tlist.is_empty() {
        lines.push("  (no targets discovered)".to_string());
    } else {
        for (name, count) in tlist.into_iter().take(5) {
            let display = if name.len() > 40 {
                format!("…{}", &name[name.len() - 39..])
            } else {
                name
            };
            lines.push(format!("  {:6}  {}", count, display));
        }
    }

    lines
}

/// Stub: pending impl-io agent's full implementation.
fn export_filtered_view(_app: &AppState, _path: &str) {
    // No-op stub so the build compiles; impl-io will add the real exporter.
}
