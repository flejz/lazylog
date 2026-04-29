pub mod applog;
pub mod json;
pub mod syslog;
pub mod text;
pub mod web;

use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
    Custom(u8), // index 5..=253 for dynamically discovered levels
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Error => "ERROR",
            LogLevel::Warn  => "WARN",
            LogLevel::Info  => "INFO",
            LogLevel::Debug => "DEBUG",
            LogLevel::Trace => "TRACE",
            LogLevel::Custom(_) => "CUSTOM",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().trim() {
            "ERROR" | "ERR" | "FATAL" | "CRITICAL" | "CRIT" => Some(LogLevel::Error),
            "WARN"  | "WARNING"                              => Some(LogLevel::Warn),
            "INFO"  | "INFORMATION" | "NOTICE"               => Some(LogLevel::Info),
            "DEBUG" | "DBG"                                  => Some(LogLevel::Debug),
            "TRACE" | "VERBOSE" | "TRC"                      => Some(LogLevel::Trace),
            _ => None,
        }
    }

    /// Map syslog/journald/dmesg severity (RFC 5424, 0=emerg … 7=debug).
    pub fn from_syslog(n: u8) -> Self {
        match n {
            0 | 1 | 2 => LogLevel::Error,  // emerg, alert, crit
            3          => LogLevel::Error,  // err
            4          => LogLevel::Warn,
            5          => LogLevel::Info,   // notice
            6          => LogLevel::Info,
            _          => LogLevel::Debug,  // 7 = debug
        }
    }

    /// Map OpenTelemetry SeverityNumber (1–24) to LogLevel.
    pub fn from_otel(n: u8) -> Self {
        match n {
            1..=4   => LogLevel::Trace,  // TRACE1..TRACE4
            5..=8   => LogLevel::Debug,  // DEBUG1..DEBUG4
            9..=12  => LogLevel::Info,   // INFO1..INFO4
            13..=16 => LogLevel::Warn,   // WARN1..WARN4
            17..=20 => LogLevel::Error,  // ERROR1..ERROR4
            _       => LogLevel::Error,  // FATAL1..FATAL4 (21–24)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatHint {
    Json,
    Syslog,
    Web,
    AppLog,
    Text,
}

/// A parsed log line. All string fields borrow from the original line bytes when possible.
#[derive(Debug, Clone)]
pub struct LogLine {
    /// Original raw bytes for the line (used for display fallback and search)
    pub raw: Vec<u8>,
    pub level: Option<LogLevel>,
    /// Crate/module target (e.g. "myapp::db")
    pub target: Option<String>,
    pub timestamp: Option<String>,
    pub message: Option<String>,
    /// Line number in the source file (0-based)
    pub line_no: u64,
    /// Index of source file when multiple files are merged (0 for single-file mode)
    pub file_idx: usize,
}

impl LogLine {
    pub fn display_message(&self) -> Cow<'_, str> {
        if let Some(ref m) = self.message {
            Cow::Borrowed(m.as_str())
        } else {
            String::from_utf8_lossy(&self.raw)
        }
    }
}

/// Detect format by sampling the first 4096 bytes.
pub fn detect_format(sample: &[u8]) -> FormatHint {
    let s = std::str::from_utf8(sample).unwrap_or("");
    for line in s.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue; // IIS/W3C headers, blank lines
        }
        // JSON: NDJSON or CloudTrail per-record
        if t.starts_with('{') {
            return FormatHint::Json;
        }
        // Syslog RFC 3164/5424: <PRI>
        if t.starts_with('<') && t[1..].find('>').is_some() {
            return FormatHint::Syslog;
        }
        // dmesg: [seconds.micros]
        if t.starts_with('[') {
            if let Some(close) = t[1..].find(']') {
                let inner = &t[1..close + 1];
                if inner.bytes().all(|b| b.is_ascii_digit() || b == b'.' || b == b' ') {
                    return FormatHint::Syslog;
                }
            }
        }
        // Nginx error log: YYYY/MM/DD HH:MM:SS [level]
        if t.len() > 20 && t.as_bytes()[4] == b'/' && t.as_bytes()[7] == b'/' {
            return FormatHint::Web;
        }
        // Apache/Nginx access (CLF): IP ... [DD/Mon/YYYY:
        if t.contains(" [") && t.contains('/') && t.contains(':') && t.contains('"') {
            // CLF has quoted request string
            return FormatHint::Web;
        }
        // IIS W3C data line: YYYY-MM-DD HH:MM:SS ...
        if t.len() > 19
            && t.as_bytes()[4] == b'-'
            && t.as_bytes()[7] == b'-'
            && t.as_bytes()[10] == b' '
            && t.as_bytes()[13] == b':'
        {
            // Could be IIS or log4j/python — check for space-separated fields (IIS)
            // vs Log4j `[thread]` / Python `,ms`
            let rest = &t[11..];
            if !rest.contains('[') && !rest.contains(',') {
                return FormatHint::Web;
            }
        }
        // Journald short: Mmm dd HH:MM:SS host unit[pid]: msg
        if t.len() > 15
            && t.as_bytes()[3] == b' '
            && t.as_bytes()[6..8] == *b": " // could be time sep
        {
            // rough check: third char is space, starts with 3 alpha chars
            if t[..3].chars().all(|c| c.is_ascii_alphabetic()) {
                return FormatHint::Syslog;
            }
        }
        // Log4j: YYYY-MM-DD HH:MM:SS.mmm [thread]
        // Python: YYYY-MM-DD HH:MM:SS,mmm LEVEL
        if t.len() > 19
            && t.as_bytes()[4] == b'-'
            && t.as_bytes()[7] == b'-'
            && (t.contains(" [") || t.as_bytes()[19] == b',')
        {
            return FormatHint::AppLog;
        }
        return FormatHint::Text;
    }
    FormatHint::Text
}

/// Parse a single line given a format hint. Falls back to a raw LogLine on failure.
pub fn parse_line(raw: &[u8], line_no: u64, hint: FormatHint) -> LogLine {
    match hint {
        FormatHint::Json => {
            if let Some(l) = json::parse_json_line(raw, line_no) {
                return l;
            }
            text::parse_text_line(raw, line_no)
        }
        FormatHint::Syslog => {
            if let Some(l) = syslog::parse_syslog_line(raw, line_no) {
                return l;
            }
            text::parse_text_line(raw, line_no)
        }
        FormatHint::Web => {
            if let Some(l) = web::parse_web_line(raw, line_no) {
                return l;
            }
            text::parse_text_line(raw, line_no)
        }
        FormatHint::AppLog => {
            if let Some(l) = applog::parse_applog_line(raw, line_no) {
                return l;
            }
            text::parse_text_line(raw, line_no)
        }
        FormatHint::Text => text::parse_text_line(raw, line_no),
    }
}
