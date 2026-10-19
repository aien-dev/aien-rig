//! Reading transcripts and caption files.
//!
//! Supported inputs:
//! - SubRip `.srt`
//! - WebVTT `.vtt`, including YouTube auto-captions (rolling two-line cues)
//! - YouTube `json3` caption files (what `yt-dlp --sub-format json3` saves)
//! - The Rig speech engine JSON (see `TRANSCRIPT.md`), Whisper-style JSON,
//!   or a bare JSON array of `{start, end, text}`
//! - Text copied from YouTube's "Show transcript" panel, or any text whose
//!   lines start with a timestamp (`0:00 hello`, `[01:02] hello`)
//! - Plain text with no timestamps (times are then estimated from word count)

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::time::parse_timestamp;

/// Speaking rate used to estimate times when the input has none (words per second).
pub const WORDS_PER_SECOND: f64 = 2.6;

/// One timed piece of speech.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    /// Start, in seconds from the beginning of the video.
    pub start: f64,
    /// End, in seconds from the beginning of the video.
    pub end: f64,
    pub text: String,
}

/// The input formats this crate understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    Srt,
    Vtt,
    YoutubeJson3,
    RigJson,
    TimedText,
    PlainText,
}

impl Format {
    /// Parse a user-supplied format name (`srt`, `vtt`, `json3`, `json`, `timed-text`, `text`).
    pub fn from_name(name: &str) -> Option<Format> {
        match name.trim().to_ascii_lowercase().as_str() {
            "srt" => Some(Format::Srt),
            "vtt" | "webvtt" => Some(Format::Vtt),
            "json3" | "youtube-json3" => Some(Format::YoutubeJson3),
            "json" | "rig-json" | "whisper" => Some(Format::RigJson),
            "timed-text" | "youtube-text" | "transcript" => Some(Format::TimedText),
            "text" | "txt" | "plain" | "plain-text" => Some(Format::PlainText),
            _ => None,
        }
    }
}

/// A parsed transcript, ordered by start time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub format: Format,
    /// True when the input had no timestamps and times were estimated.
    pub timing_estimated: bool,
    pub language: Option<String>,
    /// Video length if the input stated it (speech JSON `duration`).
    pub declared_duration: Option<f64>,
    pub segments: Vec<Segment>,
}

impl Transcript {
    /// Video length in seconds: the stated duration, or the end of the last segment.
    pub fn duration(&self) -> f64 {
        let last = self.segments.iter().map(|s| s.end).fold(0.0, f64::max);
        match self.declared_duration {
            Some(d) if d.is_finite() && d > last => d,
            _ => last,
        }
    }

    pub fn word_count(&self) -> usize {
        self.segments.iter().map(|s| s.text.split_whitespace().count()).sum()
    }
}

/// Read and parse a transcript file. `format` overrides detection.
pub fn parse_file(path: &Path, format: Option<Format>) -> Result<Transcript> {
    let bytes = std::fs::read(path)
        .map_err(|e| Error::Input(format!("cannot read {}: {e}", path.display())))?;
    let text = String::from_utf8_lossy(&bytes);
    let name = path.file_name().and_then(|n| n.to_str());
    let fmt = format.unwrap_or_else(|| detect(name, &text));
    parse_str(&text, fmt)
}

/// Guess the format from the file name and the content.
pub fn detect(file_name: Option<&str>, text: &str) -> Format {
    let text = strip_bom(text);
    let ext = file_name
        .and_then(|n| n.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()))
        .unwrap_or_default();
    let head = text.trim_start();
    match ext.as_str() {
        "srt" => return Format::Srt,
        "vtt" => return Format::Vtt,
        "json3" => return Format::YoutubeJson3,
        _ => {}
    }
    if head.starts_with("WEBVTT") {
        return Format::Vtt;
    }
    if head.starts_with('{') || head.starts_with('[') {
        if let Ok(v) = serde_json::from_str::<Value>(text) {
            if v.get("events").map(Value::is_array).unwrap_or(false) {
                return Format::YoutubeJson3;
            }
            return Format::RigJson;
        }
    }
    // "[00:00.000 --> 00:03.000] text" is Whisper's text output, not SRT.
    if text.lines().any(|l| {
        l.contains("-->") && !l.trim_start().starts_with('[') && parse_cue_time_line(l).is_some()
    }) {
        return Format::Srt;
    }
    if looks_timed(text) {
        Format::TimedText
    } else {
        Format::PlainText
    }
}

fn looks_timed(text: &str) -> bool {
    text.lines().filter(|l| leading_timestamp(l.trim()).is_some()).take(2).count() >= 2
}

