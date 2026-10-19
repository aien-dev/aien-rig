//! Small UTC time helpers so receipts carry readable timestamps without a
//! date library. Pure logic.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Days since 1970-01-01 to (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn parts(unix: i64) -> (i64, u32, u32, i64, i64, i64) {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    (y, m, d, secs / 3600, (secs % 3600) / 60, secs % 60)
}

/// RFC 3339 UTC, e.g. 2026-10-19T17:03:05Z
pub fn rfc3339(unix: i64) -> String {
    let (y, m, d, hh, mm, ss) = parts(unix);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// File-name friendly UTC stamp, e.g. 20261019-170305
pub fn file_stamp(unix: i64) -> String {
    let (y, m, d, hh, mm, ss) = parts(unix);
    format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        // 2026-10-19T17:03:05Z
        assert_eq!(rfc3339(1_792_429_385), "2026-10-19T17:03:05Z");
        assert_eq!(file_stamp(1_792_429_385), "20261019-170305");
        // leap day 2024-02-29T12:00:00Z
        assert_eq!(rfc3339(1_709_208_000), "2024-02-29T12:00:00Z");
    }
}
