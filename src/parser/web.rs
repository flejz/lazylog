use regex::Regex;
use std::sync::OnceLock;
use super::{LogLevel, LogLine};

static NGINX_ERROR_RE: OnceLock<Regex> = OnceLock::new();
static CLF_RE: OnceLock<Regex> = OnceLock::new();
static IIS_DATA_RE: OnceLock<Regex> = OnceLock::new();

fn nginx_error_re() -> &'static Regex {
    NGINX_ERROR_RE.get_or_init(|| {
        Regex::new(
            r"^(\d{4})/(\d{2})/(\d{2}) (\d{2}:\d{2}:\d{2}) \[(\w+)\] \d+#\d+: (?:\*\d+ )?(.*)$"
        ).unwrap()
    })
}

fn clf_re() -> &'static Regex {
    CLF_RE.get_or_init(|| {
        Regex::new(
            r#"^(\S+) \S+ \S+ \[(\d{2})/(\w{3})/(\d{4}):(\d{2}:\d{2}:\d{2}) ([+-]\d{4})\] "(\S+) (\S+) [^"]*" (\d{3}) (\S+)"#
        ).unwrap()
    })
}

fn iis_data_re() -> &'static Regex {
    IIS_DATA_RE.get_or_init(|| {
        Regex::new(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} ").unwrap()
    })
}

fn month_to_num(m: &str) -> Option<&'static str> {
    Some(match m {
        "Jan" => "01", "Feb" => "02", "Mar" => "03", "Apr" => "04",
        "May" => "05", "Jun" => "06", "Jul" => "07", "Aug" => "08",
        "Sep" => "09", "Oct" => "10", "Nov" => "11", "Dec" => "12",
        _ => return None,
    })
}

fn level_from_status(status: u16) -> LogLevel {
    match status {
        500..=599 => LogLevel::Error,
        400..=499 => LogLevel::Warn,
        _ => LogLevel::Info,
    }
}

