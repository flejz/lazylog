# Remaining Feature Backlog

Branch `feat/backlog-tier3` — pick up here after Tier 1 + Tier 2 complete.

All Tier 1 + Tier 2 features shipped on `master` as of 2026-04-30.

---

## Tier 3 — High Value / High Effort

### 1. Session Persistence

Save/restore per-file state on reopen.

**What to save:**
- `FilterState` (level mask, crate prefixes, time range)
- `search_query` pattern
- `viewport_top` (scroll position)
- `bookmarks` (BTreeSet<u64>)
- `show_line_numbers`, `word_wrap`, `h_scroll`

**Storage:** `~/.cache/lazylog/sessions/<hash_of_file_path>.json`

**When to save:** on quit (`q`/`Q`/Ctrl+C), before `restore_terminal()` in `run()`.

**When to load:** in `run()` after AppState is initialized, before first draw.

**New deps:** `serde` already present, `dirs` already present.

**Files to touch:**
- `src/app.rs` — save on quit, load on init
- New `src/session.rs` — `SessionState` struct, `save()`, `load()`

**Key consideration:** Hash the canonical file path, not just the name. Use `std::fs::canonicalize()`.

---

### 2. Filter Expression DSL

Inspired by lnav's `:filter-in` / `:filter-out`.

**Syntax:** `level:error`, `target:api`, `msg:timeout`, `AND`/`OR`/`NOT`, e.g.:
```
level:error AND target:api::handler
```

**UX:** New input mode `InputMode::FilterExpr` triggered by `;` key. Bottom bar shows `filter> ` prompt. Enter applies, Esc cancels.

**Parser:** Hand-written recursive descent or use `nom` crate. Produces a `FilterExpr` enum:
```rust
enum FilterExpr {
    Level(LogLevel),
    Target(String),      // substring match
    Msg(String),         // substring match
    And(Box<FilterExpr>, Box<FilterExpr>),
    Or(Box<FilterExpr>, Box<FilterExpr>),
    Not(Box<FilterExpr>),
}
```

**Integration:** Add `expr: Option<FilterExpr>` to `FilterState`. `FilterWorker` evaluates per line.

**Files to touch:**
- New `src/filter/expr.rs` — `FilterExpr` enum + parser + evaluator
- `src/filter/mod.rs` — add `expr` field to `FilterState`
- `src/filter/view.rs` — apply expr in `FilterWorker`
- `src/ui/searchbar.rs` — `InputMode::FilterExpr` + render
- `src/app.rs` — `;` key handler, `FilterExpr` input mode handling

---

### 3. Command Palette

Inspired by helix `Space`, VSCode `Ctrl+P`.

**UX:** `Ctrl+P` opens floating fuzzy-search popup listing all commands. Type to filter, Enter to execute, Esc to cancel.

**Command list** (static `&[(&str, fn(&mut AppState))]`):
- "Toggle follow mode", "Toggle dedup", "Toggle line numbers", "Toggle word wrap"
- "Clear search", "Clear filter", "Export filtered view", "Save preset", "Load preset"
- "Stats popup", "Help overlay", "JSON detail", "Time filter", "Target filter"
- "Go to top", "Go to bottom"

**Fuzzy match:** simple substring or initials match (no dep needed).

**Files to touch:**
- New `src/ui/command_palette.rs` — popup widget, fuzzy filter logic
- `src/ui/mod.rs` — add `pub mod command_palette;`
- `src/app.rs` — `command_palette_open: bool`, `command_palette_input: String`, `command_palette_cursor: usize`, `Ctrl+P` key, render call, key handler

---

### 4. Multi-File Timestamp Merge

Currently `MultiBuffer` merges files by interleaving in file-order blocks, not by timestamp.

**What's needed:** Re-sort all lines across files by parsed timestamp during `MultiBuffer::open()`.

**Challenge:** Requires parsing timestamps for ALL lines at open time — O(n) parse pass.

**Approach:**
1. Read all files, parse timestamp for each line
2. Build a sorted `Vec<(timestamp_key: String, file_idx: usize, phys_line: u64)>`
3. Store this as the logical→physical mapping in MultiBuffer
4. Lines without timestamps: insert by file order relative to surrounding timestamps

**Files to touch:**
- `src/buffer/multi.rs` — rewrite `open()` to do timestamp-sorted merge
- `src/parser/mod.rs` — expose `parse_timestamp_only()` for fast ts extraction without full parse

**Note:** Only feasible for files where timestamps are parseable. Falls back to current behavior if <50% lines have timestamps.

---

## Nice-to-Have (Low Priority)

### 5. Split Detail Pane

Persistent bottom panel showing parsed fields of the cursor line.

**UX:** `Shift+Enter` or `|` toggles a 8-row panel at the bottom showing:
- All parsed fields (timestamp, level, target, message)
- For JSON: all key-value pairs, pretty-printed across multiple rows

**Layout change:** 3-chunk → 4-chunk vertical layout:
```
[statusbar 1]
[viewport flex]
[detail pane 8]  ← new, toggleable
[searchbar 1]
```

**Files to touch:**
- `src/app.rs` — `detail_pane_open: bool`, `|` key, pass selected line to detail render
- New `src/ui/detail_pane.rs`

---

### 6. Multi-Select / Bulk Copy-Export

Select a range of lines, copy or export just those.

**UX:** `V` enters visual-line mode (vim-style). `j`/`k` extend selection. `y` copies selection, `e` exports selection.

**State:** `visual_start: Option<u64>`, `visual_end: Option<u64>` in AppState.

**Files to touch:** `src/app.rs` (visual mode state + keys), `src/ui/viewport.rs` (highlight selected range)

---

### 7. Macro Recording

`q{a}` starts recording into register `a`. Keys recorded into `Vec<KeyEvent>`. `q` stops. `@{a}` replays.

**State:** `macro_recording: Option<char>`, `macros: HashMap<char, Vec<KeyEvent>>`, `macro_active: bool`.

**Complexity:** High — need to intercept all key events during recording, replay them through `handle_key`. Risk of infinite loops (`@a` inside `a`). Add recursion guard.

**Files to touch:** `src/app.rs` only — purely key-event level feature.
