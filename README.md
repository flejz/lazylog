# lazylog

Tiny portable TUI log viewer for Rust apps. Single binary, ~1.5MB.

```
lazylog app.log
lazylog --follow app.log
cat app.log | lazylog
```

## Features

- **Rolling buffer** — never OOM on multi-GB files
  - mmap (≤256MB), chunked 8MB reads (>256MB/tail), ring buffer (stdin)
  - Sparse line index at stride 64: ~3MB for a 5GB file
- **Format auto-detection** — JSON (`tracing-subscriber`) and plain text (`env_logger`, tracing default)
  - Span context stripped: `span{k=v}: crate::module: message` → target = `crate::module`
- **Search** — `/pattern`, `n`/`N`, `F3`/`Shift-F3`
  - Active match: bright amber · other matches: dim · `Esc` clears
  - Counter: `3/47` or `3/many` when >9999 matches
- **Level filter** — keys `1`–`9` toggle dynamically discovered levels
- **Target popup** — `t` opens a checkbox list of all discovered crate targets
  - Depth control (`←`/`→`) groups by `::` segments
  - Multi-select with OR logic
- **Tail/follow mode** — `f` toggles live scroll as file grows (150ms poll)
- **Stdin** — `cat app.log | lazylog` (ring buffer, 50k lines)
- **Mouse** — scroll and click to select lines

## Keybindings

| Key | Action |
|-----|--------|
| `j`/`k`, `↑`/`↓` | scroll line |
| `Ctrl-D`/`Ctrl-U` | half page |
| `Ctrl-F`/`PgDn`, `Ctrl-B`/`PgUp` | full page |
| `gg` / `G` | top / bottom |
| `/` / `?` | search forward / backward |
| `n` / `N`, `F3` / `Shift-F3` | next / prev match |
| `Esc` | clear search |
| `f` | toggle follow mode |
| `1`–`9` | toggle level filter |
| `t` | open target filter popup |
| `:N` | go to line N |
| `q` / `Ctrl-C` | quit |

## Install

```sh
cargo install --path .
```

### Register `.log` file association

```sh
lazylog register
```

- **Windows**: writes `HKCR\.log` registry keys (run as Administrator for system-wide)
- **Linux**: creates `~/.local/share/applications/lazylog.desktop` + `xdg-mime`

## Build

```sh
cargo build --release   # → target/release/lazylog[.exe]
```

Release binary: `lto=fat`, `strip=true`, `opt-level=z`.

## Log formats supported

| Format | Example |
|--------|---------|
| tracing-subscriber (text) | `2024-01-01T10:00:00Z  INFO crate::module: message` |
| tracing-subscriber with spans | `2024-01-01T10:00:00Z  INFO span{k=v}: crate::module: message` |
| env_logger | `[2024-01-01T10:00:00Z INFO crate::module] message` |
| tracing-subscriber (JSON) | `{"level":"INFO","target":"crate","msg":"message"}` |
| OpenTelemetry / serde_json | field aliases for level/target/message/timestamp |
