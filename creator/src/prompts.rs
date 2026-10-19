//! The words we send to the model. Kept in one place so they are easy to tune.

use crate::chunk::Chunk;
use crate::pipeline::PartNotes;
use crate::time::fmt_clock;

/// Bump when prompts or post-processing change, so saved results can be told apart.
pub const PROMPT_VERSION: u32 = 1;

pub const NOTES_SYSTEM: &str = "You help a YouTube creator. You read part of a video transcript and write short, accurate notes about it. Answer with one JSON object and nothing else.";

pub const FINAL_SYSTEM: &str = "You write YouTube upload text for a creator from notes about their video. Only describe what the notes say happens in the video. Answer with one JSON object and nothing else.";

fn about_line(about: Option<&str>) -> String {
    match about.map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => format!("About the channel: {a}\n"),
        None => String::new(),
    }
}

pub fn notes_user(c: &Chunk, parts: usize, duration: f64, about: Option<&str>) -> String {
    let (a, b) = (fmt_clock(c.start), fmt_clock(c.end));
    let which = if parts == 1 {
        "This is the whole transcript.".to_string()
    } else {
        format!("This is part {} of {parts}, from {a} to {b}.", c.index + 1)
    };
    format!(
        "The video is {dur} long. {which}\n{about}\nTranscript (each line starts with its time):\n<<<\n{text}>>>\n\n\
Write a JSON object with these keys:\n\
\"summary\": 2 to 4 sentences on what happens in this part. Plain words, no hype.\n\
\"topics\": the points where the subject clearly changes, as {{\"start\": \"mm:ss\", \"title\": \"2 to 6 words\"}}. Use the time of the line where the new subject begins. Titles read like YouTube chapter names. If the subject never changes, give one topic at \"{a}\".\n\
\"moments\": up to 3 stretches of 15 to 60 seconds that would work as a YouTube Short on their own (funny, surprising, a clear tip, a big reaction, a satisfying result), as {{\"start\": \"mm:ss\", \"end\": \"mm:ss\", \"hook\": \"the first words of the clip, copied from the transcript\", \"why\": \"one short sentence\"}}. Use an empty list if nothing stands out.\n\
\"keywords\": up to 8 words or short phrases a viewer might search for to find this.\n\n\
Only use times that appear in the transcript above.",
        dur = fmt_clock(duration),
        about = about_line(about),
        text = c.text,
    )
}

/// Notes as plain text for the final step.
pub fn render_notes(notes: &[PartNotes]) -> String {
    let mut out = String::new();
    for n in notes {
        out.push_str(&format!("Part {} ({} to {})\n", n.part, fmt_clock(n.start), fmt_clock(n.end)));
        if !n.summary.is_empty() {
            out.push_str(&format!("Summary: {}\n", n.summary));
        }
        if !n.topics.is_empty() {
            let t: Vec<String> = n.topics.iter().map(|c| c.line()).collect();
            out.push_str(&format!("Topics: {}\n", t.join("; ")));
        }
        if !n.moments.is_empty() {
            out.push_str("Possible Shorts:\n");
            for m in &n.moments {
                out.push_str(&format!("- {}: \"{}\" ({})\n", m.range(), m.hook, m.why));
            }
        }
        if !n.keywords.is_empty() {
            out.push_str(&format!("Keywords: {}\n", n.keywords.join(", ")));
        }
        out.push('\n');
    }
    out
}

pub fn final_user(notes_text: &str, duration: f64, about: Option<&str>) -> String {
    let max_ch = ((duration / 60.0).floor() as usize).clamp(3, 15);
    let gap = if duration < 180.0 { 10 } else { 30 };
    format!(
        "The video is {dur} long.\n{about}\nNotes on the video, part by part:\n<<<\n{notes_text}>>>\n\n\
Write a JSON object with these keys:\n\
\"titles\": exactly 5 different title ideas, each under 70 characters, specific to this video, made to be clicked but honest. Mix styles: one plain, one question, one with a number or result, one curiosity hook, one short and punchy. No emoji, no hashtags, no sentences in ALL CAPS.\n\
\"description\": the top of the YouTube description: a 1 to 2 sentence hook that makes people want to watch, then a short paragraph on what the video covers. Under 800 characters. No timestamps, no links, no hashtags.\n\
\"chapters\": 3 to {max_ch} chapters as {{\"start\": \"mm:ss\", \"title\": \"2 to 6 words\"}}. The first starts at \"00:00\". Each starts at least {gap} seconds after the one before it and at least {gap} seconds before the video ends at {dur}. Use the topic times from the notes.\n\
\"tags\": 10 to 15 search tags, most important first: the game or subject, then what happens, then broader terms. Lowercase, no # signs.\n\
\"shorts\": 3 to 5 picks for YouTube Shorts, strongest first, taken from the possible Shorts in the notes, as {{\"start\": \"mm:ss\", \"end\": \"mm:ss\", \"hook\": \"the opening words\", \"why\": \"one sentence on why it works as a Short\"}}. Each 15 to 60 seconds long. They must not overlap.",
        dur = fmt_clock(duration),
        about = about_line(about),
    )
}

pub fn condense_user(notes_text: &str, start: f64, end: f64) -> String {
    format!(
        "Here are notes on consecutive parts of one video, covering {a} to {b}:\n<<<\n{notes_text}>>>\n\n\
Merge them into one set of notes, as a JSON object with the keys \"summary\" (3 to 6 sentences), \"topics\" (the most important subject changes, as {{\"start\": \"mm:ss\", \"title\": \"...\"}}, keeping their times), \"moments\" (the best 4 or fewer possible Shorts, as {{\"start\": \"mm:ss\", \"end\": \"mm:ss\", \"hook\": \"...\", \"why\": \"...\"}}, keeping their times) and \"keywords\" (up to 10).",
        a = fmt_clock(start),
        b = fmt_clock(end),
    )
}

pub fn titles_user(summary: &str, about: Option<&str>) -> String {
    format!(
        "{about}What the video is about:\n<<<\n{summary}\n>>>\n\n\
Write exactly 5 different YouTube title ideas for this video, each under 70 characters, honest and specific. Mix styles: plain, question, number or result, curiosity hook, short and punchy. Answer as {{\"titles\": [\"...\", \"...\", \"...\", \"...\", \"...\"]}}.",
        about = about_line(about),
    )
}
