use serde_json::Value;
use super::{LogLevel, LogLine};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonSchema {
    Gelf,
    Otel,
    Splunk,
    CloudTrail,
    CloudWatch,
    Gcp,
    Datadog,
    Ecs,
    Logstash,
    Generic,
}

pub fn detect_json_schema(obj: &serde_json::Map<String, Value>) -> JsonSchema {
    // 1. GELF: version="1.1" AND short_message
    if matches!(obj.get("version"), Some(Value::String(s)) if s == "1.1")
        && obj.contains_key("short_message")
    {
        return JsonSchema::Gelf;
    }
    // 2. OTel flat: timeUnixNano
    if obj.contains_key("timeUnixNano") {
        return JsonSchema::Otel;
    }
    // 3. Splunk HEC: event AND time is number
    if obj.contains_key("event") && matches!(obj.get("time"), Some(Value::Number(_))) {
        return JsonSchema::Splunk;
    }
    // 4. CloudTrail: eventTime AND eventName
    if obj.contains_key("eventTime") && obj.contains_key("eventName") {
        return JsonSchema::CloudTrail;
    }
    // 5. CloudWatch: timestamp number AND message string
    if matches!(obj.get("timestamp"), Some(Value::Number(_)))
        && matches!(obj.get("message"), Some(Value::String(_)))
    {
        return JsonSchema::CloudWatch;
    }
    // 6. GCP: severity AND (time OR receiveTimestamp)
    if obj.contains_key("severity")
        && (obj.contains_key("time") || obj.contains_key("receiveTimestamp"))
    {
        return JsonSchema::Gcp;
    }
    // 7. Datadog: ddsource OR ddtags
    if obj.contains_key("ddsource") || obj.contains_key("ddtags") {
        return JsonSchema::Datadog;
    }
    // 8. ECS: ecs.version OR (@timestamp AND log.level)
    if obj.contains_key("ecs.version")
        || (obj.contains_key("@timestamp") && obj.contains_key("log.level"))
    {
        return JsonSchema::Ecs;
    }
    // 9. Logstash: @version AND @timestamp
    if obj.contains_key("@version") && obj.contains_key("@timestamp") {
        return JsonSchema::Logstash;
    }
    JsonSchema::Generic
}

pub fn parse_json_line(raw: &[u8], line_no: u64) -> Option<LogLine> {
    let s = std::str::from_utf8(raw).ok()?;
    let v: Value = serde_json::from_str(s.trim()).ok()?;
    let obj = v.as_object()?;

    let (level, target, timestamp, message) = match detect_json_schema(obj) {
        JsonSchema::Gelf => parse_gelf(obj),
        JsonSchema::Otel => parse_otel(obj),
        JsonSchema::Splunk => parse_splunk(obj),
        JsonSchema::CloudTrail => parse_cloudtrail(obj),
        JsonSchema::CloudWatch => parse_cloudwatch(obj),
        JsonSchema::Gcp => parse_gcp(obj),
        JsonSchema::Datadog => parse_datadog(obj),
        JsonSchema::Ecs => parse_ecs(obj),
        JsonSchema::Logstash => parse_logstash(obj),
        JsonSchema::Generic => parse_generic(obj),
    };

    Some(LogLine {
        raw: raw.to_vec(),
        level,
        target,
        timestamp,
        message,
        line_no,
        file_idx: 0,
    })
}

type Parsed = (Option<LogLevel>, Option<String>, Option<String>, Option<String>);

fn parse_gelf(obj: &serde_json::Map<String, Value>) -> Parsed {
    let level = obj.get("level").and_then(|v| v.as_u64()).map(|n| LogLevel::from_syslog(n as u8));
    let timestamp = obj.get("timestamp").and_then(|v| v.as_f64()).map(format_epoch_secs);
    let message = obj.get("short_message").and_then(|v| v.as_str()).map(|s| s.to_owned());
    let target = obj.get("host").and_then(|v| v.as_str()).map(|s| s.to_owned());
    (level, target, timestamp, message)
}

