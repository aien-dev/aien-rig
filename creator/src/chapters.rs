//! YouTube chapter rules: checking and repairing.
//!
//! YouTube only shows chapters when the description lists timestamps that:
//! 1. start with `00:00`,
//! 2. number at least three,
//! 3. go up in order, and
//! 4. are each at least 10 seconds long.
//!
//! The model's suggestions are treated as candidates; [`repair`] turns them
//! into a list that always passes [`check`] when the video is long enough.

use serde::{Deserialize, Serialize};

use crate::time::fmt_clock;

/// YouTube's shortest allowed chapter, in seconds.
pub const MIN_CHAPTER_SECS: f64 = 10.0;
/// YouTube's fewest allowed chapters.
pub const MIN_CHAPTERS: usize = 3;
/// Longest chapter title we keep.
pub const MAX_TITLE_CHARS: usize = 80;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    /// Start in seconds.
    pub start: f64,
    pub title: String,
}

impl Chapter {
    pub fn new(start: f64, title: &str) -> Chapter {
        Chapter { start, title: title.to_string() }
    }

    /// `00:00 Title`
    pub fn line(&self) -> String {
        format!("{} {}", fmt_clock(self.start), self.title)
    }
}

/// List every rule the chapters break. Empty means YouTube will accept them.
pub fn check(chapters: &[Chapter], duration: f64) -> Vec<String> {
    let mut problems = Vec::new();
    if chapters.len() < MIN_CHAPTERS {
        problems.push(format!("needs at least {MIN_CHAPTERS} chapters, has {}", chapters.len()));
    }
    if let Some(first) = chapters.first() {
        if fmt_clock(first.start) != "00:00" {
            problems.push(format!("first chapter must start at 00:00, starts at {}", fmt_clock(first.start)));
        }
    }
    for (i, c) in chapters.iter().enumerate() {
        if c.title.trim().is_empty() {
            problems.push(format!("chapter {} has no title", i + 1));
        }
        // YouTube reads whole seconds, so check the times as written.
        let start = c.start.floor();
        let next = chapters.get(i + 1).map(|n| n.start.floor()).unwrap_or(duration);
        if next <= start && i + 1 < chapters.len() {
            problems.push(format!("chapter {} at {} is not after the one before it", i + 2, fmt_clock(next)));
        } else if next - start < MIN_CHAPTER_SECS {
            problems.push(format!(
                "chapter {} at {} is shorter than {MIN_CHAPTER_SECS} seconds",
                i + 1,
                fmt_clock(start)
            ));
        }
    }
    problems
}

/// Tidy a chapter title: no timestamp prefix, no angle brackets (YouTube rejects them), length cap.
pub fn clean_title(t: &str) -> String {
    let mut s = t.trim();
    // Drop a leading "00:00" or "1." the model may have added.
    if let Some((first, rest)) = s.split_once(char::is_whitespace) {
        let f = first.trim_end_matches(['-', '–', ':', '.', ')']);
        if crate::time::parse_timestamp(f).is_some() {
            s = rest.trim_start_matches(['-', '–', '—', ':', ' ']);
        }
    }
    let s = crate::transcript::clean_text(s);
    let s: String = s.chars().filter(|c| *c != '<' && *c != '>').collect();
    let s = s.trim().trim_matches('"').trim();
    truncate_words(s, MAX_TITLE_CHARS)
}