pub fn parse_web_line(raw: &[u8], line_no: u64) -> Option<LogLine> {
    let s = std::str::from_utf8(raw).ok()?;
    let s = s.trim_end_matches(|c| c == '\r' || c == '\n');
    if s.is_empty() {
        return None;
    }

    // IIS W3C metadata/comment lines — caller skips
    if s.starts_with('#') {
        return None;
    }

    // 1) Nginx error log: distinctive YYYY/MM/DD timestamp + [level]
    if let Some(caps) = nginx_error_re().captures(s) {
        let yyyy = caps.get(1).unwrap().as_str();
        let mm   = caps.get(2).unwrap().as_str();
        let dd   = caps.get(3).unwrap().as_str();
        let time = caps.get(4).unwrap().as_str();
        let level_str = caps.get(5).unwrap().as_str();
        let rest = caps.get(6).map(|m| m.as_str()).unwrap_or("");

        let timestamp = format!("{}-{}-{}T{}", yyyy, mm, dd, time);
        let level = LogLevel::from_str(level_str);
        let message = match rest.find(", client:") {
            Some(idx) => rest[..idx].to_owned(),
            None => rest.to_owned(),
        };

        return Some(LogLine {
            raw: raw.to_vec(),
            level,
            target: None,
            timestamp: Some(timestamp),
            message: Some(message),
            line_no,
            file_idx: 0,
        });
    }

    // 2) IIS W3C data line: starts with `YYYY-MM-DD HH:MM:SS `
    if iis_data_re().is_match(s) {
        let parts: Vec<&str> = s.split_whitespace().collect();
        // Default fields: date time c-ip cs-username s-ip s-port cs-method
        // cs-uri-stem cs-uri-query sc-status sc-bytes cs-bytes time-taken
        if parts.len() >= 13 {
            let date       = parts[0];
            let time       = parts[1];
            let c_ip       = parts[2];
            let method     = parts[6];
            let uri_stem   = parts[7];
            let status_str = parts[9];
            let bytes      = parts[10];
            let time_taken = parts[12];

            let status: u16 = status_str.parse().unwrap_or(0);
            let level = Some(level_from_status(status));
            let timestamp = format!("{}T{}Z", date, time);
            let message = format!(
                "method={} path={} status={} bytes={} ms={}",
                method, uri_stem, status_str, bytes, time_taken
            );

            return Some(LogLine {
                raw: raw.to_vec(),
                level,
                target: Some(c_ip.to_owned()),
                timestamp: Some(timestamp),
                message: Some(message),
                line_no,
                file_idx: 0,
            });
        }
    }

    // 3) Apache CLF / Combined / Nginx access (CLF bracket timestamp)
    if let Some(caps) = clf_re().captures(s) {
        let ip         = caps.get(1).unwrap().as_str();
        let dd         = caps.get(2).unwrap().as_str();
        let mon        = caps.get(3).unwrap().as_str();
        let yyyy       = caps.get(4).unwrap().as_str();
        let time       = caps.get(5).unwrap().as_str();
        let tz         = caps.get(6).unwrap().as_str();
        let method     = caps.get(7).unwrap().as_str();
        let path       = caps.get(8).unwrap().as_str();
        let status_str = caps.get(9).unwrap().as_str();
        let bytes      = caps.get(10).unwrap().as_str();

        let mon_num = month_to_num(mon)?;
        let timestamp = format!("{}-{}-{}T{}{}", yyyy, mon_num, dd, time, tz);
        let status: u16 = status_str.parse().unwrap_or(0);
        let level = Some(level_from_status(status));
        let message = format!(
            "method={} path={} status={} bytes={}",
            method, path, status_str, bytes
        );

        return Some(LogLine {
            raw: raw.to_vec(),
            level,
            target: Some(ip.to_owned()),
            timestamp: Some(timestamp),
            message: Some(message),
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
    fn parses_apache_common_first_line() {
        let line = b"127.0.0.1 - frank [10/Oct/2000:13:55:36 -0700] \"GET /apache_pb.gif HTTP/1.0\" 200 2326";
        let p = parse_web_line(line, 0).expect("parsed");
        assert_eq!(p.level, Some(LogLevel::Info));
        assert_eq!(p.target.as_deref(), Some("127.0.0.1"));
        assert_eq!(p.timestamp.as_deref(), Some("2000-10-10T13:55:36-0700"));
        assert_eq!(
            p.message.as_deref(),
            Some("method=GET path=/apache_pb.gif status=200 bytes=2326")
        );
    }

    #[test]
    fn parses_apache_common_404_as_warn() {
        let line = b"172.16.0.5 - - [10/Oct/2000:13:55:39 -0700] \"GET /nonexistent.html HTTP/1.1\" 404 217";
        let p = parse_web_line(line, 0).expect("parsed");
        assert_eq!(p.level, Some(LogLevel::Warn));
        assert_eq!(p.target.as_deref(), Some("172.16.0.5"));
    }

    #[test]
    fn parses_apache_common_dash_bytes() {
        let line = b"192.168.1.100 - - [10/Oct/2000:13:55:40 -0700] \"GET /images/logo.png HTTP/1.1\" 304 -";
        let p = parse_web_line(line, 0).expect("parsed");
        assert_eq!(p.level, Some(LogLevel::Info));
        assert_eq!(
            p.message.as_deref(),
            Some("method=GET path=/images/logo.png status=304 bytes=-")
        );
    }

    #[test]
    fn parses_apache_common_403_as_warn() {
        let line = b"10.0.0.2 - bob [10/Oct/2000:13:55:41 -0700] \"DELETE /api/users/42 HTTP/1.1\" 403 89";
        let p = parse_web_line(line, 0).expect("parsed");
        assert_eq!(p.level, Some(LogLevel::Warn));
        assert_eq!(p.target.as_deref(), Some("10.0.0.2"));
    }

    #[test]
    fn parses_apache_combined_first_line() {
        let line = b"127.0.0.1 - frank [10/Oct/2000:13:55:36 -0700] \"GET /apache_pb.gif HTTP/1.0\" 200 2326 \"http://www.example.com/start.html\" \"Mozilla/4.08 [en] (Win98; I ;Nav)\"";
        let p = parse_web_line(line, 0).expect("parsed");
        assert_eq!(p.level, Some(LogLevel::Info));
        assert_eq!(p.target.as_deref(), Some("127.0.0.1"));
        assert_eq!(p.timestamp.as_deref(), Some("2000-10-10T13:55:36-0700"));
        assert_eq!(
            p.message.as_deref(),
            Some("method=GET path=/apache_pb.gif status=200 bytes=2326")
        );
    }

    #[test]
    fn parses_nginx_access_first_line() {
        let line = b"192.168.1.1 - - [03/Mar/2025:15:42:31 +0000] \"GET /api/users HTTP/1.1\" 200 1234 \"https://example.com/dashboard\" \"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36\"";
        let p = parse_web_line(line, 0).expect("parsed");
        assert_eq!(p.level, Some(LogLevel::Info));
        assert_eq!(p.target.as_deref(), Some("192.168.1.1"));
        assert_eq!(p.timestamp.as_deref(), Some("2025-03-03T15:42:31+0000"));
    }

    #[test]
    fn parses_nginx_access_http2() {
        let line = b"172.16.1.10 - - [03/Mar/2025:15:42:33 +0000] \"GET /static/app.js HTTP/2.0\" 304 0 \"https://example.com/\" \"Mozilla/5.0 (Macintosh)\"";
        let p = parse_web_line(line, 0).expect("parsed");
        assert_eq!(p.level, Some(LogLevel::Info));
        assert_eq!(
            p.message.as_deref(),
            Some("method=GET path=/static/app.js status=304 bytes=0")
        );
    }

    #[test]
    fn parses_nginx_access_401_as_warn() {
        let line = b"192.168.1.200 - - [03/Mar/2025:15:42:34 +0000] \"GET /api/admin/users HTTP/1.1\" 401 45 \"-\" \"python-requests/2.28.0\"";
        let p = parse_web_line(line, 0).expect("parsed");
        assert_eq!(p.level, Some(LogLevel::Warn));
    }

    #[test]
    fn parses_nginx_error_with_client() {
        let line = b"2025/03/03 15:42:31 [error] 12345#0: *67 open() \"/var/www/html/missing.html\" failed (2: No such file or directory), client: 192.168.1.1, server: example.com, request: \"GET /missing.html HTTP/1.1\", host: \"example.com\"";
        let p = parse_web_line(line, 0).expect("parsed");
        assert_eq!(p.level, Some(LogLevel::Error));
        assert_eq!(p.target, None);
        assert_eq!(p.timestamp.as_deref(), Some("2025-03-03T15:42:31"));
        assert_eq!(
            p.message.as_deref(),
            Some("open() \"/var/www/html/missing.html\" failed (2: No such file or directory)")
        );
    }

    #[test]
    fn parses_nginx_error_warn_level() {
        let line = b"2025/03/03 15:42:33 [warn] 12345#0: *68 upstream response time 5.123 while reading response header from upstream, client: 10.0.0.5, server: api.example.com, request: \"POST /api/process HTTP/1.1\", upstream: \"http://127.0.0.1:8080/api/process\"";
        let p = parse_web_line(line, 0).expect("parsed");
        assert_eq!(p.level, Some(LogLevel::Warn));
        assert_eq!(
            p.message.as_deref(),
            Some("upstream response time 5.123 while reading response header from upstream")
        );
    }

    #[test]
    fn parses_nginx_error_crit_level() {
        let line = b"2025/03/03 15:42:40 [crit] 12345#0: *0 SSL_do_handshake() failed (SSL: error:14094418:SSL routines:ssl3_read_bytes:tlsv1 alert unknown ca) while SSL handshaking, client: 192.168.100.5, server: 0.0.0.0:443";
        let p = parse_web_line(line, 0).expect("parsed");
        assert_eq!(p.level, Some(LogLevel::Error));
        assert_eq!(
            p.message.as_deref(),
            Some("SSL_do_handshake() failed (SSL: error:14094418:SSL routines:ssl3_read_bytes:tlsv1 alert unknown ca) while SSL handshaking")
        );
    }

    #[test]
    fn parses_nginx_error_notice_no_cid() {
        let line = b"2025/03/03 15:43:01 [notice] 1#1: signal process started";
        let p = parse_web_line(line, 0).expect("parsed");
        assert_eq!(p.level, Some(LogLevel::Info));
        assert_eq!(p.message.as_deref(), Some("signal process started"));
        assert_eq!(p.target, None);
    }

    #[test]
    fn iis_comment_line_returns_none() {
        let line = b"#Software: Microsoft HTTP Server API 2.0";
        assert!(parse_web_line(line, 0).is_none());
    }

    #[test]
    fn iis_fields_header_returns_none() {
        let line = b"#Fields: date time c-ip cs-username s-ip s-port cs-method cs-uri-stem cs-uri-query sc-status sc-bytes cs-bytes time-taken cs(User-Agent) cs(Referer)";
        assert!(parse_web_line(line, 0).is_none());
    }

    #[test]
    fn parses_iis_w3c_data_200() {
        let line = b"2025-04-29 08:00:01 192.168.1.10 - 10.0.0.1 80 GET /default.aspx - 200 5432 512 123 Mozilla/5.0+(Windows+NT+10.0;+Win64;+x64) -";
        let p = parse_web_line(line, 0).expect("parsed");
        assert_eq!(p.level, Some(LogLevel::Info));
        assert_eq!(p.target.as_deref(), Some("192.168.1.10"));
        assert_eq!(p.timestamp.as_deref(), Some("2025-04-29T08:00:01Z"));
        assert_eq!(
            p.message.as_deref(),
            Some("method=GET path=/default.aspx status=200 bytes=5432 ms=123")
        );
    }

    #[test]
    fn parses_iis_w3c_data_403_warn() {
        let line = b"2025-04-29 08:00:03 10.0.0.50 - 10.0.0.1 443 GET /secure/data - 403 128 256 12 curl/7.64.1 -";
        let p = parse_web_line(line, 0).expect("parsed");
        assert_eq!(p.level, Some(LogLevel::Warn));
        assert_eq!(p.target.as_deref(), Some("10.0.0.50"));
        assert_eq!(
            p.message.as_deref(),
            Some("method=GET path=/secure/data status=403 bytes=128 ms=12")
        );
    }

    #[test]
    fn parses_iis_w3c_data_204_info() {
        let line = b"2025-04-29 08:01:00 192.168.1.100 admin 10.0.0.1 443 DELETE /api/users/99 - 204 0 128 88 python-requests/2.28.0 -";
        let p = parse_web_line(line, 0).expect("parsed");
        assert_eq!(p.level, Some(LogLevel::Info));
        assert_eq!(p.target.as_deref(), Some("192.168.1.100"));
        assert_eq!(
            p.message.as_deref(),
            Some("method=DELETE path=/api/users/99 status=204 bytes=0 ms=88")
        );
    }
}