fn parse_otel(obj: &serde_json::Map<String, Value>) -> Parsed {
    let level = obj.get("severityNumber")
        .and_then(|v| v.as_u64())
        .map(|n| LogLevel::from_otel(n as u8))
        .or_else(|| obj.get("severityText").and_then(|v| v.as_str()).and_then(LogLevel::from_str));
    let timestamp = obj.get("timeUnixNano").and_then(|v| v.as_str()).map(format_epoch_nanos);
    let message = obj.get("body").and_then(|v| v.as_str()).map(|s| s.to_owned());
    let target = obj.get("serviceName").and_then(|v| v.as_str()).map(|s| s.to_owned());
    (level, target, timestamp, message)
}

fn parse_splunk(obj: &serde_json::Map<String, Value>) -> Parsed {
    let timestamp = obj.get("time").and_then(|v| v.as_f64()).map(format_epoch_secs);
    let target = obj.get("source").and_then(|v| v.as_str()).map(|s| s.to_owned());

    let (level, message) = match obj.get("event") {
        Some(Value::Object(ev)) => {
            let lvl = ev.get("level").and_then(|v| v.as_str()).and_then(LogLevel::from_str);
            let msg = ev.get("message").and_then(|v| v.as_str()).map(|s| s.to_owned());
            (lvl, msg)
        }
        Some(Value::String(s)) => (None, Some(s.clone())),
        _ => (None, None),
    };
    (level, target, timestamp, message)
}

fn parse_cloudtrail(obj: &serde_json::Map<String, Value>) -> Parsed {
    let timestamp = obj.get("eventTime").and_then(|v| v.as_str()).map(|s| s.to_owned());
    let message = obj.get("eventName").and_then(|v| v.as_str()).map(|s| s.to_owned());
    let target = obj.get("eventSource").and_then(|v| v.as_str()).map(|s| s.to_owned());
    let level = if obj.contains_key("errorCode") { Some(LogLevel::Error) } else { Some(LogLevel::Info) };
    (level, target, timestamp, message)
}

fn parse_cloudwatch(obj: &serde_json::Map<String, Value>) -> Parsed {
    let timestamp = obj.get("timestamp").and_then(|v| v.as_u64()).map(format_epoch_millis);
    let raw_msg = obj.get("message").and_then(|v| v.as_str()).unwrap_or("");

    let (level, message) = if raw_msg.trim_start().starts_with('{') {
        match serde_json::from_str::<Value>(raw_msg) {
            Ok(Value::Object(inner)) => {
                let lvl = inner.get("level")
                    .and_then(|v| v.as_str())
                    .and_then(LogLevel::from_str);
                let inner_msg = inner.get("message")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_owned())
                    .unwrap_or_else(|| raw_msg.to_owned());
                (lvl, Some(inner_msg))
            }
            _ => (None, Some(raw_msg.to_owned())),
        }
    } else {
        (None, Some(raw_msg.to_owned()))
    };
    (level, None, timestamp, message)
}

fn parse_gcp(obj: &serde_json::Map<String, Value>) -> Parsed {
    let level = obj.get("severity").and_then(|v| v.as_str()).map(map_gcp_severity);
    let timestamp = obj.get("time")
        .and_then(|v| v.as_str())
        .or_else(|| obj.get("receiveTimestamp").and_then(|v| v.as_str()))
        .map(|s| s.to_owned());
    let message = obj.get("message").and_then(|v| v.as_str()).map(|s| s.to_owned())
        .or_else(|| extract_nested(obj, "jsonPayload", "message").map(|s| s.to_owned()))
        .or_else(|| obj.get("textPayload").and_then(|v| v.as_str()).map(|s| s.to_owned()));
    (level, None, timestamp, message)
}