/// Parse text in the given format.
pub fn parse_str(text: &str, format: Format) -> Result<Transcript> {
    let text = strip_bom(text);
    let mut t = Transcript {
        format,
        timing_estimated: false,
        language: None,
        declared_duration: None,
        segments: Vec::new(),
    };
    t.segments = match format {
        Format::Srt | Format::Vtt => cues_to_segments(parse_cues(text)),
        Format::YoutubeJson3 => parse_json3(text)?,
        Format::RigJson => {
            let (segs, lang, dur) = parse_rig_json(text)?;
            t.language = lang;
            t.declared_duration = dur;
            segs
        }
        Format::TimedText => parse_timed_text(text).ok_or_else(|| {
            Error::Input("expected lines starting with a timestamp such as 0:00".into())
        })?,
        Format::PlainText => {
            // Text copied from YouTube's transcript panel still works if saved as .txt.
            if let Some(segs) = parse_timed_text(text) {
                t.format = Format::TimedText;
                segs
            } else {
                t.timing_estimated = true;
                estimate_prose(text)
            }
        }
    };
    normalize(&mut t.segments);
    if t.segments.is_empty() {
        return Err(Error::Input(format!(
            "no caption text found (read it as {format:?}); is this the right file?"
        )));
    }
    Ok(t)
}

fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

// ---------- SRT and WebVTT ----------

type Cue = (f64, f64, Vec<String>);

fn parse_cue_time_line(line: &str) -> Option<(f64, f64)> {
    let (l, r) = line.split_once("-->")?;
    let a = parse_timestamp(l.trim())?;
    let b = parse_timestamp(r.split_whitespace().next()?)?;
    Some((a, b))
}

fn parse_cues(text: &str) -> Vec<Cue> {
    let lines: Vec<&str> = text.lines().collect();
    let mut cues = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some((a, b)) = parse_cue_time_line(lines[i]) {
            i += 1;
            let mut body: Vec<String> = Vec::new();
            while i < lines.len() && !lines[i].trim().is_empty() {
                if parse_cue_time_line(lines[i]).is_some() {
                    // Missing blank line: the previous line was the next cue's number.
                    if body
                        .last()
                        .map(|l| !l.trim().is_empty() && l.trim().chars().all(|c| c.is_ascii_digit()))
                        .unwrap_or(false)
                    {
                        body.pop();
                    }
                    break;
                }
                body.push(lines[i].to_string());
                i += 1;
            }
            cues.push((a, b, body));
        } else {
            i += 1;
        }
    }
    cues
}

/// Turn cues into segments. YouTube auto-captions repeat the previous line at the
/// top of each cue ("rolling" captions); a line already shown in the previous cue
/// is not repeated.
fn cues_to_segments(cues: Vec<Cue>) -> Vec<Segment> {
    let mut prev: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for (a, b, body) in cues {
        let cleaned: Vec<String> =
            body.iter().map(|l| clean_text(l)).filter(|l| !l.is_empty()).collect();
        let new: Vec<&str> =
            cleaned.iter().filter(|l| !prev.contains(l)).map(|s| s.as_str()).collect();
        if !new.is_empty() {
            out.push(Segment { start: a, end: b, text: new.join(" ") });
        }
        prev = cleaned;
    }
    out
}

// ---------- YouTube json3 ----------

fn parse_json3(text: &str) -> Result<Vec<Segment>> {
    let v: Value = serde_json::from_str(text)
        .map_err(|e| Error::Input(format!("not valid JSON: {e}")))?;
    let events = v
        .get("events")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Input("json3 file has no \"events\" list".into()))?;
    let mut out = Vec::new();
    for ev in events {
        let Some(segs) = ev.get("segs").and_then(Value::as_array) else { continue };
        let raw: String = segs.iter().filter_map(|s| s.get("utf8").and_then(Value::as_str)).collect();
        let text = clean_text(&raw.replace('\n', " "));
        if text.is_empty() {
            continue;
        }
        let start = ev.get("tStartMs").and_then(Value::as_f64).unwrap_or(0.0) / 1000.0;
        let dur = ev.get("dDurationMs").and_then(Value::as_f64).unwrap_or(0.0) / 1000.0;
        out.push(Segment { start, end: start + dur, text });
    }
    Ok(out)
}

// ---------- Rig speech JSON / Whisper JSON / bare array ----------

fn num_or_time(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => parse_timestamp(s),
        _ => None,
    }
}

type RigParts = (Vec<Segment>, Option<String>, Option<f64>);

