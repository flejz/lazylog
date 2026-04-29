use regex::Regex;
use std::sync::OnceLock;
use super::{LogLevel, LogLine};

// Log4j / Logback: `YYYY-MM-DD HH:MM:SS.mmm [thread] LEVEL logger - message`
// Tolerates an optional extra bracket token (e.g. `[File.java:142]`) between
// logger and ` - `, which appears in some log4j layouts.
static LOG4J: OnceLock<Regex> = OnceLock::new();
// Python `logging`: `YYYY-MM-DD HH:MM:SS,mmm LEVEL name: message`
static PYLOG: OnceLock<Regex> = OnceLock::new();

fn log4j() -> &'static Regex {
    LOG4J.get_or_init(|| {
        Regex::new(
            r"^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) \[([^\]]+)\] (ERROR|WARN(?:ING)?|INFO|DEBUG|TRACE|FATAL) +([\w.\-]+)(?:\s+\[[^\]]+\])* - (.+)$"
        ).unwrap()
    })
}

fn pylog() -> &'static Regex {
    PYLOG.get_or_init(|| {
        Regex::new(
            r"^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3}) (ERROR|WARN(?:ING)?|INFO|DEBUG|TRACE|CRITICAL|FATAL) ([\w.\-]+): (.+)$"
        ).unwrap()
    })
}

pub fn parse_applog_line(raw: &[u8], line_no: u64) -> Option<LogLine> {
    let s = std::str::from_utf8(raw).ok()?;
    let trimmed = s.trim_end_matches(['\r', '\n']);

    if let Some(caps) = log4j().captures(trimmed) {
        let timestamp = caps.get(1).map(|m| m.as_str().to_owned());
        let level = caps.get(3).and_then(|m| LogLevel::from_str(m.as_str()));
        let target = caps.get(4).map(|m| m.as_str().to_owned());
        let message = caps.get(5).map(|m| m.as_str().to_owned());
        return Some(LogLine {
            raw: raw.to_vec(),
            level,
            target,
            timestamp,
            message,
            line_no,
            file_idx: 0,
        });
    }

    if let Some(caps) = pylog().captures(trimmed) {
        let timestamp = caps.get(1).map(|m| m.as_str().to_owned());
        let level = caps.get(2).and_then(|m| LogLevel::from_str(m.as_str()));
        let target = caps.get(3).map(|m| m.as_str().to_owned());
        let message = caps.get(4).map(|m| m.as_str().to_owned());
        return Some(LogLine {
            raw: raw.to_vec(),
            level,
            target,
            timestamp,
            message,
            line_no,
            file_idx: 0,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log4j_info_double_space() {
        let line = b"2025-04-29 08:14:32.001 [main] INFO  com.example.Application - Application starting up";
        let l = parse_applog_line(line, 0).expect("should parse");
        assert_eq!(l.level, Some(LogLevel::Info));
        assert_eq!(l.timestamp.as_deref(), Some("2025-04-29 08:14:32.001"));
        assert_eq!(l.target.as_deref(), Some("com.example.Application"));
        assert_eq!(l.message.as_deref(), Some("Application starting up"));
    }

    #[test]
    fn log4j_debug_with_pool_message() {
        let line = b"2025-04-29 08:14:32.789 [main] DEBUG com.example.db.DataSource - Connection pool initialized with 10 connections";
        let l = parse_applog_line(line, 0).expect("should parse");
        assert_eq!(l.level, Some(LogLevel::Debug));
        assert_eq!(l.target.as_deref(), Some("com.example.db.DataSource"));
        assert_eq!(l.message.as_deref(), Some("Connection pool initialized with 10 connections"));
    }

    #[test]
    fn log4j_error_with_source_location_bracket() {
        let line = b"2025-04-29 08:14:45.123 [http-nio-8080-exec-3] ERROR com.example.service.UserService [UserService.java:142] - Failed to fetch user id=9001: database timeout after 10000ms";
        let l = parse_applog_line(line, 0).expect("should parse");
        assert_eq!(l.level, Some(LogLevel::Error));
        assert_eq!(l.target.as_deref(), Some("com.example.service.UserService"));
        assert_eq!(
            l.message.as_deref(),
            Some("Failed to fetch user id=9001: database timeout after 10000ms")
        );
    }

    #[test]
    fn log4j_thread_with_dashes() {
        let line = b"2025-04-29 08:14:33.001 [http-nio-8080-exec-1] INFO  com.example.web.RequestFilter - Incoming request: GET /api/health from 10.0.0.1";
        let l = parse_applog_line(line, 0).expect("should parse");
        assert_eq!(l.level, Some(LogLevel::Info));
        assert_eq!(l.target.as_deref(), Some("com.example.web.RequestFilter"));
        assert!(l.message.as_deref().unwrap().starts_with("Incoming request:"));
    }

    #[test]
    fn pylog_info_root() {
        let line = b"2025-04-29 08:14:30,001 INFO root: Application starting";
        let l = parse_applog_line(line, 0).expect("should parse");
        assert_eq!(l.level, Some(LogLevel::Info));
        assert_eq!(l.timestamp.as_deref(), Some("2025-04-29 08:14:30,001"));
        assert_eq!(l.target.as_deref(), Some("root"));
        assert_eq!(l.message.as_deref(), Some("Application starting"));
    }

    #[test]
    fn pylog_warning_dotted_name() {
        let line = b"2025-04-29 08:14:31,001 WARNING myapp.db: Connection pool 80% utilized (8/10 connections in use)";
        let l = parse_applog_line(line, 0).expect("should parse");
        assert_eq!(l.level, Some(LogLevel::Warn));
        assert_eq!(l.target.as_deref(), Some("myapp.db"));
        assert!(l.message.as_deref().unwrap().contains("80%"));
    }

    #[test]
    fn pylog_critical_maps_to_error() {
        let line = b"2025-04-29 08:14:36,790 CRITICAL myapp.payments: Payment gateway unreachable after 3 retries host=pay.example.com";
        let l = parse_applog_line(line, 0).expect("should parse");
        assert_eq!(l.level, Some(LogLevel::Error));
        assert_eq!(l.target.as_deref(), Some("myapp.payments"));
    }

    #[test]
    fn pylog_error_with_equals_in_message() {
        let line = b"2025-04-29 08:14:36,789 ERROR myapp.api.orders: Failed to process order order_id=9001 error=PaymentGatewayTimeout";
        let l = parse_applog_line(line, 0).expect("should parse");
        assert_eq!(l.level, Some(LogLevel::Error));
        assert_eq!(l.target.as_deref(), Some("myapp.api.orders"));
        assert!(l.message.as_deref().unwrap().contains("order_id=9001"));
    }

    #[test]
    fn tracing_subscriber_line_does_not_match() {
        // Has `T` separator and `Z` suffix — must NOT be claimed by either applog format
        // so that mod.rs can fall through to text.rs.
        let line = b"2026-04-28T14:49:55.397326Z  INFO clinikit::pkg::singleton: singleton instance assert";
        assert!(parse_applog_line(line, 0).is_none());
    }

    #[test]
    fn empty_line_does_not_match() {
        assert!(parse_applog_line(b"", 0).is_none());
    }

    #[test]
    fn random_text_does_not_match() {
        assert!(parse_applog_line(b"this is not a log line", 0).is_none());
    }

    #[test]
    fn trailing_newline_is_tolerated() {
        let line = b"2025-04-29 08:14:30,001 INFO root: Application starting\n";
        let l = parse_applog_line(line, 0).expect("should parse");
        assert_eq!(l.message.as_deref(), Some("Application starting"));
    }
}