fn parse_datadog(obj: &serde_json::Map<String, Value>) -> Parsed {
    let level = obj.get("status").and_then(|v| v.as_str()).and_then(LogLevel::from_str);
    let timestamp = obj.get("timestamp").and_then(|v| v.as_str())
        .or_else(|| obj.get("date").and_then(|v| v.as_str()))
        .map(|s| s.to_owned());
    let message = obj.get("message").and_then(|v| v.as_str()).map(|s| s.to_owned());
    let target = obj.get("service").and_then(|v| v.as_str()).map(|s| s.to_owned());
    (level, target, timestamp, message)
}

fn parse_ecs(obj: &serde_json::Map<String, Value>) -> Parsed {
    let timestamp = obj.get("@timestamp").and_then(|v| v.as_str()).map(|s| s.to_owned());
    let message = obj.get("message").and_then(|v| v.as_str()).map(|s| s.to_owned());
    let level = obj.get("log.level").and_then(|v| v.as_str()).and_then(LogLevel::from_str);
    let target = obj.get("service.name").and_then(|v| v.as_str()).map(|s| s.to_owned());
    (level, target, timestamp, message)
}

fn parse_logstash(obj: &serde_json::Map<String, Value>) -> Parsed {
    let timestamp = obj.get("@timestamp").and_then(|v| v.as_str()).map(|s| s.to_owned());
    let message = obj.get("message").and_then(|v| v.as_str()).map(|s| s.to_owned());
    let level = obj.get("level").and_then(|v| v.as_str()).and_then(LogLevel::from_str);
    let target = obj.get("logger_name").and_then(|v| v.as_str()).map(|s| s.to_owned());
    (level, target, timestamp, message)
}

fn parse_generic(obj: &serde_json::Map<String, Value>) -> Parsed {
    let level = extract_level(obj);
    let target = extract_str_field(obj, &["target", "module", "logger"])
        .or_else(|| extract_nested(obj, "fields", "target"))
        .map(|s| s.to_owned());
    let timestamp = extract_str_field(obj, &["timestamp", "time", "ts", "@timestamp", "datetime"])
        .map(|s| s.to_owned());
    let message = extract_str_field(obj, &["msg", "message", "body"])
        .or_else(|| extract_nested(obj, "fields", "message"))
        .map(|s| s.to_owned());
    (level, target, timestamp, message)
}

fn map_gcp_severity(s: &str) -> LogLevel {
    match s.to_ascii_uppercase().as_str() {
        "EMERGENCY" | "ALERT" | "CRITICAL" | "ERROR" => LogLevel::Error,
        "WARNING" => LogLevel::Warn,
        "DEBUG" => LogLevel::Debug,
        _ => LogLevel::Info, // NOTICE, INFO, DEFAULT, unknown
    }
}

// ---- epoch helpers (no chrono) ----

fn format_epoch_secs(secs: f64) -> String {
    format_unix_secs(secs as i64)
}

fn format_epoch_millis(ms: u64) -> String {
    format_unix_secs((ms / 1000) as i64)
}

fn format_epoch_nanos(ns_str: &str) -> String {
    match ns_str.parse::<u64>() {
        Ok(ns) => format_unix_secs((ns / 1_000_000_000) as i64),
        Err(_) => ns_str.to_owned(),
    }
}

fn format_unix_secs(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let hh = tod / 3600;
    let mm = (tod % 3600) / 60;
    let ss = tod % 60;
    let (y, mo, d) = days_to_ymd(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, hh, mm, ss)
}

/// Convert days-since-1970-01-01 into (year, month, day). Howard Hinnant algorithm.
fn days_to_ymd(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32)
}

