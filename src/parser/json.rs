use serde_json::Value;
use super::{LogLevel, LogLine};

pub fn parse_json_line(raw: &[u8], line_no: u64) -> Option<LogLine> {
    let s = std::str::from_utf8(raw).ok()?;
    let v: Value = serde_json::from_str(s.trim()).ok()?;
    let obj = v.as_object()?;

    let level = extract_level(obj);
    let target = extract_str_field(obj, &["target", "module", "logger"])
        .or_else(|| extract_nested(obj, "fields", "target"))
        .map(|s| s.to_owned());
    let timestamp = extract_str_field(obj, &["timestamp", "time", "ts", "@timestamp", "datetime"])
        .map(|s| s.to_owned());
    let message = extract_str_field(obj, &["msg", "message", "body"])
        .or_else(|| extract_nested(obj, "fields", "message"))
        .map(|s| s.to_owned());

    Some(LogLine {
        raw: raw.to_vec(),
        level,
        target,
        timestamp,
        message,
        line_no,
    })
}

fn extract_str_field<'a>(obj: &'a serde_json::Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(Value::String(s)) = obj.get(*key) {
            return Some(s.as_str());
        }
    }
    None
}

fn extract_nested<'a>(obj: &'a serde_json::Map<String, Value>, parent: &str, child: &str) -> Option<&'a str> {
    if let Some(Value::Object(inner)) = obj.get(parent) {
        if let Some(Value::String(s)) = inner.get(child) {
            return Some(s.as_str());
        }
    }
    None
}

fn extract_level(obj: &serde_json::Map<String, Value>) -> Option<LogLevel> {
    for key in &["level", "severity", "lvl", "loglevel"] {
        if let Some(Value::String(s)) = obj.get(*key) {
            if let Some(l) = LogLevel::from_str(s) {
                return Some(l);
            }
        }
    }
    None
}
