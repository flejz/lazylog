use regex::Regex;
use std::sync::OnceLock;
use super::{LogLevel, LogLine};

// env_logger / log4rs style: [TIMESTAMP LEVEL target] message
static ENV_LOGGER: OnceLock<Regex> = OnceLock::new();
// tracing-subscriber text style: TIMESTAMP  LEVEL [spans: ]target: message
static TRACING_HEADER: OnceLock<Regex> = OnceLock::new();
// level anywhere fallback
static LEVEL_ANYWHERE: OnceLock<Regex> = OnceLock::new();

fn env_logger() -> &'static Regex {
    ENV_LOGGER.get_or_init(|| {
        Regex::new(
            r"^\[?(\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}[^\]]*)\]?\s+(ERROR|WARN(?:ING)?|INFO(?:RMATION)?|DEBUG|DBG|TRACE|VERBOSE|TRC|FATAL|CRITICAL|CRIT|ERR)\s+([\w][\w:\-]*)\s+(.+)$"
        ).unwrap()
    })
}

fn tracing_header() -> &'static Regex {
    TRACING_HEADER.get_or_init(|| {
        // Matches: TIMESTAMP  LEVEL  <rest>
        // Timestamp includes optional sub-second and Z/offset
        Regex::new(
            r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?)\s{1,4}(ERROR|WARN|INFO|DEBUG|TRACE)\s+(.+)$"
        ).unwrap()
    })
}

fn level_anywhere() -> &'static Regex {
    LEVEL_ANYWHERE.get_or_init(|| {
        Regex::new(r"\b(ERROR|WARN(?:ING)?|INFO(?:RMATION)?|DEBUG|TRACE|FATAL|CRITICAL)\b").unwrap()
    })
}

pub fn parse_text_line(raw: &[u8], line_no: u64) -> LogLine {
    let s = match std::str::from_utf8(raw) {
        Ok(s) => s,
        Err(_) => return raw_line(raw, line_no),
    };
    let trimmed = s.trim();

    // Try tracing-subscriber format first (most common in Rust apps)
    if let Some(caps) = tracing_header().captures(trimmed) {
        let timestamp = caps.get(1).map(|m| m.as_str().to_owned());
        let level = caps.get(2).and_then(|m| LogLevel::from_str(m.as_str()));
        let rest = caps.get(3).map(|m| m.as_str()).unwrap_or("");
        let (target, message) = split_tracing_rest(rest);
        return LogLine { raw: raw.to_vec(), level, target, timestamp, message: Some(message), line_no };
    }

    // Try env_logger / bracket format
    if let Some(caps) = env_logger().captures(trimmed) {
        let timestamp = caps.get(1).map(|m| m.as_str().to_owned());
        let level = caps.get(2).and_then(|m| LogLevel::from_str(m.as_str()));
        let target = caps.get(3).map(|m| m.as_str().to_owned());
        let message = caps.get(4).map(|m| m.as_str().to_owned());
        return LogLine { raw: raw.to_vec(), level, target, timestamp, message, line_no };
    }

    // Fallback: extract level from anywhere
    let level = level_anywhere()
        .find(trimmed)
        .and_then(|m| LogLevel::from_str(m.as_str()));

    LogLine { raw: raw.to_vec(), level, target: None, timestamp: None, message: None, line_no }
}

/// Parse the "rest" portion of a tracing-subscriber line:
///   [span1:span2{fields}: ...]target: message
///
/// Strategy: split on ": " and walk right-to-left to find the last segment
/// that looks like a module path (no braces, spaces, or `=`). That's the target.
/// Everything after is the message.
fn split_tracing_rest(rest: &str) -> (Option<String>, String) {
    // Collect split positions of ": "
    let mut positions: Vec<usize> = Vec::new();
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b':' && bytes[i + 1] == b' ' {
            positions.push(i);
        }
        i += 1;
    }

    // Walk from right to left; find the rightmost ": " where the left side
    // is a valid module-path candidate (no braces, equals, spaces).
    for &pos in positions.iter().rev() {
        let candidate = &rest[..pos];
        // The candidate may be "spans: target" — take only the last `: `-free segment
        let segment = candidate.rsplit(": ").next().unwrap_or(candidate);
        if is_module_path(segment) {
            let target = segment.to_owned();
            let message = rest[pos + 2..].trim_start().to_owned();
            return (Some(target), message);
        }
    }

    // No valid target found — treat everything as message
    (None, rest.to_owned())
}

/// A module path contains only word chars, `::`, `-`, `.`
/// No spaces, braces, `=`, `(`, `)`, `"`, `'`
fn is_module_path(s: &str) -> bool {
    if s.is_empty() { return false; }
    let first = s.chars().next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' { return false; }
    !s.chars().any(|c| matches!(c, '{' | '}' | '=' | ' ' | '\t' | '(' | ')' | '"' | '\''))
}

fn raw_line(raw: &[u8], line_no: u64) -> LogLine {
    LogLine { raw: raw.to_vec(), level: None, target: None, timestamp: None, message: None, line_no }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::LogLevel;

    fn parse(s: &str) -> (Option<LogLevel>, Option<String>, Option<String>) {
        let l = parse_text_line(s.as_bytes(), 0);
        (l.level, l.target, l.message)
    }

    #[test]
    fn tracing_simple() {
        let (lv, tgt, msg) = parse("2026-04-28T14:49:55.397326Z  INFO clinikit::pkg::singleton: singleton instance assert");
        assert_eq!(lv, Some(LogLevel::Info));
        assert_eq!(tgt.as_deref(), Some("clinikit::pkg::singleton"));
        assert_eq!(msg.as_deref(), Some("singleton instance assert"));
    }

    #[test]
    fn tracing_with_spans() {
        let (lv, tgt, msg) = parse("2026-04-28T14:49:57.597666Z  INFO issue_access_token:force_access_token_refresh:resolve{service=CustomerAuth country=None}: clinikit::pkg::url_resolver::service_urls: resolving service url service=CustomerAuth country=None");
        assert_eq!(lv, Some(LogLevel::Info));
        assert_eq!(tgt.as_deref(), Some("clinikit::pkg::url_resolver::service_urls"));
        assert!(msg.as_deref().unwrap().starts_with("resolving service url"));
    }

    #[test]
    fn tracing_error() {
        let (lv, tgt, msg) = parse("2026-04-28T14:49:57.741841Z ERROR clinikit::state_machine::initialization_state_machine: tablet is not found in the system");
        assert_eq!(lv, Some(LogLevel::Error));
        assert_eq!(tgt.as_deref(), Some("clinikit::state_machine::initialization_state_machine"));
        assert_eq!(msg.as_deref(), Some("tablet is not found in the system"));
    }

    #[test]
    fn tracing_span_with_braces() {
        let (lv, tgt, _msg) = parse("2026-04-28T14:49:57.874139Z  INFO update{msg=Disable}: client_gui::components::video::video_pipeline: handling pipeline state change msg=Disable name=\"local-video\"");
        assert_eq!(lv, Some(LogLevel::Info));
        assert_eq!(tgt.as_deref(), Some("client_gui::components::video::video_pipeline"));
    }

    #[test]
    fn tracing_otel_double_space() {
        let (lv, tgt, msg) = parse("2026-04-28T14:49:56.406191Z ERROR opentelemetry_sdk:  name=\"BatchLogProcessor.ExportError\" error=\"Operation failed\"");
        assert_eq!(lv, Some(LogLevel::Error));
        assert_eq!(tgt.as_deref(), Some("opentelemetry_sdk"));
        assert!(msg.as_deref().unwrap().starts_with("name="));
    }
}
