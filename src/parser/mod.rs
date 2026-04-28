pub mod json;
pub mod text;

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
            "INFO"  | "INFORMATION"                          => Some(LogLevel::Info),
            "DEBUG" | "DBG"                                  => Some(LogLevel::Debug),
            "TRACE" | "VERBOSE" | "TRC"                      => Some(LogLevel::Trace),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatHint {
    Json,
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
        if t.is_empty() {
            continue;
        }
        if t.starts_with('{') {
            return FormatHint::Json;
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
            // Fall through to text on JSON parse failure
            text::parse_text_line(raw, line_no)
        }
        FormatHint::Text => text::parse_text_line(raw, line_no),
    }
}
