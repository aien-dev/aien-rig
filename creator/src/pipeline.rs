//! The YouTube helper itself: transcript in, upload text out.
//!
//! 1. Split the transcript into parts that fit the model's context.
//! 2. Ask the model for notes on each part (summary, topic changes, Shorts moments, keywords).
//! 3. If the notes are too long for one request, merge them in rounds.
//! 4. Ask once for titles, description, chapters, tags and Shorts picks.
//! 5. Check everything in code: chapter rules, clip times, title and tag limits.
//!    Fill gaps from the notes, so a weak model answer still gives usable output.

use serde::Serialize;
use serde_json::Value;

use crate::chapters::{self, Chapter};
use crate::chunk::{estimate_tokens, make_chunks, merge_lines};
use crate::error::Result;
use crate::jsonx::extract_object;
use crate::llm::{ChatModel, ChatRequest};
use crate::prompts;
use crate::shorts::{self, ClipLimits, Short};
use crate::time::{fmt_clock, parse_timestamp};
use crate::transcript::{leading_timestamp, Format, Transcript};

/// Tokens reserved for prompt wording around the transcript or notes.
const PROMPT_OVERHEAD: usize = 700;
/// YouTube title limit.
pub const MAX_TITLE_CHARS: usize = 100;
/// YouTube description limit.
pub const MAX_DESCRIPTION_CHARS: usize = 5000;
/// YouTube limit for all tags together.
pub const MAX_TAGS_CHARS: usize = 500;

#[derive(Debug, Clone)]
pub struct Options {
    /// The model's context window in tokens.
    pub context_tokens: usize,
    /// Transcript tokens per part; by default worked out from `context_tokens`.
    pub chunk_tokens: Option<usize>,
    pub notes_reply_tokens: u32,
    pub final_reply_tokens: u32,
    pub temperature: f32,
    /// A line about the channel ("Minecraft survival builds, family friendly").
    pub about: Option<String>,
    pub clips: ClipLimits,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            context_tokens: 8192,
            chunk_tokens: None,
            notes_reply_tokens: 1200,
            final_reply_tokens: 2000,
            temperature: 0.3,
            about: None,
            clips: ClipLimits::default(),
        }
    }
}

impl Options {
    /// Transcript tokens per part.
    pub fn chunk_budget(&self) -> usize {
        self.chunk_tokens.unwrap_or_else(|| {
            self.context_tokens
                .saturating_sub(self.notes_reply_tokens as usize + PROMPT_OVERHEAD)
                .clamp(400, 4000)
        })
    }

    /// Notes tokens that fit in the final request.
    pub fn notes_budget(&self) -> usize {
        self.context_tokens
            .saturating_sub(self.final_reply_tokens as usize + PROMPT_OVERHEAD)
            .max(500)
    }
}

/// The model's notes on one part of the video.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PartNotes {
    /// 1-based.
    pub part: usize,
    pub start: f64,
    pub end: f64,
    pub summary: String,
    pub topics: Vec<Chapter>,
    pub moments: Vec<Short>,
    pub keywords: Vec<String>,
}

/// One request to the model, for the record.
#[derive(Debug, Clone, Serialize)]
pub struct RequestLog {
    pub step: String,
    pub seconds: f64,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub finish_reason: Option<String>,
    pub busy_retries: u32,
    /// Whether a JSON object could be read from the reply.
    pub parsed: bool,
}

/// Everything the helper produces for one video.
#[derive(Debug, Clone, Serialize)]
pub struct Package {
    pub tool: String,
    pub prompt_version: u32,
    pub model: String,
    pub source_format: Format,
    pub duration_secs: f64,
    pub timing_estimated: bool,
    pub titles: Vec<String>,
    /// Description text without the chapter list.
    pub description: String,
    pub chapters: Vec<Chapter>,
    pub tags: Vec<String>,
    pub shorts: Vec<Short>,
    /// Ready to paste: description, a blank line, then the chapter list.
    pub full_description: String,
    /// Chapter rules the final list breaks (empty when YouTube will accept it).
    pub chapter_problems: Vec<String>,
    pub notes: Vec<PartNotes>,
    pub warnings: Vec<String>,
    pub requests: Vec<RequestLog>,
}