// ---- existing helpers (kept for generic fallback) ----

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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> LogLine {
        parse_json_line(line.as_bytes(), 0).expect("parse failed")
    }

    fn detect(line: &str) -> JsonSchema {
        let v: Value = serde_json::from_str(line).unwrap();
        detect_json_schema(v.as_object().unwrap())
    }

    #[test]
    fn epoch_secs_formats_correctly() {
        // 20207 days × 86400 + 28472s → 2025-04-29T07:54:32Z
        assert_eq!(format_unix_secs(1_745_913_272), "2025-04-29T07:54:32Z");
        assert_eq!(format_unix_secs(0), "1970-01-01T00:00:00Z");
        // leap-year boundary: 2024-02-29
        assert_eq!(format_unix_secs(1_709_164_800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn detects_gelf() {
        let line = r#"{"version":"1.1","host":"example.org","short_message":"Login failed","timestamp":1745913272.307,"level":4}"#;
        assert_eq!(detect(line), JsonSchema::Gelf);
        let l = parse(line);
        assert_eq!(l.level, Some(LogLevel::Warn));
        assert_eq!(l.message.as_deref(), Some("Login failed"));
        assert_eq!(l.target.as_deref(), Some("example.org"));
        assert!(l.timestamp.as_deref().unwrap().starts_with("2025-04-29T"));
    }

    #[test]
    fn detects_otel() {
        let line = r#"{"timeUnixNano":"1745913272001000000","severityNumber":17,"severityText":"ERROR","body":"Payment processing failed","serviceName":"checkout"}"#;
        assert_eq!(detect(line), JsonSchema::Otel);
        let l = parse(line);
        assert_eq!(l.level, Some(LogLevel::Error));
        assert_eq!(l.message.as_deref(), Some("Payment processing failed"));
        assert_eq!(l.target.as_deref(), Some("checkout"));
        assert!(l.timestamp.as_deref().unwrap().starts_with("2025-04-29T"));
    }

    #[test]
    fn detects_splunk_hec_object_event() {
        let line = r#"{"time":1745913272.001,"host":"web-01","source":"/var/log/myapp/app.log","sourcetype":"x","index":"main","event":{"level":"INFO","message":"Application started"}}"#;
        assert_eq!(detect(line), JsonSchema::Splunk);
        let l = parse(line);
        assert_eq!(l.level, Some(LogLevel::Info));
        assert_eq!(l.message.as_deref(), Some("Application started"));
        assert_eq!(l.target.as_deref(), Some("/var/log/myapp/app.log"));
    }

    #[test]
    fn detects_splunk_hec_string_event() {
        let line = r#"{"time":1745913274.0,"host":"web-01","source":"/x","sourcetype":"y","index":"web","event":"raw nginx error line"}"#;
        assert_eq!(detect(line), JsonSchema::Splunk);
        let l = parse(line);
        assert_eq!(l.message.as_deref(), Some("raw nginx error line"));
    }

    #[test]
    fn detects_cloudtrail() {
        let line = r#"{"eventVersion":"1.09","eventTime":"2025-04-29T08:10:00Z","eventSource":"signin.amazonaws.com","eventName":"ConsoleLogin","errorCode":"Failed authentication"}"#;
        assert_eq!(detect(line), JsonSchema::CloudTrail);
        let l = parse(line);
        assert_eq!(l.level, Some(LogLevel::Error));
        assert_eq!(l.message.as_deref(), Some("ConsoleLogin"));
        assert_eq!(l.target.as_deref(), Some("signin.amazonaws.com"));
        assert_eq!(l.timestamp.as_deref(), Some("2025-04-29T08:10:00Z"));
    }

    #[test]
    fn detects_cloudtrail_no_error() {
        let line = r#"{"eventTime":"2025-04-29T08:00:01Z","eventSource":"s3.amazonaws.com","eventName":"GetObject"}"#;
        assert_eq!(detect(line), JsonSchema::CloudTrail);
        let l = parse(line);
        assert_eq!(l.level, Some(LogLevel::Info));
    }

    #[test]
    fn detects_cloudwatch_with_inner_json() {
        let line = r#"{"timestamp":1745913272500,"message":"{\"timestamp\":\"2025-04-29T08:14:32.500Z\",\"level\":\"ERROR\",\"message\":\"Payment failed\"}"}"#;
        assert_eq!(detect(line), JsonSchema::CloudWatch);
        let l = parse(line);
        assert_eq!(l.level, Some(LogLevel::Error));
        assert_eq!(l.message.as_deref(), Some("Payment failed"));
        assert!(l.timestamp.as_deref().unwrap().starts_with("2025-04-29T"));
    }

    #[test]
    fn detects_cloudwatch_plain_message() {
        let line = r#"{"timestamp":1745913272001,"message":"START RequestId: abc Version: $LATEST"}"#;
        assert_eq!(detect(line), JsonSchema::CloudWatch);
        let l = parse(line);
        assert_eq!(l.level, None);
        assert_eq!(l.message.as_deref(), Some("START RequestId: abc Version: $LATEST"));
    }

    #[test]
    fn detects_gcp() {
        let line = r#"{"severity":"WARNING","message":"Slow query","time":"2025-04-29T08:14:33.500Z","jsonPayload":{"duration_ms":2345}}"#;
        assert_eq!(detect(line), JsonSchema::Gcp);
        let l = parse(line);
        assert_eq!(l.level, Some(LogLevel::Warn));
        assert_eq!(l.message.as_deref(), Some("Slow query"));
        assert_eq!(l.timestamp.as_deref(), Some("2025-04-29T08:14:33.500Z"));
    }

    #[test]
    fn detects_gcp_critical() {
        let line = r#"{"severity":"CRITICAL","message":"OOM","time":"2025-04-29T08:15:00.000Z"}"#;
        let l = parse(line);
        assert_eq!(l.level, Some(LogLevel::Error));
    }

    #[test]
    fn detects_gcp_json_payload_message() {
        let line = r#"{"severity":"INFO","time":"2025-04-29T08:14:33.500Z","jsonPayload":{"message":"hi from payload"}}"#;
        let l = parse(line);
        assert_eq!(l.message.as_deref(), Some("hi from payload"));
    }

    #[test]
    fn detects_datadog() {
        let line = r#"{"timestamp":"2025-04-29T08:14:33.000Z","status":"error","service":"payment-api","host":"web-01","message":"Charge failed","ddsource":"nodejs","ddtags":"env:prod"}"#;
        assert_eq!(detect(line), JsonSchema::Datadog);
        let l = parse(line);
        assert_eq!(l.level, Some(LogLevel::Error));
        assert_eq!(l.message.as_deref(), Some("Charge failed"));
        assert_eq!(l.target.as_deref(), Some("payment-api"));
        assert_eq!(l.timestamp.as_deref(), Some("2025-04-29T08:14:33.000Z"));
    }

    #[test]
    fn detects_ecs() {
        let line = r#"{"@timestamp":"2025-04-29T08:14:40.789Z","log.level":"error","message":"Database query failed","ecs.version":"8.11.0","service.name":"auth-api"}"#;
        assert_eq!(detect(line), JsonSchema::Ecs);
        let l = parse(line);
        assert_eq!(l.level, Some(LogLevel::Error));
        assert_eq!(l.message.as_deref(), Some("Database query failed"));
        assert_eq!(l.target.as_deref(), Some("auth-api"));
        assert_eq!(l.timestamp.as_deref(), Some("2025-04-29T08:14:40.789Z"));
    }

    #[test]
    fn detects_logstash() {
        let line = r#"{"@timestamp":"2025-04-29T08:14:35.456Z","@version":"1","level":"WARN","message":"pool nearly exhausted","logger_name":"com.zaxxer.hikari.HikariPool"}"#;
        assert_eq!(detect(line), JsonSchema::Logstash);
        let l = parse(line);
        assert_eq!(l.level, Some(LogLevel::Warn));
        assert_eq!(l.message.as_deref(), Some("pool nearly exhausted"));
        assert_eq!(l.target.as_deref(), Some("com.zaxxer.hikari.HikariPool"));
    }

    #[test]
    fn falls_back_to_generic() {
        let line = r#"{"level":"info","msg":"hello","target":"my::mod"}"#;
        assert_eq!(detect(line), JsonSchema::Generic);
        let l = parse(line);
        assert_eq!(l.level, Some(LogLevel::Info));
        assert_eq!(l.message.as_deref(), Some("hello"));
        assert_eq!(l.target.as_deref(), Some("my::mod"));
    }
}