fn parse_rig_json(text: &str) -> Result<RigParts> {
    let v: Value = serde_json::from_str(text)
        .map_err(|e| Error::Input(format!("not valid JSON: {e}")))?;
    let (arr, lang, dur) = match &v {
        Value::Array(a) => (a.clone(), None, None),
        Value::Object(o) => {
            let arr = o
                .get("segments")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| Error::Input("JSON transcript has no \"segments\" list".into()))?;
            let lang = o.get("language").and_then(Value::as_str).map(str::to_string);
            let dur = num_or_time(o.get("duration"));
            (arr, lang, dur)
        }
        _ => return Err(Error::Input("JSON transcript must be an object or a list".into())),
    };
    let mut out = Vec::new();
    for (i, s) in arr.iter().enumerate() {
        let text = match s.get("text").and_then(Value::as_str) {
            Some(t) => t.to_string(),
            None => s
                .get("words")
                .and_then(Value::as_array)
                .map(|w| {
                    w.iter()
                        .filter_map(|w| w.get("word").or_else(|| w.get("text")).and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default(),
        };
        let start = num_or_time(s.get("start"))
            .ok_or_else(|| Error::Input(format!("segment {} has no \"start\" time", i + 1)))?;
        let end = num_or_time(s.get("end")).unwrap_or(f64::NAN);
        out.push(Segment { start, end, text: clean_text(&text) });
    }
    fill_missing_ends(&mut out);
    Ok((out, lang, dur))
}

// ---------- Timed text (YouTube transcript panel, "0:00 text" lines) ----------

/// If the line starts with a timestamp (`0:00`, `[01:02]`, `(1:02:03)`), return
/// the time and the rest of the line.
pub(crate) fn leading_timestamp(line: &str) -> Option<(f64, &str)> {
    let first = line.split_whitespace().next()?;
    let tok = first
        .trim_end_matches(['-', '–', '—', ':'])
        .trim_start_matches(['[', '('])
        .trim_end_matches([']', ')']);
    if !tok.contains(':') || !tok.chars().next()?.is_ascii_digit() {
        return None;
    }
    let t = parse_timestamp(tok)?;
    let mut rest = line[first.len()..].trim_start();
    // Whisper CLI style: "[00:00.000 --> 00:04.000]  text"
    if let Some(r) = rest.strip_prefix("-->") {
        let r = r.trim_start();
        rest = match r.find(']') {
            Some(p) => &r[p + 1..],
            None => r.split_once(char::is_whitespace).map(|x| x.1).unwrap_or(""),
        };
    }
    let rest = rest.trim_start_matches(['-', '–', '—', ':', ' ', '\t']).trim();
    Some((t, rest))
}

fn parse_timed_text(text: &str) -> Option<Vec<Segment>> {
    let mut segs: Vec<Segment> = Vec::new();
    let mut before_first = String::new();
    let mut stamps = 0;
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if let Some((t, rest)) = leading_timestamp(l) {
            stamps += 1;
            segs.push(Segment { start: t, end: f64::NAN, text: rest.to_string() });
        } else if let Some(last) = segs.last_mut() {
            if !last.text.is_empty() {
                last.text.push(' ');
            }
            last.text.push_str(l);
        } else {
            before_first.push_str(l);
            before_first.push(' ');
        }
    }
    if stamps < 2 {
        return None;
    }
    if !before_first.trim().is_empty() {
        let first = &mut segs[0];
        first.text = format!("{} {}", before_first.trim(), first.text);
    }
    for s in segs.iter_mut() {
        s.text = clean_text(&s.text);
    }
    fill_missing_ends(&mut segs);
    Some(segs)
}

// ---------- Plain prose (no timestamps) ----------

fn estimate_prose(text: &str) -> Vec<Segment> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut segs = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut t = 0.0;
    let flush = |cur: &mut Vec<&str>, t: &mut f64, segs: &mut Vec<Segment>| {
        if cur.is_empty() {
            return;
        }
        let dur = (cur.len() as f64 / WORDS_PER_SECOND).max(1.0);
        segs.push(Segment { start: *t, end: *t + dur, text: cur.join(" ") });
        *t += dur;
        cur.clear();
    };
    for w in words {
        cur.push(w);
        let ends_sentence = w.ends_with(['.', '?', '!']);
        if (ends_sentence && cur.len() >= 4) || cur.len() >= 25 {
            flush(&mut cur, &mut t, &mut segs);
        }
    }
    flush(&mut cur, &mut t, &mut segs);
    for s in segs.iter_mut() {
        s.text = clean_text(&s.text);
    }
    segs
}

// ---------- shared clean-up ----------