/// Run the whole helper on a transcript.
pub fn create(
    tr: &Transcript,
    model: &dyn ChatModel,
    opts: &Options,
    progress: &mut dyn FnMut(&str),
) -> Result<Package> {
    let duration = tr.duration();
    let mut warnings: Vec<String> = Vec::new();
    let mut log: Vec<RequestLog> = Vec::new();
    if tr.timing_estimated {
        warnings.push(
            "The input had no timestamps, so every time is an estimate from word count. Check chapters and Shorts against the video before using them."
                .into(),
        );
    }

    // 1-2. Notes per part.
    let lines = merge_lines(&tr.segments, 10.0, 160);
    let chunks = make_chunks(&lines, opts.chunk_budget());
    let mut notes: Vec<PartNotes> = Vec::with_capacity(chunks.len());
    for c in &chunks {
        progress(&format!(
            "Reading part {} of {} ({} to {})",
            c.index + 1,
            chunks.len(),
            fmt_clock(c.start),
            fmt_clock(c.end)
        ));
        let req = ChatRequest {
            system: prompts::NOTES_SYSTEM.into(),
            user: prompts::notes_user(c, chunks.len(), duration, opts.about.as_deref()),
            max_tokens: opts.notes_reply_tokens,
            temperature: opts.temperature,
        };
        let step = format!("notes part {}", c.index + 1);
        match ask_json(model, &req, &step, &mut log)? {
            Some(v) => notes.push(parse_notes(&v, c.index + 1, c.start, c.end)),
            None => {
                warnings.push(format!(
                    "The model's notes for part {} ({} to {}) could not be read, so that part may be thin in the results.",
                    c.index + 1,
                    fmt_clock(c.start),
                    fmt_clock(c.end)
                ));
                notes.push(PartNotes { part: c.index + 1, start: c.start, end: c.end, ..Default::default() });
            }
        }
    }

    // 3. Merge notes until they fit one request.
    let mut working = notes.clone();
    let budget = opts.notes_budget();
    let mut round = 0;
    while estimate_tokens(&prompts::render_notes(&working)) > budget && working.len() > 1 && round < 6 {
        round += 1;
        progress(&format!("Combining notes (round {round})"));
        working = condense(model, &working, budget, opts, &mut log)?;
    }

    // 4. Titles, description, chapters, tags, Shorts.
    progress("Writing titles, description, chapters, tags and Shorts picks");
    let notes_text = prompts::render_notes(&working);
    let req = ChatRequest {
        system: prompts::FINAL_SYSTEM.into(),
        user: prompts::final_user(&notes_text, duration, opts.about.as_deref()),
        max_tokens: opts.final_reply_tokens,
        temperature: opts.temperature,
    };
    let v = match ask_json(model, &req, "final", &mut log)? {
        Some(v) => v,
        None => {
            warnings.push("The model's final answer could not be read; results were built from the part notes only.".into());
            Value::Null
        }
    };

    // 5. Check and repair.
    let mut titles = clean_titles(&string_list(v.get("titles")));
    if titles.len() < 5 {
        progress("Asking for more title ideas");
        let summary: Vec<&str> = notes.iter().map(|n| n.summary.as_str()).filter(|s| !s.is_empty()).collect();
        let req = ChatRequest {
            system: prompts::FINAL_SYSTEM.into(),
            user: prompts::titles_user(&summary.join("\n"), opts.about.as_deref()),
            max_tokens: 600,
            temperature: (opts.temperature + 0.2).min(1.0),
        };
        if let Some(v2) = ask_json(model, &req, "more titles", &mut log)? {
            let mut more = string_list(v2.get("titles"));
            let mut all = titles.clone();
            all.append(&mut more);
            titles = clean_titles(&all);
        }
        if titles.len() < 5 {
            warnings.push(format!("Only {} title ideas came back instead of 5.", titles.len()));
        }
    }

    let mut description = clean_description(v.get("description").and_then(Value::as_str).unwrap_or(""));
    if description.is_empty() {
        description = notes.iter().map(|n| n.summary.as_str()).filter(|s| !s.is_empty()).take(2).collect::<Vec<_>>().join(" ");
        description = clean_description(&description);
        warnings.push("The model gave no description; the first part summaries were used instead. Edit it before posting.".into());
    }

    let snap: Vec<f64> = tr.segments.iter().map(|s| s.start).collect();
    let model_chapters = parse_chapters(v.get("chapters"));
    let mut chapter_list = chapters::repair(&model_chapters, duration, &snap);
    if chapter_list.len() < chapters::MIN_CHAPTERS {
        let mut cands = model_chapters.clone();
        cands.extend(notes.iter().flat_map(|n| n.topics.iter().cloned()));
        chapter_list = chapters::repair(&cands, duration, &snap);
        if chapter_list.len() >= chapters::MIN_CHAPTERS {
            warnings.push("The model's chapter list was too short, so topic changes from the part notes were added.".into());
        }
    }
    if chapter_list.len() < chapters::MIN_CHAPTERS {
        let topics: Vec<Chapter> = notes.iter().flat_map(|n| n.topics.iter().cloned()).collect();
        let nearest = |t: f64| -> Option<String> {
            topics
                .iter()
                .min_by(|a, b| (a.start - t).abs().partial_cmp(&(b.start - t).abs()).unwrap_or(std::cmp::Ordering::Equal))
                .filter(|c| (c.start - t).abs() < 120.0)
                .map(|c| c.title.clone())
        };
        chapter_list = chapters::even_split(duration, &nearest);
        if chapter_list.is_empty() {
            warnings.push("The video is too short for chapters (YouTube needs 3 chapters of at least 10 seconds).".into());
        } else {
            warnings.push("Not enough clear topic changes were found, so chapters are evenly spaced. Rename or move them to fit the video.".into());
        }
    }
    let chapter_problems = chapters::check(&chapter_list, duration);

    let mut tags = clean_tags(&string_list(v.get("tags")));
    if tags.is_empty() {
        let kw: Vec<String> = notes.iter().flat_map(|n| n.keywords.iter().cloned()).collect();
        tags = clean_tags(&kw);
        if !tags.is_empty() {
            warnings.push("The model gave no tags; keywords from the part notes were used.".into());
        }
    }

    let model_shorts = parse_shorts(v.get("shorts"));
    let mut picks = shorts::pick(&model_shorts, &tr.segments, duration, opts.clips);
    if picks.len() < 3 {
        let mut cands = model_shorts.clone();
        cands.extend(notes.iter().flat_map(|n| n.moments.iter().cloned()));
        let lim = ClipLimits { max_picks: 3, ..opts.clips };
        let more = shorts::pick(&cands, &tr.segments, duration, lim);
        if more.len() > picks.len() {
            picks = more;
        }
    }
    if picks.len() < 3 {
        warnings.push(format!("Only {} Shorts picks could be matched to the transcript (3 to 5 wanted).", picks.len()));
    }

    let full_description = full_description(&description, &chapter_list);
    progress("Done");
    Ok(Package {
        tool: format!("aien-rig-creator {}", env!("CARGO_PKG_VERSION")),
        prompt_version: prompts::PROMPT_VERSION,
        model: model.model_id(),
        source_format: tr.format,
        duration_secs: duration,
        timing_estimated: tr.timing_estimated,
        titles,
        description,
        chapters: chapter_list,
        tags,
        shorts: picks,
        full_description,
        chapter_problems,
        notes,
        warnings,
        requests: log,
    })
}

