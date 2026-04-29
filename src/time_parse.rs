/// Timestamp parsing for log-line timestamps and user input.
///
/// # Log-line timestamps
/// `parse_ts_key` accepts only full YYYY-MM-DDTHH:MM:SS strings (19 chars).
/// HH:MM:SS-only timestamps return None — lines without a date cannot be
/// compared reliably to date-bound user input, so they always pass the filter.
///
/// # User input
/// `parse_user_input` handles:
/// - Absolute with date: "2024-01-01T10:00:00" or "2024-01-01 10:00:00"
/// - Time-only "10:00:00" — filled with today's date
/// - Relative: "-30m", "-1h", "-5s", "-2d"
/// - Empty → None (no bound)
///
/// # Relative time
/// Manual arithmetic is used (no external crates). Month lengths are fixed
/// [31,28,31,...] — no leap-year handling (documented limitation).
/// Only `-Nd`, `-Nh`, `-Nm`, `-Ns` patterns are supported.

/// Validate "YYYY-MM-DDTHH:MM:SS" — 19-char format check.
fn is_ts_prefix(s: &str) -> bool {
    if s.len() < 19 {
        return false;
    }
    let b = s.as_bytes();
    // YYYY-MM-DDTHH:MM:SS
    b[4] == b'-' && b[7] == b'-' && (b[10] == b'T' || b[10] == b' ')
        && b[13] == b':' && b[16] == b':'
        && b[0..4].iter().all(|c| c.is_ascii_digit())
        && b[5..7].iter().all(|c| c.is_ascii_digit())
        && b[8..10].iter().all(|c| c.is_ascii_digit())
        && b[11..13].iter().all(|c| c.is_ascii_digit())
        && b[14..16].iter().all(|c| c.is_ascii_digit())
        && b[17..19].iter().all(|c| c.is_ascii_digit())
}

/// Validate "HH:MM:SS" — 8-char time-only format.
fn is_time_only(s: &str) -> bool {
    if s.len() < 8 {
        return false;
    }
    let b = s.as_bytes();
    b[2] == b':' && b[5] == b':'
        && b[0..2].iter().all(|c| c.is_ascii_digit())
        && b[3..5].iter().all(|c| c.is_ascii_digit())
        && b[6..8].iter().all(|c| c.is_ascii_digit())
}

/// Get current date as "YYYY-MM-DD" using SystemTime.
pub fn today_key() -> String {
    let (y, mo, d, _, _, _) = system_ymdhms();
    format!("{:04}-{:02}-{:02}", y, mo, d)
}

/// Get current datetime as "YYYY-MM-DDTHH:MM:SS" using SystemTime.
pub fn now_key() -> String {
    let (y, mo, d, h, mi, s) = system_ymdhms();
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", y, mo, d, h, mi, s)
}

/// Decompose SystemTime into (year, month, day, hour, min, sec).
/// Uses simple epoch arithmetic (no leap-second handling).
fn system_ymdhms() -> (u32, u32, u32, u32, u32, u32) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    epoch_secs_to_ymdhms(secs as i64)
}

/// Fixed month lengths (no leap year).
const MONTH_DAYS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// Convert epoch seconds (may be negative) to (y,mo,d,h,mi,s).
/// Uses simplified calendar arithmetic — no leap-year correction.
fn epoch_secs_to_ymdhms(mut secs: i64) -> (u32, u32, u32, u32, u32, u32) {
    // Handle negative by offsetting into a safe positive range
    // We add enough years worth of seconds to make it positive.
    let offset_years: i64 = if secs < 0 { ((-secs / 31_536_000) + 1) * 400 } else { 0 };
    secs += offset_years * 365 * 86400;

    let time_of_day = (secs % 86400) as u32;
    let mut days = (secs / 86400) as u32;

    let h = time_of_day / 3600;
    let mi = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;

    // days since 1970-01-01
    // year loop
    let mut year: u32 = 1970;
    loop {
        let days_in_year = 365u32;
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }

    // Adjust back for the artificial offset
    year = year.saturating_sub(offset_years as u32 / 365);

    let mut month: u32 = 1;
    for &md in &MONTH_DAYS {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }

    (year, month, days + 1, h, mi, s)
}

/// Convert (y,mo,d,h,mi,s) to a pseudo-epoch integer (seconds since 0000-01-01).
/// Uses simplified calendar (no leap years, fixed month lengths).
fn ymdhms_to_pseudo_epoch(y: u32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
    let mut days: i64 = (y as i64) * 365;
    for m in 1..mo {
        days += MONTH_DAYS[(m - 1) as usize] as i64;
    }
    days += (d as i64) - 1;
    days * 86400 + (h as i64) * 3600 + (mi as i64) * 60 + s as i64
}

