//! Finding filler words ("um", "uh", stutters) in captions.
//!
//! This is deliberately cautious: it only marks words that are almost never
//! meaningful. "you know", "I mean" and "like" only count when commas set them
//! apart, so auto-captions (which have no commas) only get the obvious ones.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::time::fmt_srt;
use crate::transcript::Segment;

/// Always fillers.
const DEFINITE: &[&str] = &[
    "um", "umm", "ummm", "uh", "uhh", "uhhh", "uhm", "er", "erm", "hmm", "hmmm", "mmm",
];

/// Short words that are fillers when said twice in a row ("the the").
const STUTTER: &[&str] = &[
    "i", "the", "a", "an", "and", "to", "it", "it's", "we", "you", "so", "but", "is", "was", "i'm",
    "in", "of", "my", "this", "that's", "he", "she", "they", "like",
];

/// One filler found in the captions.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FillerHit {
    /// Estimated time of the word, in seconds.
    pub time: f64,
    /// The filler as written ("um", "you know", "the" for a stutter).
    pub text: String,
}

/// Counts of filler words.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FillerReport {
    pub total: usize,
    pub per_minute: f64,
    pub counts: BTreeMap<String, usize>,
    pub hits: Vec<FillerHit>,
}

/// Captions with fillers marked and removed.
#[derive(Debug, Clone, Default)]
pub struct Captions {
    /// Fillers wrapped in curly brackets: `so {um} I think`.
    pub marked: Vec<Segment>,
    /// Fillers removed; segments left empty are dropped.
    pub clean: Vec<Segment>,
    pub report: FillerReport,
}

fn core(tok: &str) -> String {
    tok.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
        .to_lowercase()
        .replace('’', "'")
}

/// Which tokens in `toks` are fillers. Returns, per token, the label to count it under.
fn find_fillers(toks: &[&str]) -> Vec<Option<String>> {
    let n = toks.len();
    let cores: Vec<String> = toks.iter().map(|t| core(t)).collect();
    let mut out: Vec<Option<String>> = vec![None; n];
    let ends_comma = |i: usize| toks[i].ends_with(',');
    for i in 0..n {
        if out[i].is_some() {
            continue;
        }
        let c = cores[i].as_str();
        if DEFINITE.contains(&c) {
            out[i] = Some(c.to_string());
            continue;
        }
        // "you know," and "I mean," (comma after the phrase)
        if i + 1 < n && ends_comma(i + 1) && !toks[i].ends_with([',', '.', '?', '!']) {
            let phrase = format!("{} {}", c, cores[i + 1]);
            if phrase == "you know" || phrase == "i mean" {
                out[i] = Some(phrase.clone());
                out[i + 1] = Some(phrase);
                continue;
            }
        }
        // ", like," set apart by commas
        if c == "like" && ends_comma(i) && (i == 0 || ends_comma(i - 1)) {
            out[i] = Some("like".into());
            continue;
        }
        // Stutter: "the the" -> mark the first one.
        if i + 1 < n
            && STUTTER.contains(&c)
            && cores[i + 1] == c
            && !toks[i].ends_with(['.', '?', '!', ','])
        {
            out[i] = Some(format!("{c} (repeat)"));
        }
    }
    out
}

/// Normalize spellings so "umm" and "um" count together.
fn label_for(c: &str) -> String {
    match c {
        x if x.starts_with("um") || x == "uhm" => "um".into(),
        x if x.starts_with("uh") => "uh".into(),
        x if x.starts_with("hm") => "hmm".into(),
        x if x.starts_with("mm") => "mm".into(),
        "er" | "erm" => "er".into(),
        other => other.into(),
    }
}