/// Ask the model, read a JSON object from its reply. Tries twice.
fn ask_json(
    model: &dyn ChatModel,
    req: &ChatRequest,
    step: &str,
    log: &mut Vec<RequestLog>,
) -> Result<Option<Value>> {
    let mut r = req.clone();
    for attempt in 0..2 {
        let reply = model.chat(&r)?;
        let parsed = extract_object(&reply.text).filter(Value::is_object);
        log.push(RequestLog {
            step: if attempt == 0 { step.to_string() } else { format!("{step} (retry)") },
            seconds: (reply.seconds * 10.0).round() / 10.0,
            prompt_tokens: reply.prompt_tokens,
            completion_tokens: reply.completion_tokens,
            finish_reason: reply.finish_reason.clone(),
            busy_retries: reply.busy_retries,
            parsed: parsed.is_some(),
        });
        if parsed.is_some() {
            return Ok(parsed);
        }
        r.user.push_str("\n\nReply with only the JSON object, starting with { and ending with }.");
        r.temperature = (r.temperature + 0.2).min(1.0);
        if reply.finish_reason.as_deref() == Some("length") {
            r.max_tokens = r.max_tokens.saturating_mul(3) / 2;
        }
    }
    Ok(None)
}

/// Merge neighbouring notes in groups that fit `budget`.
fn condense(
    model: &dyn ChatModel,
    notes: &[PartNotes],
    budget: usize,
    opts: &Options,
    log: &mut Vec<RequestLog>,
) -> Result<Vec<PartNotes>> {
    let mut groups: Vec<Vec<PartNotes>> = Vec::new();
    let mut cur: Vec<PartNotes> = Vec::new();
    for n in notes {
        let mut trial = cur.clone();
        trial.push(n.clone());
        if cur.len() >= 2 && estimate_tokens(&prompts::render_notes(&trial)) > budget {
            groups.push(std::mem::take(&mut cur));
        }
        cur.push(n.clone());
    }
    if !cur.is_empty() {
        // A lone last part joins the group before it so every round shrinks the list.
        if cur.len() == 1 && !groups.is_empty() {
            groups.last_mut().unwrap().append(&mut cur);
        } else {
            groups.push(cur);
        }
    }
    let mut out = Vec::new();
    for (gi, g) in groups.iter().enumerate() {
        let (start, end) = (g[0].start, g[g.len() - 1].end);
        let req = ChatRequest {
            system: prompts::NOTES_SYSTEM.into(),
            user: prompts::condense_user(&prompts::render_notes(g), start, end),
            max_tokens: opts.notes_reply_tokens,
            temperature: opts.temperature,
        };
        let merged = match ask_json(model, &req, &format!("combine notes {}", gi + 1), log)? {
            Some(v) => parse_notes(&v, gi + 1, start, end),
            None => mechanical_merge(g, gi + 1),
        };
        out.push(merged);
    }
    Ok(out)
}

