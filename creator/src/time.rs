//! Timestamp parsing and formatting.

/// Parse a timestamp into seconds.
///
/// Accepts `HH:MM:SS,mmm` (SRT), `HH:MM:SS.mmm` / `MM:SS.mmm` (WebVTT),
/// `M:SS` and `H:MM:SS` (YouTube), optionally wrapped in `[]` or `()`,
/// and plain seconds such as `83` or `83.5`.
pub fn parse_timestamp(s: &str) -> Option<f64> {
    let s = s
        .trim()
        .trim_start_matches(['[', '('])
        .trim_end_matches([']', ')'])
        .trim();
    if s.is_empty() {
        return None;
    }
    let s = s.replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() > 3 {
        return None;
    }
    let mut total = 0.0f64;
    for (i, p) in parts.iter().enumerate() {
        let last = i == parts.len() - 1;
        if p.is_empty() {
            return None;
        }
        let v: f64 = if last {
            if !p.chars().all(|c| c.is_ascii_digit() || c == '.')
                || p.matches('.').count() > 1
                || p.starts_with('.')
                || p.ends_with('.')
            {
                return None;
            }
            p.parse().ok()?
        } else {
            if !p.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            p.parse().ok()?
        };
        // Minutes and seconds fields must be below 60 when a larger unit precedes them.
        if i > 0 && v >= 60.0 {
            return None;
        }
        total = total * 60.0 + v;
    }
    if total.is_finite() {
        Some(total)
    } else {
        None
    }
}

/// YouTube style clock: `00:00`, `07:05`, `1:02:03`. Fractions are dropped.
pub fn fmt_clock(secs: f64) -> String {
    let t = if secs.is_finite() && secs > 0.0 { secs.floor() as u64 } else { 0 };
    let (h, m, s) = (t / 3600, (t % 3600) / 60, t % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// SRT timestamp: `HH:MM:SS,mmm`.
pub fn fmt_srt(secs: f64) -> String {
    let ms = if secs.is_finite() && secs > 0.0 { (secs * 1000.0).round() as u64 } else { 0 };
    let (h, m, s, f) = (ms / 3_600_000, (ms % 3_600_000) / 60_000, (ms % 60_000) / 1000, ms % 1000);
    format!("{h:02}:{m:02}:{s:02},{f:03}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_forms() {
        assert_eq!(parse_timestamp("00:01:02,500"), Some(62.5));
        assert_eq!(parse_timestamp("00:01:02.500"), Some(62.5));
        assert_eq!(parse_timestamp("01:02.250"), Some(62.25));
        assert_eq!(parse_timestamp("1:02"), Some(62.0));
        assert_eq!(parse_timestamp("1:02:03"), Some(3723.0));
        assert_eq!(parse_timestamp("[0:05]"), Some(5.0));
        assert_eq!(parse_timestamp("(12:00)"), Some(720.0));
        assert_eq!(parse_timestamp("83.5"), Some(83.5));
        assert_eq!(parse_timestamp("75"), Some(75.0));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_timestamp(""), None);
        assert_eq!(parse_timestamp("hello"), None);
        assert_eq!(parse_timestamp("1:75"), None);
        assert_eq!(parse_timestamp("1:2:3:4"), None);
        assert_eq!(parse_timestamp("1.2.3"), None);
        assert_eq!(parse_timestamp("-5"), None);
        assert_eq!(parse_timestamp("1:"), None);
    }

    #[test]
    fn formats() {
        assert_eq!(fmt_clock(0.0), "00:00");
        assert_eq!(fmt_clock(65.9), "01:05");
        assert_eq!(fmt_clock(3723.0), "1:02:03");
        assert_eq!(fmt_srt(62.5), "00:01:02,500");
        assert_eq!(fmt_srt(3723.004), "01:02:03,004");
    }
}
