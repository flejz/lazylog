# lazylog

Tiny portable TUI log viewer. Zero config, single binary (~1.5MB), opens any log format instantly.

```
lazylog app.log
lazylog --follow app.log
lazylog service-a.log service-b.log   # merged chronologically
cat app.log | lazylog
```

## Features

### Buffer engine — never OOM
- **mmap** for files ≤256MB — zero-copy reads
- **Chunked** 8MB streaming for larger files
- **Ring buffer** for stdin (50k lines)
- **Multi-file merge** — N files merged into a single chronological stream, each line color-coded by source
- Sparse line index at stride 64: ~3MB overhead for a 5GB file

### Format auto-detection — no flags needed

Detects format from the first non-empty line. Supports 20+ formats across five families:

#### Syslog / System logs
| Format | Example |
|--------|---------|
| RFC 3164 (BSD syslog) | `<34>Oct 11 22:14:15 host su: 'su root' failed` |
| RFC 5424 (modern syslog) | `<34>1 2025-01-03T14:07:15.003Z host app 123 ID47 - message` |
| systemd journald (short) | `Apr 29 08:14:32 host myapp[1234]: message` |
| dmesg / kernel ring buffer | `[   19.303922] usb 1-3: new high-speed USB device` |

#### Web server logs
| Format | Example |
|--------|---------|
| Apache Common Log Format | `127.0.0.1 - frank [10/Oct/2000:13:55:36 -0700] "GET /index HTTP/1.0" 200 2326` |
| Apache Combined | CLF + `"referer" "user-agent"` |
| Nginx access | `192.168.1.1 - - [03/Mar/2025:15:42:31 +0000] "GET /api HTTP/1.1" 200 1234 ...` |
| Nginx error | `2025/03/03 15:42:31 [error] 1234#0: *67 open() failed ...` |
| IIS W3C Extended | `2025-04-29 08:00:01 192.168.1.10 - 10.0.0.1 80 GET /default.aspx - 200 ...` |

HTTP status → level: `5xx` = ERROR · `4xx` = WARN · `2xx`/`3xx` = INFO

#### Application logs
| Format | Example |
|--------|---------|
| Log4j / Logback | `2025-04-29 08:14:32.001 [main] INFO  com.example.App - Started` |
| Python `logging` | `2025-04-29 08:14:30,001 INFO myapp.db: Connected` |
| tracing-subscriber (text) | `2024-01-01T10:00:00Z  INFO crate::module: message` |
| env_logger / log4rs | `[2024-01-01T10:00:00Z INFO crate::module] message` |

#### Structured JSON (NDJSON)
| Format | Timestamp field | Level field |
|--------|----------------|-------------|
| Elastic Common Schema (ECS) | `@timestamp` | `log.level` |
| GELF (Graylog) | `timestamp` (epoch s) | `level` (syslog 0–7) |
| Logstash | `@timestamp` | `level` |
| AWS CloudTrail | `eventTime` | derived from `errorCode` |
| AWS CloudWatch Logs | `timestamp` (epoch ms) | nested JSON |
| GCP Cloud Logging | `time` | `severity` |
| OpenTelemetry (flat NDJSON) | `timeUnixNano` (epoch ns) | `severityNumber` (1–24) |
| Datadog | `timestamp` / `date` | `status` |
| Splunk HEC | `time` (epoch s) | `event.level` |
| tracing-subscriber (JSON) | `timestamp` | `level` |

Epoch timestamps (seconds / milliseconds / nanoseconds) are converted to ISO 8601 for display.

#### Generic fallback
Any line containing `ERROR` / `WARN` / `INFO` / `DEBUG` / `TRACE` gets level extraction. Nothing is lost.

---

### Search
- `/pattern` forward · `?pattern` backward
- `n`/`N` or `F3`/`Shift-F3` — next/prev match
- Active match: bright amber · other matches: dim
- Counter: `3/47` or `3/many` when >9999 matches
- `Esc` clears

### Level filter
Keys `1`–`9` toggle dynamically discovered levels (ERROR → TRACE or custom).  
Severity mapping is unified across all formats: syslog 0–7, OTel 1–24, GCP enum strings → ERROR/WARN/INFO/DEBUG/TRACE.

### Target/module filter
`t` opens a popup listing all log sources discovered in the file.
- Depth control (`←`/`→`) groups by path separator (`::` `.` `/`)
- Multi-select with OR logic

### Tail / follow mode
`f` toggles live scroll as file grows (150ms poll). Works with any format.

### Stdin
`cat app.log | lazylog` — ring buffer, 50k lines.

### Mouse
Scroll and click to select lines.

---

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
| `t` | open source/module filter popup |
| `:N` | go to line N |
| `q` / `Ctrl-C` | quit |

---

## Install

```sh
cargo install --path .
```

### Register `.log` file association

```sh
lazylog register
```

- **Windows** — writes `HKCR\.log` registry keys (run as Administrator for system-wide)
- **Linux** — creates `~/.local/share/applications/lazylog.desktop` + `xdg-mime`

## Build

```sh
cargo build --release   # → target/release/lazylog[.exe]
```

Release binary: `lto=fat`, `strip=true`, `opt-level=z`.

## Example files

`examples/` contains real-format sample files for every supported format — useful for testing, parser development, and verifying detection behavior:

```
examples/
  syslog_rfc3164.log     apache_common.log      ecs.jsonl
  syslog_rfc5424.log     apache_combined.log    gelf.jsonl
  journald_export.log    nginx_access.log       logstash.jsonl
  dmesg.log              nginx_error.log        cloudtrail.json
  log4j.log              iis_w3c.log            cloudwatch.jsonl
  python_logging.log                            gcp_logging.jsonl
                                                opentelemetry.jsonl
                                                datadog.jsonl
                                                splunk_hec.jsonl
```
