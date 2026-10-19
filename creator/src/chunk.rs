//! Splitting long transcripts into pieces that fit the model's context.

use crate::time::fmt_clock;
use crate::transcript::Segment;

/// Rough token count. Errs high for English (about 3.5 characters per token)
/// and counts each non-ASCII character as one token.
pub fn estimate_tokens(s: &str) -> usize {
    let (mut ascii, mut other) = (0usize, 0usize);
    for c in s.chars() {
        if c.is_ascii() {
            ascii += 1;
        } else {
            other += 1;
        }
    }
    (ascii * 2).div_ceil(7) + other
}

/// A transcript line as the model sees it: `[mm:ss] text`.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

impl Line {
    pub fn render(&self) -> String {
        format!("[{}] {}", fmt_clock(self.start), self.text)
    }
}

/// Join short caption segments into lines of up to `max_secs` seconds or
/// `max_chars` characters, so each timestamp costs fewer tokens.
pub fn merge_lines(segs: &[Segment], max_secs: f64, max_chars: usize) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    for s in segs {
        if let Some(last) = out.last_mut() {
            let fits = s.end - last.start <= max_secs
                && last.text.len() + 1 + s.text.len() <= max_chars
                && s.start - last.end < 2.0;
            if fits {
                last.text.push(' ');
                last.text.push_str(&s.text);
                last.end = last.end.max(s.end);
                continue;
            }
        }
        out.push(Line { start: s.start, end: s.end, text: s.text.clone() });
    }
    out
}

/// A piece of the transcript sized for one model request.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// 0-based position.
    pub index: usize,
    pub start: f64,
    pub end: f64,
    /// Rendered lines, one per line of text.
    pub text: String,
}

/// Split lines into chunks of at most `budget_tokens` (estimated). A single line
/// longer than the budget gets a chunk of its own.
pub fn make_chunks(lines: &[Line], budget_tokens: usize) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut text = String::new();
    let mut tokens = 0usize;
    let mut start = 0.0;
    let mut end = 0.0;
    for l in lines {
        let r = l.render();
        let t = estimate_tokens(&r) + 1;
        if !text.is_empty() && tokens + t > budget_tokens {
            chunks.push(Chunk { index: chunks.len(), start, end, text: std::mem::take(&mut text) });
            tokens = 0;
        }
        if text.is_empty() {
            start = l.start;
        }
        text.push_str(&r);
        text.push('\n');
        tokens += t;
        end = l.end;
    }
    if !text.is_empty() {
        chunks.push(Chunk { index: chunks.len(), start, end, text });
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(a: f64, b: f64, t: &str) -> Segment {
        Segment { start: a, end: b, text: t.into() }
    }

    #[test]
    fn token_estimate_is_conservative() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcdefg"), 2);
        assert_eq!(estimate_tokens("héllo"), 3);
    }

    #[test]
    fn merges_short_segments() {
        let segs = [seg(0.0, 2.0, "a"), seg(2.0, 4.0, "b"), seg(4.0, 12.0, "c"), seg(20.0, 21.0, "d")];
        let lines = merge_lines(&segs, 10.0, 200);
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["a b", "c", "d"]);
        assert_eq!(lines[0].render(), "[00:00] a b");
    }

    #[test]
    fn chunks_respect_budget_and_cover_everything() {
        let segs: Vec<Segment> = (0..200)
            .map(|i| seg(i as f64 * 5.0, i as f64 * 5.0 + 5.0, "some words that people said here"))
            .collect();
        let lines = merge_lines(&segs, 4.0, 100);
        let chunks = make_chunks(&lines, 300);
        assert!(chunks.len() > 3);
        for c in &chunks {
            assert!(estimate_tokens(&c.text) <= 300 + c.text.lines().count());
        }
        let total: usize = chunks.iter().map(|c| c.text.lines().count()).sum();
        assert_eq!(total, lines.len());
        assert_eq!(chunks[0].start, 0.0);
        assert_eq!(chunks.last().unwrap().end, 1000.0);
        for w in chunks.windows(2) {
            assert!(w[1].start >= w[0].end);
        }
    }

    #[test]
    fn one_chunk_when_it_fits() {
        let lines = merge_lines(&[seg(0.0, 3.0, "short video")], 10.0, 100);
        assert_eq!(make_chunks(&lines, 4000).len(), 1);
    }
}
