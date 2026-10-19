//! Shorts picks: clip times that line up with the speech.

use serde::{Deserialize, Serialize};

use crate::time::fmt_clock;
use crate::transcript::Segment;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Short {
    pub start: f64,
    pub end: f64,
    /// The opening line that grabs attention.
    pub hook: String,
    /// Why this works as a Short.
    pub why: String,
}

impl Short {
    pub fn seconds(&self) -> f64 {
        self.end - self.start
    }

    pub fn range(&self) -> String {
        format!("{} to {}", fmt_clock(self.start), fmt_clock(self.end))
    }
}

/// Clip length limits.
#[derive(Debug, Clone, Copy)]
pub struct ClipLimits {
    pub min_secs: f64,
    pub max_secs: f64,
    pub max_picks: usize,
}

impl Default for ClipLimits {
    fn default() -> Self {
        ClipLimits { min_secs: 15.0, max_secs: 60.0, max_picks: 5 }
    }
}

fn clean(s: &str) -> String {
    s.chars().filter(|c| *c != '<' && *c != '>').collect::<String>().trim().trim_matches('"').trim().to_string()
}

/// Make one candidate fit the speech and the length limits. `None` if it cannot.
pub fn fit(c: &Short, segs: &[Segment], duration: f64, lim: ClipLimits) -> Option<Short> {
    if !(c.start.is_finite() && c.end.is_finite()) || c.start < 0.0 || c.start >= duration {
        return None;
    }
    let mut end = if c.end > c.start { c.end.min(duration) } else { (c.start + lim.min_secs).min(duration) };
    // Start at the beginning of the sentence that contains the start time.
    let start = segs
        .iter()
        .filter(|s| s.start <= c.start + 0.5)
        .map(|s| s.start)
        .fold(f64::NAN, f64::max);
    let start = if start.is_finite() && c.start - start <= 8.0 { start } else { c.start };
    // End at the end of the sentence that contains the end time.
    if let Some(e) = segs.iter().filter(|s| s.end >= end - 0.5 && s.start < end).map(|s| s.end).reduce(f64::min) {
        end = e.min(duration);
    }
    if end - start > lim.max_secs {
        let limit = start + lim.max_secs;
        end = segs
            .iter()
            .map(|s| s.end)
            .filter(|&e| e >= start + lim.min_secs && e <= limit)
            .fold(f64::NAN, f64::max);
        if !end.is_finite() {
            end = limit;
        }
    }
    if end - start < lim.min_secs {
        let target = start + lim.min_secs;
        end = segs
            .iter()
            .map(|s| s.end)
            .filter(|&e| e >= target)
            .reduce(f64::min)
            .unwrap_or(duration)
            .min(start + lim.max_secs)
            .min(duration);
    }
    if end - start < lim.min_secs.min(5.0) || end <= start {
        return None;
    }
    let mut hook = clean(&c.hook);
    if hook.is_empty() {
        hook = segs.iter().find(|s| s.start >= start - 0.01).map(|s| s.text.clone()).unwrap_or_default();
    }
    Some(Short { start, end, hook, why: clean(&c.why) })
}

/// Fit every candidate, drop ones that mostly overlap an earlier pick, keep at most `max_picks`.
pub fn pick(candidates: &[Short], segs: &[Segment], duration: f64, lim: ClipLimits) -> Vec<Short> {
    let mut out: Vec<Short> = Vec::new();
    for c in candidates {
        if out.len() >= lim.max_picks {
            break;
        }
        let Some(s) = fit(c, segs, duration, lim) else { continue };
        let clash = out.iter().any(|o| {
            let overlap = (o.end.min(s.end) - o.start.max(s.start)).max(0.0);
            overlap > 0.5 * o.seconds().min(s.seconds())
        });
        if !clash {
            out.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segs() -> Vec<Segment> {
        (0..60)
            .map(|i| Segment { start: i as f64 * 5.0, end: i as f64 * 5.0 + 5.0, text: format!("line {i}") })
            .collect()
    }

    fn s(a: f64, b: f64) -> Short {
        Short { start: a, end: b, hook: "hook".into(), why: "why".into() }
    }

    #[test]
    fn snaps_to_sentences() {
        let f = fit(&s(12.0, 41.0), &segs(), 300.0, ClipLimits::default()).unwrap();
        assert_eq!((f.start, f.end), (10.0, 45.0));
    }

    #[test]
    fn too_long_is_trimmed_too_short_is_extended() {
        let lim = ClipLimits::default();
        let f = fit(&s(0.0, 200.0), &segs(), 300.0, lim).unwrap();
        assert!(f.seconds() <= 60.0 && f.seconds() >= 15.0, "{f:?}");
        assert_eq!(f.end, 60.0);
        let f = fit(&s(100.0, 103.0), &segs(), 300.0, lim).unwrap();
        assert_eq!((f.start, f.end), (100.0, 115.0));
    }

    #[test]
    fn rejects_out_of_range() {
        let lim = ClipLimits::default();
        assert!(fit(&s(400.0, 420.0), &segs(), 300.0, lim).is_none());
        assert!(fit(&s(f64::NAN, 20.0), &segs(), 300.0, lim).is_none());
        assert!(fit(&s(-1.0, 20.0), &segs(), 300.0, lim).is_none());
    }

    #[test]
    fn near_end_of_video() {
        let f = fit(&s(290.0, 299.0), &segs(), 300.0, ClipLimits::default()).unwrap();
        assert_eq!(f.end, 300.0);
        assert!(f.seconds() >= 5.0);
    }

    #[test]
    fn overlaps_are_dropped_and_count_is_capped() {
        let c = [s(10.0, 40.0), s(15.0, 45.0), s(100.0, 130.0), s(150.0, 180.0), s(200.0, 230.0), s(240.0, 270.0), s(50.0, 80.0)];
        let p = pick(&c, &segs(), 300.0, ClipLimits::default());
        assert_eq!(p.len(), 5);
        assert_eq!(p[1].start, 100.0);
    }

    #[test]
    fn empty_hook_uses_the_first_line() {
        let mut c = s(20.0, 50.0);
        c.hook = "  ".into();
        let f = fit(&c, &segs(), 300.0, ClipLimits::default()).unwrap();
        assert_eq!(f.hook, "line 4");
    }
}