/// Convert pseudo-epoch back to (y,mo,d,h,mi,s).
fn pseudo_epoch_to_ymdhms(mut secs: i64) -> (u32, u32, u32, u32, u32, u32) {
    // Clamp to non-negative
    if secs < 0 {
        secs = 0;
    }
    let time_of_day = (secs % 86400) as u32;
    let mut days = (secs / 86400) as u32;

    let h = time_of_day / 3600;
    let mi = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;

    let year = days / 365;
    days %= 365;

    let mut month: u32 = 1;
    for &md in &MONTH_DAYS {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }

    (year, month, days + 1, h, mi, s)
}

/// Add/subtract seconds from a key string "YYYY-MM-DDTHH:MM:SS".
/// Returns None if `base` is not a valid timestamp key.
pub fn offset_key(base: &str, delta_secs: i64) -> Option<String> {
    if !is_ts_prefix(base) {
        return None;
    }
    let y: u32 = base[0..4].parse().ok()?;
    let mo: u32 = base[5..7].parse().ok()?;
    let d: u32 = base[8..10].parse().ok()?;
    let h: u32 = base[11..13].parse().ok()?;
    let mi: u32 = base[14..16].parse().ok()?;
    let s: u32 = base[17..19].parse().ok()?;

    let epoch = ymdhms_to_pseudo_epoch(y, mo, d, h, mi, s);
    let new_epoch = epoch + delta_secs;
    let (ny, nmo, nd, nh, nmi, ns) = pseudo_epoch_to_ymdhms(new_epoch);
    Some(format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", ny, nmo, nd, nh, nmi, ns))
}

/// Parse a log-line timestamp into a sortable key "YYYY-MM-DDTHH:MM:SS".
/// Returns None for time-only timestamps (HH:MM:SS) — those lines always pass.
pub fn parse_ts_key(ts: &str) -> Option<String> {
    let s = ts.trim();
    if is_ts_prefix(s) {
        // Normalise the separator to 'T'
        let mut key = s[..19].to_string();
        // Safety: index 10 is ASCII
        unsafe {
            key.as_bytes_mut()[10] = b'T';
        }
        Some(key)
    } else {
        None
    }
}

/// Parse user-typed input into a sortable key "YYYY-MM-DDTHH:MM:SS".
///
/// Accepted formats:
/// - `"2024-01-01T10:00:00"` or `"2024-01-01 10:00:00"` — absolute with date
/// - `"10:00:00"` — time-only, filled with today's date
/// - `"-30m"`, `"-1h"`, `"-5s"`, `"-2d"` — relative from `now_key`
/// - `""` — returns None (no filter)
pub fn parse_user_input(s: &str, now_key_str: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Absolute with date
    if is_ts_prefix(s) {
        return parse_ts_key(s);
    }

    // Time-only: "HH:MM:SS"
    if is_time_only(s) && s.len() == 8 {
        let today = today_key();
        let combined = format!("{}T{}", today, s);
        return parse_ts_key(&combined);
    }

    // Relative: starts with '-'
    if let Some(rest) = s.strip_prefix('-') {
        let (num_str, unit) = if rest.ends_with('d') {
            (&rest[..rest.len() - 1], 'd')
        } else if rest.ends_with('h') {
            (&rest[..rest.len() - 1], 'h')
        } else if rest.ends_with('m') {
            (&rest[..rest.len() - 1], 'm')
        } else if rest.ends_with('s') {
            (&rest[..rest.len() - 1], 's')
        } else {
            return None;
        };

        let n: i64 = num_str.parse().ok()?;
        let delta_secs: i64 = match unit {
            'd' => n * 86400,
            'h' => n * 3600,
            'm' => n * 60,
            's' => n,
            _ => return None,
        };

        // now_key_str should be "YYYY-MM-DDTHH:MM:SS"
        let base = if is_ts_prefix(now_key_str) {
            now_key_str.to_string()
        } else {
            now_key()
        };

        return offset_key(&base, -delta_secs);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ts_key_normalises_separator() {
        let key = parse_ts_key("2024-03-15 08:30:45").unwrap();
        assert_eq!(key, "2024-03-15T08:30:45");
    }

    #[test]
    fn parse_ts_key_rejects_time_only() {
        assert!(parse_ts_key("08:30:45").is_none());
    }

    #[test]
    fn parse_user_input_empty() {
        assert!(parse_user_input("", "2024-01-01T00:00:00").is_none());
    }

    #[test]
    fn parse_user_input_absolute() {
        let key = parse_user_input("2024-03-15T08:30:45", "2024-01-01T00:00:00").unwrap();
        assert_eq!(key, "2024-03-15T08:30:45");
    }

    #[test]
    fn parse_user_input_relative_hours() {
        let key = parse_user_input("-1h", "2024-03-15T08:30:45").unwrap();
        assert_eq!(key, "2024-03-15T07:30:45");
    }

    #[test]
    fn parse_user_input_relative_days() {
        let key = parse_user_input("-1d", "2024-03-15T08:30:45").unwrap();
        assert_eq!(key, "2024-03-14T08:30:45");
    }

    #[test]
    fn offset_key_backward() {
        let k = offset_key("2024-03-15T08:00:00", -3600).unwrap();
        assert_eq!(k, "2024-03-15T07:00:00");
    }
}