/// Give segments without an end time the next segment's start (or an estimate).
fn fill_missing_ends(segs: &mut [Segment]) {
    for i in 0..segs.len() {
        if segs[i].end.is_finite() && segs[i].end > segs[i].start {
            continue;
        }
        let est = segs[i].start
            + (segs[i].text.split_whitespace().count() as f64 / WORDS_PER_SECOND).max(2.0);
        segs[i].end = match segs.get(i + 1) {
            // Speech is continuous: run up to the next segment unless there is a long gap.
            Some(n) if n.start > segs[i].start && n.start - segs[i].start <= 30.0 => n.start,
            Some(n) if n.start > segs[i].start => est.min(n.start),
            _ => est,
        };
    }
}

fn normalize(segs: &mut Vec<Segment>) {
    segs.retain(|s| s.start.is_finite() && s.start >= 0.0 && !s.text.trim().is_empty());
    for s in segs.iter_mut() {
        if !s.end.is_finite() || s.end < s.start {
            s.end = s.start;
        }
        s.text = s.text.trim().to_string();
    }
    segs.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
}

/// Strip markup (`<i>`, `<c>`, inline `<00:00:01.000>` times, `{\an8}`), decode
/// HTML entities and collapse whitespace.
pub fn clean_text(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '<' {
            if let Some(n) = chars.get(i + 1) {
                if n.is_ascii_alphabetic() || *n == '/' || n.is_ascii_digit() {
                    if let Some(j) = (i + 1..chars.len().min(i + 100)).find(|&j| chars[j] == '>') {
                        i = j + 1;
                        continue;
                    }
                }
            }
        }
        if c == '{' && chars.get(i + 1) == Some(&'\\') {
            if let Some(j) = (i + 1..chars.len()).find(|&j| chars[j] == '}') {
                i = j + 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    let out = decode_entities(&out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        let tail = &rest[p..];
        let semi = tail[..tail.len().min(12)].find(';');
        let mut done = false;
        if let Some(e) = semi {
            let name = &tail[1..e];
            let rep: Option<char> = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                _ => name
                    .strip_prefix("#x")
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            };
            if let Some(c) = rep {
                out.push(c);
                rest = &tail[e + 1..];
                done = true;
            }
        }
        if !done {
            out.push('&');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRT: &str = "1\n00:00:00,000 --> 00:00:02,500\nHey everyone, <i>welcome</i> back.\n\n2\n00:00:02,500 --> 00:00:05,000\nToday we&#39;re building\na castle.\n\n";

    #[test]
    fn srt_basic() {
        let t = parse_str(SRT, Format::Srt).unwrap();
        assert_eq!(t.segments.len(), 2);
        assert_eq!(t.segments[0].text, "Hey everyone, welcome back.");
        assert_eq!(t.segments[1].text, "Today we're building a castle.");
        assert_eq!(t.segments[1].start, 2.5);
        assert_eq!(t.duration(), 5.0);
    }

    #[test]
    fn srt_crlf_bom_and_missing_blank_line() {
        let s = "\u{feff}1\r\n00:00:01,000 --> 00:00:02,000\r\nfirst\r\n2\r\n00:00:02,000 --> 00:00:03,000\r\nsecond\r\n";
        assert_eq!(detect(None, s), Format::Srt);
        let t = parse_str(s, Format::Srt).unwrap();
        let texts: Vec<&str> = t.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["first", "second"]);
    }

    #[test]
    fn vtt_youtube_rolling_captions_are_not_doubled() {
        let v = "WEBVTT\nKind: captions\nLanguage: en\n\n00:00:00.320 --> 00:00:02.990 align:start position:0%\n \nhey<00:00:00.640><c> guys</c><00:00:00.960><c> welcome</c>\n\n00:00:02.990 --> 00:00:03.000 align:start position:0%\nhey guys welcome\n \n\n00:00:03.000 --> 00:00:05.110 align:start position:0%\nhey guys welcome\nback<00:00:03.360><c> to</c><00:00:03.520><c> the</c><00:00:03.680><c> channel</c>\n\n00:00:05.110 --> 00:00:05.120 align:start position:0%\nback to the channel\n \n";
        assert_eq!(detect(Some("a.en.vtt"), v), Format::Vtt);
        let t = parse_str(v, Format::Vtt).unwrap();
        let texts: Vec<&str> = t.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["hey guys welcome", "back to the channel"]);
        assert_eq!(t.segments[1].start, 3.0);
    }

    #[test]
    fn vtt_skips_notes_and_voice_tags() {
        let v = "WEBVTT\n\nNOTE made by hand\n\nintro\n00:01.000 --> 00:04.000\n<v Drake>Let's go &amp; win\n";
        let t = parse_str(v, Format::Vtt).unwrap();
        assert_eq!(t.segments.len(), 1);
        assert_eq!(t.segments[0].text, "Let's go & win");
        assert_eq!(t.segments[0].start, 1.0);
    }

    #[test]
    fn json3() {
        let j = r#"{"wireMagic":"pb3","events":[{"tStartMs":0,"dDurationMs":4000,"id":1},{"tStartMs":120,"dDurationMs":2000,"segs":[{"utf8":"what's"},{"utf8":" up","tOffsetMs":200}]},{"tStartMs":2100,"aAppend":1,"segs":[{"utf8":"\n"}]},{"tStartMs":2200,"dDurationMs":1800,"segs":[{"utf8":"everybody"}]}]}"#;
        assert_eq!(detect(Some("x.json"), j), Format::YoutubeJson3);
        let t = parse_str(j, Format::YoutubeJson3).unwrap();
        let texts: Vec<&str> = t.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["what's up", "everybody"]);
        assert!((t.segments[0].start - 0.12).abs() < 1e-9);
        assert!((t.segments[1].end - 4.0).abs() < 1e-9);
    }

    #[test]
    fn rig_json_schema() {
        let j = r#"{"schema":"aien-rig-transcript/1","language":"en","duration":30.5,
            "segments":[{"start":0.0,"end":3.2,"text":" Hello there. "},
                        {"start":3.2,"end":6.0,"text":"Second part.","words":[]}]}"#;
        assert_eq!(detect(Some("talk.json"), j), Format::RigJson);
        let t = parse_str(j, Format::RigJson).unwrap();
        assert_eq!(t.language.as_deref(), Some("en"));
        assert_eq!(t.segments[0].text, "Hello there.");
        assert_eq!(t.duration(), 30.5);
    }

    #[test]
    fn rig_json_words_only_and_bare_array() {
        let j = r#"[{"start":"0:01","end":2,"words":[{"word":"just"},{"word":"words"}]}]"#;
        let t = parse_str(j, Format::RigJson).unwrap();
        assert_eq!(t.segments[0].text, "just words");
        assert_eq!(t.segments[0].start, 1.0);
        let bad = r#"{"segments":[{"text":"no start"}]}"#;
        assert!(parse_str(bad, Format::RigJson).is_err());
    }

    #[test]
    fn youtube_transcript_panel_copy() {
        let p = "Transcript\n0:00\nhey guys welcome back\n0:04\ntoday we are\ngoing to build\n1:02:03\nthe end\n";
        assert_eq!(detect(Some("t.txt"), p), Format::TimedText);
        let t = parse_str(p, Format::PlainText).unwrap();
        assert_eq!(t.format, Format::TimedText);
        assert!(!t.timing_estimated);
        let texts: Vec<&str> = t.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["Transcript hey guys welcome back", "today we are going to build", "the end"]);
        assert_eq!(t.segments[0].end, 4.0);
        assert_eq!(t.segments[2].start, 3723.0);
    }

    #[test]
    fn inline_timestamps_and_whisper_cli() {
        let p = "[00:00.000 --> 00:03.000]  one two\n[00:03.000 --> 00:06.000]  three\n";
        let t = parse_str(p, Format::TimedText).unwrap();
        assert_eq!(t.segments[0].text, "one two");
        assert_eq!(t.segments[1].start, 3.0);
        let q = "(0:00) - first\n(0:10) - second\n";
        let t = parse_str(q, Format::TimedText).unwrap();
        assert_eq!(t.segments[1].text, "second");
    }

    #[test]
    fn plain_prose_gets_estimated_times() {
        let p = "This is a sentence with some words. And here is another one that is longer than the first one.";
        assert_eq!(detect(Some("notes.txt"), p), Format::PlainText);
        let t = parse_str(p, Format::PlainText).unwrap();
        assert!(t.timing_estimated);
        assert_eq!(t.segments.len(), 2);
        assert_eq!(t.segments[0].start, 0.0);
        assert!(t.segments[1].start > 2.0);
    }

    #[test]
    fn empty_input_is_an_error() {
        assert!(parse_str("WEBVTT\n\n", Format::Vtt).is_err());
        assert!(parse_str("   ", Format::PlainText).is_err());
    }

    #[test]
    fn clean_text_keeps_real_less_than() {
        assert_eq!(clean_text("a < b and <b>bold</b>"), "a < b and bold");
        assert_eq!(clean_text("{\\an8}top &#x41;&unknown;"), "top A&unknown;");
    }
}