fn mechanical_merge(g: &[PartNotes], part: usize) -> PartNotes {
    PartNotes {
        part,
        start: g[0].start,
        end: g[g.len() - 1].end,
        summary: g.iter().map(|n| n.summary.as_str()).collect::<Vec<_>>().join(" "),
        topics: g.iter().flat_map(|n| n.topics.iter().cloned()).collect(),
        moments: g.iter().flat_map(|n| n.moments.iter().take(2).cloned()).collect(),
        keywords: g.iter().flat_map(|n| n.keywords.iter().cloned()).take(10).collect(),
    }
}

// ---------- reading the model's JSON ----------

/// A time the model wrote: "mm:ss", "h:mm:ss", seconds, or "01:20 - 02:00" (takes the first).
pub fn model_time(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let s = s.trim();
            parse_timestamp(s).or_else(|| {
                let first = s.split(|c: char| c.is_whitespace() || c == '-' || c == '–').next()?;
                parse_timestamp(first.trim_end_matches('s'))
            })
        }
        _ => None,
    }
}

fn text_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// A list of strings from an array of strings, an array of objects with a
/// text-like field, or one comma/newline separated string.
pub fn string_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| match x {
                Value::String(s) => Some(s.clone()),
                Value::Object(o) => ["title", "tag", "text", "name", "value"]
                    .iter()
                    .find_map(|k| o.get(*k).and_then(Value::as_str))
                    .map(str::to_string),
                _ => None,
            })
            .collect(),
        Some(Value::String(s)) => {
            let sep = if s.contains('\n') { '\n' } else { ',' };
            s.split(sep).map(|x| x.to_string()).collect()
        }
        _ => Vec::new(),
    }
}