/// Mark and remove fillers in every segment.
pub fn process(segs: &[Segment], duration: f64) -> Captions {
    let mut caps = Captions::default();
    for s in segs {
        let toks: Vec<&str> = s.text.split_whitespace().collect();
        let found = find_fillers(&toks);
        let mut marked: Vec<String> = Vec::with_capacity(toks.len());
        let mut kept: Vec<String> = Vec::with_capacity(toks.len());
        let mut capitalize_next = false;
        let mut i = 0;
        while i < toks.len() {
            let tok = toks[i];
            match &found[i] {
                None => {
                    let mut w = tok.to_string();
                    if capitalize_next {
                        w = capitalize(&w);
                        capitalize_next = false;
                    }
                    marked.push(tok.to_string());
                    kept.push(w);
                    i += 1;
                }
                Some(label) => {
                    // A two-word phrase is handled as one hit.
                    let span = if label.contains(' ') && !label.ends_with("(repeat)") { 2 } else { 1 };
                    let span = span.min(toks.len() - i);
                    let words: Vec<&str> = toks[i..i + span].to_vec();
                    let last = words[span - 1];
                    let trail: String = last.chars().rev().take_while(|c| !c.is_alphanumeric()).collect::<Vec<_>>().into_iter().rev().collect();
                    let body: Vec<String> = words
                        .iter()
                        .enumerate()
                        .map(|(k, w)| if k == span - 1 { w.trim_end_matches(|c: char| !c.is_alphanumeric()).to_string() } else { w.to_string() })
                        .collect();
                    marked.push(format!("{{{}}}{}", body.join(" "), trail));
                    // Keep sentence-ending punctuation on the previous kept word.
                    if trail.contains(['.', '?', '!']) {
                        if let Some(prev) = kept.last_mut() {
                            if !prev.ends_with(['.', '?', '!']) {
                                let p = prev.trim_end_matches([',', ';', ':']).to_string();
                                *prev = p + &trail.chars().filter(|c| ".?!".contains(*c)).collect::<String>();
                            }
                        }
                        capitalize_next = true;
                    }
                    // A removed first word that was capitalized passes that on.
                    if kept.is_empty() && words[0].chars().next().map(char::is_uppercase).unwrap_or(false) {
                        capitalize_next = true;
                    }
                    let frac = if toks.len() > 1 { i as f64 / toks.len() as f64 } else { 0.0 };
                    let time = s.start + (s.end - s.start).max(0.0) * frac;
                    let key = if label.ends_with("(repeat)") { label.clone() } else { label_for(label) };
                    let shown = if label.ends_with("(repeat)") { core(words[0]) } else { body.join(" ").to_lowercase() };
                    caps.report.hits.push(FillerHit { time, text: shown });
                    *caps.report.counts.entry(key).or_insert(0) += 1;
                    i += span;
                }
            }
        }
        caps.marked.push(Segment { start: s.start, end: s.end, text: marked.join(" ") });
        let clean = kept.join(" ");
        let clean = clean.trim_start_matches([',', ' ']).to_string();
        if clean.chars().any(char::is_alphanumeric) {
            caps.clean.push(Segment { start: s.start, end: s.end, text: clean });
        }
    }
    caps.report.total = caps.report.hits.len();
    caps.report.per_minute = if duration > 0.0 {
        (caps.report.total as f64 / (duration / 60.0) * 10.0).round() / 10.0
    } else {
        0.0
    };
    caps
}

fn capitalize(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Render segments as an SRT file.
pub fn to_srt(segs: &[Segment]) -> String {
    let mut out = String::new();
    for (i, s) in segs.iter().enumerate() {
        let end = if s.end > s.start { s.end } else { s.start + 0.5 };
        out.push_str(&format!("{}\n{} --> {}\n{}\n\n", i + 1, fmt_srt(s.start), fmt_srt(end), s.text));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(t: &str) -> Captions {
        process(&[Segment { start: 0.0, end: 10.0, text: t.into() }], 60.0)
    }

    #[test]
    fn marks_and_removes_um_uh() {
        let c = one("So um I think, uh, we should go.");
        assert_eq!(c.marked[0].text, "So {um} I think, {uh}, we should go.");
        assert_eq!(c.clean[0].text, "So I think, we should go.");
        assert_eq!(c.report.total, 2);
        assert_eq!(c.report.counts["um"], 1);
        assert_eq!(c.report.counts["uh"], 1);
        assert_eq!(c.report.per_minute, 2.0);
    }

    #[test]
    fn leading_filler_passes_on_capital() {
        let c = one("Um, okay let's start");
        assert_eq!(c.clean[0].text, "Okay let's start");
        assert_eq!(c.marked[0].text, "{Um}, okay let's start");
    }

    #[test]
    fn sentence_end_is_kept() {
        let c = one("that was close uh. Anyway");
        assert_eq!(c.clean[0].text, "that was close. Anyway");
    }

    #[test]
    fn phrases_only_with_commas() {
        let c = one("it was, like, huge and you know, fast");
        assert_eq!(c.clean[0].text, "it was, huge and fast");
        assert_eq!(c.report.counts["like"], 1);
        assert_eq!(c.report.counts["you know"], 1);
        let c = one("do you know what I mean by like a castle");
        assert_eq!(c.report.total, 0);
    }

    #[test]
    fn stutters() {
        let c = one("and the the boss died");
        assert_eq!(c.marked[0].text, "and {the} the boss died");
        assert_eq!(c.clean[0].text, "and the boss died");
        let c = one("very very fast");
        assert_eq!(c.report.total, 0);
    }

    #[test]
    fn segment_of_only_fillers_is_dropped_from_clean() {
        let c = process(
            &[
                Segment { start: 0.0, end: 1.0, text: "um".into() },
                Segment { start: 1.0, end: 2.0, text: "hello".into() },
            ],
            2.0,
        );
        assert_eq!(c.marked.len(), 2);
        assert_eq!(c.clean.len(), 1);
        assert_eq!(c.clean[0].text, "hello");
    }

    #[test]
    fn umbrella_is_not_um() {
        let c = one("grab the umbrella and the hummus");
        assert_eq!(c.report.total, 0);
    }

    #[test]
    fn srt_output() {
        let s = to_srt(&[Segment { start: 1.0, end: 2.5, text: "hi".into() }]);
        assert_eq!(s, "1\n00:00:01,000 --> 00:00:02,500\nhi\n\n");
    }
}