/// Cut at a word boundary so the result is at most `max` characters.
pub fn truncate_words(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out = String::new();
    for w in s.split_whitespace() {
        let add = if out.is_empty() { w.chars().count() } else { w.chars().count() + 1 };
        if out.chars().count() + add > max {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    if out.is_empty() {
        s.chars().take(max).collect()
    } else {
        out
    }
}

/// Turn candidate chapters into a list YouTube accepts.
///
/// `snap_points` are segment start times: chapter starts move to the nearest
/// one within 3 seconds so chapters begin at a sentence. Returns fewer than
/// three chapters only when the candidates (or the video) are too short.
pub fn repair(candidates: &[Chapter], duration: f64, snap_points: &[f64]) -> Vec<Chapter> {
    let mut c: Vec<Chapter> = candidates
        .iter()
        .filter(|c| c.start.is_finite() && c.start >= 0.0)
        .map(|c| Chapter { start: snap(c.start, snap_points), title: clean_title(&c.title) })
        .filter(|c| !c.title.is_empty() && c.start.floor() <= duration - MIN_CHAPTER_SECS)
        .collect();
    c.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    // Whole seconds, as YouTube reads them.
    for ch in c.iter_mut() {
        ch.start = ch.start.floor();
    }
    if c.is_empty() {
        return c;
    }
    // Rule 1: first chapter at 00:00. Pull an early first chapter back to 0,
    // otherwise add an intro chapter.
    if c[0].start > 0.0 {
        if c[0].start <= 15.0 {
            c[0].start = 0.0;
        } else {
            c.insert(0, Chapter::new(0.0, "Intro"));
        }
    }
    // Rules 3 and 4: keep a chapter only if it starts 10+ s after the last kept one.
    let mut kept: Vec<Chapter> = Vec::with_capacity(c.len());
    for ch in c {
        match kept.last() {
            Some(last) if ch.start - last.start < MIN_CHAPTER_SECS => {}
            _ => kept.push(ch),
        }
    }
    // The last chapter must also run 10+ s before the video ends.
    while kept.len() > 1 && duration.floor() - kept.last().map(|c| c.start).unwrap_or(0.0) < MIN_CHAPTER_SECS {
        kept.pop();
    }
    // Neighbouring chapters with the same title read as one.
    kept.dedup_by(|b, a| a.title.eq_ignore_ascii_case(&b.title));
    kept
}

/// Evenly spaced chapters for when the model gave too few usable ones.
/// Titles come from `titles_at` (the nearest topic title for a time), or "Part N".
pub fn even_split(duration: f64, titles_at: &dyn Fn(f64) -> Option<String>) -> Vec<Chapter> {
    if duration < MIN_CHAPTER_SECS * MIN_CHAPTERS as f64 {
        return Vec::new();
    }
    let n = ((duration / 180.0).round() as usize).clamp(MIN_CHAPTERS, 8);
    let step = (duration / n as f64).floor();
    (0..n)
        .map(|i| {
            let start = if i == 0 { 0.0 } else { step * i as f64 };
            let title = titles_at(start)
                .map(|t| clean_title(&t))
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| format!("Part {}", i + 1));
            Chapter { start, title }
        })
        .collect()
}

fn snap(t: f64, points: &[f64]) -> f64 {
    let mut best = t;
    let mut best_d = 3.0;
    for &p in points {
        let d = (p - t).abs();
        if d <= best_d {
            best_d = d;
            best = p;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(s: f64, t: &str) -> Chapter {
        Chapter::new(s, t)
    }

    #[test]
    fn valid_list_passes() {
        let c = [ch(0.0, "Intro"), ch(30.0, "Build"), ch(90.0, "Fight")];
        assert!(check(&c, 120.0).is_empty());
    }

    #[test]
    fn each_rule_is_reported() {
        let p = check(&[ch(0.0, "a"), ch(30.0, "b")], 100.0);
        assert!(p.iter().any(|p| p.contains("at least 3")), "{p:?}");

        let p = check(&[ch(5.0, "a"), ch(30.0, "b"), ch(60.0, "c")], 100.0);
        assert!(p.iter().any(|p| p.contains("00:00")), "{p:?}");

        let p = check(&[ch(0.0, "a"), ch(60.0, "b"), ch(30.0, "c")], 100.0);
        assert!(p.iter().any(|p| p.contains("not after")), "{p:?}");

        let p = check(&[ch(0.0, "a"), ch(5.0, "b"), ch(30.0, "c")], 100.0);
        assert!(p.iter().any(|p| p.contains("shorter than 10")), "{p:?}");

        // The last chapter needs 10 s before the end too.
        let p = check(&[ch(0.0, "a"), ch(30.0, "b"), ch(95.0, "c")], 100.0);
        assert!(p.iter().any(|p| p.contains("shorter than 10")), "{p:?}");

        let p = check(&[ch(0.0, "a"), ch(30.0, " "), ch(60.0, "c")], 100.0);
        assert!(p.iter().any(|p| p.contains("no title")), "{p:?}");
    }

    #[test]
    fn exactly_ten_seconds_is_allowed() {
        assert!(check(&[ch(0.0, "a"), ch(10.0, "b"), ch(20.0, "c")], 30.0).is_empty());
        assert!(!check(&[ch(0.0, "a"), ch(10.0, "b"), ch(20.0, "c")], 29.9).is_empty());
    }

    #[test]
    fn repair_fixes_order_start_and_gaps() {
        let raw = [
            ch(130.0, "Boss fight"),
            ch(4.0, "Welcome"),
            ch(60.0, "Getting gear"),
            ch(65.0, "Too close"),
            ch(595.0, "Too near the end"),
            ch(-3.0, "Negative"),
            ch(f64::NAN, "Nan"),
        ];
        let fixed = repair(&raw, 600.0, &[]);
        let titles: Vec<&str> = fixed.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Welcome", "Getting gear", "Boss fight"]);
        assert_eq!(fixed[0].start, 0.0);
        assert!(check(&fixed, 600.0).is_empty());
    }

    #[test]
    fn repair_adds_intro_when_first_is_late() {
        let fixed = repair(&[ch(40.0, "Part A"), ch(120.0, "Part B")], 300.0, &[]);
        assert_eq!(fixed[0], ch(0.0, "Intro"));
        assert!(check(&fixed, 300.0).is_empty());
    }

    #[test]
    fn repair_snaps_to_sentence_starts_and_cleans_titles() {
        let fixed = repair(
            &[ch(0.0, "00:00 - Intro"), ch(31.5, "<b>Build</b> time"), ch(88.0, "\"Fight\"")],
            200.0,
            &[0.0, 29.0, 90.2],
        );
        assert_eq!(fixed[1].start, 29.0);
        assert_eq!(fixed[1].title, "Build time");
        assert_eq!(fixed[2].start, 90.0);
        assert_eq!(fixed[0].title, "Intro");
        assert_eq!(fixed[2].title, "Fight");
    }

    #[test]
    fn even_split_always_valid_when_long_enough() {
        for d in [30.0, 31.0, 95.0, 600.0, 7200.0] {
            let c = even_split(d, &|_| None);
            assert!(check(&c, d).is_empty(), "duration {d}: {:?}", check(&c, d));
        }
        assert!(even_split(29.0, &|_| None).is_empty());
    }

    #[test]
    fn line_format() {
        assert_eq!(ch(0.0, "Intro").line(), "00:00 Intro");
        assert_eq!(ch(3725.0, "Late").line(), "1:02:05 Late");
    }
}