fn object_list(v: Option<&Value>) -> Vec<&serde_json::Map<String, Value>> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_object).collect()).unwrap_or_default()
}

pub fn parse_chapters(v: Option<&Value>) -> Vec<Chapter> {
    object_list(v)
        .into_iter()
        .filter_map(|o| {
            let start = model_time(o.get("start").or_else(|| o.get("time")).or_else(|| o.get("timestamp")))?;
            Some(Chapter { start, title: text_of(o.get("title").or_else(|| o.get("name"))) })
        })
        .collect()
}

pub fn parse_shorts(v: Option<&Value>) -> Vec<Short> {
    object_list(v)
        .into_iter()
        .filter_map(|o| {
            let start = model_time(o.get("start"))?;
            let end = model_time(o.get("end")).unwrap_or(f64::NAN);
            // A model may write "start": "01:00 - 01:40"; take the end from it.
            let end = if end.is_finite() {
                end
            } else {
                o.get("start")
                    .and_then(Value::as_str)
                    .and_then(|s| s.split(['-', '–']).nth(1))
                    .and_then(|e| parse_timestamp(e.trim()))
                    .unwrap_or(start)
            };
            Some(Short {
                start,
                end,
                hook: text_of(o.get("hook").or_else(|| o.get("quote"))),
                why: text_of(o.get("why").or_else(|| o.get("reason"))),
            })
        })
        .collect()
}

fn parse_notes(v: &Value, part: usize, start: f64, end: f64) -> PartNotes {
    let in_part = |t: f64| t >= start - 5.0 && t <= end + 5.0;
    PartNotes {
        part,
        start,
        end,
        summary: text_of(v.get("summary")),
        topics: parse_chapters(v.get("topics")).into_iter().filter(|c| in_part(c.start)).collect(),
        moments: parse_shorts(v.get("moments")).into_iter().filter(|m| in_part(m.start)).collect(),
        keywords: string_list(v.get("keywords"))
            .into_iter()
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .take(12)
            .collect(),
    }
}

// ---------- tidying ----------

fn no_angle(s: &str) -> String {
    s.chars().filter(|c| *c != '<' && *c != '>').collect()
}

/// Up to 5 distinct titles, numbering and quotes removed, at most 100 characters.
pub fn clean_titles(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in raw {
        let mut s = t.trim();
        // "1. Title", "2) Title", "- Title"
        let digits = s.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 && digits < 3 {
            let rest = &s[digits..];
            if let Some(r) = rest.strip_prefix(['.', ')', ':']) {
                s = r.trim_start();
            }
        }
        s = s.trim_start_matches(['-', '*', '•']).trim();
        let s = no_angle(s.trim_matches(['"', '\'', '“', '”']).trim());
        let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
        let s = chapters::truncate_words(&s, MAX_TITLE_CHARS);
        if s.is_empty() || out.iter().any(|o| o.eq_ignore_ascii_case(&s)) {
            continue;
        }
        out.push(s);
        if out.len() == 5 {
            break;
        }
    }
    out
}

/// Description text without timestamp lines or angle brackets.
pub fn clean_description(s: &str) -> String {
    let lines: Vec<String> = s
        .lines()
        .filter(|l| leading_timestamp(l.trim()).is_none())
        .map(|l| no_angle(l).trim_end().to_string())
        .collect();
    let mut out = String::new();
    let mut blank = 0;
    for l in lines {
        if l.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(&l);
        out.push('\n');
    }
    out.trim().to_string()
}

