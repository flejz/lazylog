use std::sync::OnceLock;
use regex::Regex;
use super::{LogLevel, LogLine};

fn re_rfc5424() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r#"^<(\d+)>1 (\S+) (\S+) (\S+) (\S+) (\S+) (-|\[[^\]]*\](?:\[[^\]]*\])*)\s*(.*)$"#,
        )
        .unwrap()
    })
}

fn re_rfc3164() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"^<(\d+)>(\w{3}\s+\d{1,2} \d{2}:\d{2}:\d{2}) (\S+) (.*)$").unwrap()
    })
}

fn re_dmesg() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^\[\s*(\d+\.\d+)\]\s*(.*)$").unwrap())
}

fn re_journald() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"^(\w{3}\s+\d{1,2} \d{2}:\d{2}:\d{2}) (\S+) ([\w.\-]+)(?:\[\d+\])?: (.*)$",
        )
        .unwrap()
    })
}

pub fn parse_syslog_line(raw: &[u8], line_no: u64) -> Option<LogLine> {
    let s = std::str::from_utf8(raw).ok()?;
    let s = s.trim_end_matches(['\r', '\n']);

    if s.starts_with('<') {
        if let Some(c) = re_rfc5424().captures(s) {
            let pri: u16 = c[1].parse().ok()?;
            let level = LogLevel::from_syslog((pri & 7) as u8);
            return Some(LogLine {
                raw: raw.to_vec(),
                level: Some(level),
                target: Some(c[3].to_string()),
                timestamp: Some(c[2].to_string()),
                message: Some(c[8].to_string()),
                line_no,
                file_idx: 0,
            });
        }
        if let Some(c) = re_rfc3164().captures(s) {
            let pri: u16 = c[1].parse().ok()?;
            let level = LogLevel::from_syslog((pri & 7) as u8);
            return Some(LogLine {
                raw: raw.to_vec(),
                level: Some(level),
                target: Some(c[3].to_string()),
                timestamp: Some(c[2].to_string()),
                message: Some(c[4].to_string()),
                line_no,
                file_idx: 0,
            });
        }
        return None;
    }

    if s.starts_with('[') {
        if let Some(c) = re_dmesg().captures(s) {
            return Some(LogLine {
                raw: raw.to_vec(),
                level: Some(LogLevel::Info),
                target: None,
                timestamp: Some(c[1].to_string()),
                message: Some(c[2].to_string()),
                line_no,
                file_idx: 0,
            });
        }
        return None;
    }

    if let Some(c) = re_journald().captures(s) {
        return Some(LogLine {
            raw: raw.to_vec(),
            level: None,
            target: Some(c[3].to_string()),
            timestamp: Some(c[1].to_string()),
            message: Some(c[4].to_string()),
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
    fn parses_rfc5424() {
        let line = b"<34>1 2025-01-03T14:07:15.003Z mymachine.example.com su 12345 ID47 - 'su root' failed for user on /dev/pts/0";
        let l = parse_syslog_line(line, 0).expect("should parse");
        // 34 & 7 = 2 (crit) -> Error
        assert_eq!(l.level, Some(LogLevel::Error));
        assert_eq!(l.timestamp.as_deref(), Some("2025-01-03T14:07:15.003Z"));
        assert_eq!(l.target.as_deref(), Some("mymachine.example.com"));
        assert_eq!(
            l.message.as_deref(),
            Some("'su root' failed for user on /dev/pts/0")
        );
    }

    #[test]
    fn parses_rfc5424_with_structured_data() {
        let line = b"<165>1 2003-10-11T22:14:15.003Z mymachine.example.com evntslog - ID47 [exampleSDID@32473 iut=\"3\" eventSource=\"App\" eventID=\"1011\"] BOMAn application event log entry";
        let l = parse_syslog_line(line, 1).expect("should parse");
        // 165 & 7 = 5 -> Info (notice)
        assert_eq!(l.level, Some(LogLevel::Info));
        assert_eq!(l.timestamp.as_deref(), Some("2003-10-11T22:14:15.003Z"));
        assert_eq!(l.target.as_deref(), Some("mymachine.example.com"));
        assert_eq!(
            l.message.as_deref(),
            Some("BOMAn application event log entry")
        );
    }

    #[test]
    fn parses_rfc3164() {
        let line = b"<34>Oct 11 22:14:15 mymachine su: 'su root' failed for lonvick on /dev/pts/8";
        let l = parse_syslog_line(line, 0).expect("should parse");
        assert_eq!(l.level, Some(LogLevel::Error));
        assert_eq!(l.timestamp.as_deref(), Some("Oct 11 22:14:15"));
        assert_eq!(l.target.as_deref(), Some("mymachine"));
        assert_eq!(
            l.message.as_deref(),
            Some("su: 'su root' failed for lonvick on /dev/pts/8")
        );
    }

    #[test]
    fn parses_rfc3164_double_space_day() {
        let line = b"<28>Oct  5 09:10:01 mymachine CRON[9999]: (root) CMD (true)";
        let l = parse_syslog_line(line, 0).expect("should parse");
        // 28 & 7 = 4 -> Warn
        assert_eq!(l.level, Some(LogLevel::Warn));
        assert_eq!(l.timestamp.as_deref(), Some("Oct  5 09:10:01"));
        assert_eq!(l.target.as_deref(), Some("mymachine"));
    }

    #[test]
    fn parses_dmesg() {
        let line = b"[    0.000000] Linux version 6.1.0-18-amd64 (debian-kernel@lists.debian.org)";
        let l = parse_syslog_line(line, 0).expect("should parse");
        assert_eq!(l.level, Some(LogLevel::Info));
        assert_eq!(l.timestamp.as_deref(), Some("0.000000"));
        assert_eq!(l.target, None);
        assert_eq!(
            l.message.as_deref(),
            Some("Linux version 6.1.0-18-amd64 (debian-kernel@lists.debian.org)")
        );
    }

    #[test]
    fn parses_dmesg_large_offset() {
        let line = b"[10021.123456] EXT4-fs error (device sda2): bad block bitmap checksum";
        let l = parse_syslog_line(line, 0).expect("should parse");
        assert_eq!(l.level, Some(LogLevel::Info));
        assert_eq!(l.timestamp.as_deref(), Some("10021.123456"));
        assert_eq!(
            l.message.as_deref(),
            Some("EXT4-fs error (device sda2): bad block bitmap checksum")
        );
    }

    #[test]
    fn parses_journald_short() {
        let line = b"Apr 29 08:14:31 epsilon gdm-password[825]: AccountsService-DEBUG(+): ActUserManager: ignoring unspecified session";
        let l = parse_syslog_line(line, 0).expect("should parse");
        assert_eq!(l.level, None);
        assert_eq!(l.timestamp.as_deref(), Some("Apr 29 08:14:31"));
        assert_eq!(l.target.as_deref(), Some("gdm-password"));
        assert_eq!(
            l.message.as_deref(),
            Some("AccountsService-DEBUG(+): ActUserManager: ignoring unspecified session")
        );
    }

    #[test]
    fn parses_journald_short_no_pid() {
        let line = b"Apr 29 08:14:32 epsilon kernel: usb 1-3: new high-speed USB device number 4 using xhci_hcd";
        let l = parse_syslog_line(line, 0).expect("should parse");
        assert_eq!(l.level, None);
        assert_eq!(l.target.as_deref(), Some("kernel"));
        assert_eq!(
            l.message.as_deref(),
            Some("usb 1-3: new high-speed USB device number 4 using xhci_hcd")
        );
    }

    #[test]
    fn returns_none_for_garbage() {
        assert!(parse_syslog_line(b"not a log line", 0).is_none());
    }
}