/// Distinct tags without `#`, within YouTube's 500 character total.
pub fn clean_tags(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut used = 0usize;
    for t in raw.iter().flat_map(|t| t.split(',')) {
        let s = no_angle(t.trim().trim_start_matches('#').trim_matches('"').trim());
        let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
        if s.is_empty() || s.chars().count() > 100 || out.iter().any(|o| o.eq_ignore_ascii_case(&s)) {
            continue;
        }
        // YouTube counts quotes around tags with spaces, plus the commas.
        let cost = s.chars().count() + if s.contains(' ') { 2 } else { 0 } + usize::from(!out.is_empty());
        if used + cost > MAX_TAGS_CHARS {
            break;
        }
        used += cost;
        out.push(s);
    }
    out
}

/// Description plus chapter list, within YouTube's 5000 character limit.
pub fn full_description(description: &str, chapters: &[Chapter]) -> String {
    let list: String = chapters.iter().map(|c| c.line()).collect::<Vec<_>>().join("\n");
    let room = MAX_DESCRIPTION_CHARS.saturating_sub(list.chars().count() + 2);
    let body = if description.chars().count() > room {
        chapters::truncate_words(description, room)
    } else {
        description.to_string()
    };
    match (body.is_empty(), list.is_empty()) {
        (true, _) => list,
        (_, true) => body,
        _ => format!("{body}\n\n{list}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn times_from_the_model() {
        assert_eq!(model_time(Some(&json!("01:20"))), Some(80.0));
        assert_eq!(model_time(Some(&json!(80))), Some(80.0));
        assert_eq!(model_time(Some(&json!("01:20 - 02:00"))), Some(80.0));
        assert_eq!(model_time(Some(&json!("95s"))), Some(95.0));
        assert_eq!(model_time(Some(&json!("soon"))), None);
        assert_eq!(model_time(None), None);
    }

    #[test]
    fn titles_are_tidied() {
        let raw: Vec<String> = ["1. \"First <one>\"", "2) second", "- Third", "first one", "", "Fifth", "Sixth", "Seventh"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let t = clean_titles(&raw);
        assert_eq!(t, ["First one", "second", "Third", "Fifth", "Sixth"]);
        let long = vec!["word ".repeat(40)];
        assert!(clean_titles(&long)[0].chars().count() <= 100);
    }

    #[test]
    fn tags_respect_youtube_limit() {
        let raw: Vec<String> = (0..100).map(|i| format!("#tag number {i}")).collect();
        let t = clean_tags(&raw);
        let cost: usize = t.iter().map(|s| s.len() + 2).sum::<usize>() + t.len() - 1;
        assert!(cost <= 500, "{cost}");
        assert_eq!(t[0], "tag number 0");
        assert_eq!(clean_tags(&["a, b, A".to_string()]), ["a", "b"]);
    }

    #[test]
    fn description_drops_timestamps_and_brackets() {
        let d = clean_description("Watch <this>!\n\n\n\n00:00 Intro\n1:20 Build\nMore text");
        assert_eq!(d, "Watch this!\n\nMore text");
    }

    #[test]
    fn full_description_layout_and_limit() {
        let ch = vec![Chapter::new(0.0, "Intro"), Chapter::new(30.0, "Build"), Chapter::new(60.0, "End")];
        let d = full_description("Hello.", &ch);
        assert_eq!(d, "Hello.\n\n00:00 Intro\n00:30 Build\n01:00 End");
        let d = full_description(&"word ".repeat(2000), &ch);
        assert!(d.chars().count() <= MAX_DESCRIPTION_CHARS);
        assert!(d.ends_with("01:00 End"));
    }

    #[test]
    fn lists_in_odd_shapes() {
        assert_eq!(string_list(Some(&json!("a, b"))), ["a", " b"]);
        assert_eq!(string_list(Some(&json!([{"title": "x"}, "y", 3]))), ["x", "y"]);
        let s = parse_shorts(Some(&json!([{"start": "01:00 - 01:40", "hook": "h"}])));
        assert_eq!((s[0].start, s[0].end), (60.0, 100.0));
    }
}
