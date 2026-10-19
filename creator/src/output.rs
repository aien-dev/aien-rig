//! Writing the results to a folder.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::error::Result;
use crate::filler::{to_srt, Captions};
use crate::pipeline::Package;
use crate::time::fmt_clock;
use crate::transcript::Transcript;

pub const YOUTUBE_TXT: &str = "youtube.txt";
pub const DESCRIPTION_TXT: &str = "description.txt";
pub const TAGS_TXT: &str = "tags.txt";
pub const RESULT_JSON: &str = "result.json";
pub const CLEAN_SRT: &str = "captions.clean.srt";
pub const MARKED_SRT: &str = "captions.marked.srt";
pub const FILLERS_TXT: &str = "fillers.txt";

/// Write every output file into `dir` (created if needed). Returns the paths written.
pub fn write_all(
    dir: &Path,
    source_name: &str,
    tr: &Transcript,
    caps: &Captions,
    pkg: Option<&Package>,
) -> Result<Vec<PathBuf>> {
    fs::create_dir_all(dir)?;
    let mut files = Vec::new();
    let mut put = |name: &str, body: String| -> Result<()> {
        let p = dir.join(name);
        fs::write(&p, body)?;
        files.push(p);
        Ok(())
    };
    if let Some(p) = pkg {
        put(YOUTUBE_TXT, render_youtube_txt(source_name, p, caps))?;
        put(DESCRIPTION_TXT, format!("{}\n", p.full_description))?;
        put(TAGS_TXT, format!("{}\n", p.tags.join(", ")))?;
        put(RESULT_JSON, serde_json::to_string_pretty(&result_json(source_name, p, caps)).unwrap_or_default() + "\n")?;
    }
    put(CLEAN_SRT, to_srt(&caps.clean))?;
    put(MARKED_SRT, to_srt(&caps.marked))?;
    put(FILLERS_TXT, render_fillers(caps, tr))?;
    Ok(files)
}

/// The machine-readable result: the package plus clock strings for every time.
pub fn result_json(source_name: &str, p: &Package, caps: &Captions) -> serde_json::Value {
    let mut v = serde_json::to_value(p).unwrap_or_default();
    v["source"] = json!(source_name);
    v["chapters"] = json!(p
        .chapters
        .iter()
        .map(|c| json!({"time": fmt_clock(c.start), "start": c.start, "title": c.title}))
        .collect::<Vec<_>>());
    v["shorts"] = json!(p
        .shorts
        .iter()
        .map(|s| json!({
            "start_time": fmt_clock(s.start), "end_time": fmt_clock(s.end),
            "start": s.start, "end": s.end, "seconds": (s.seconds() * 10.0).round() / 10.0,
            "hook": s.hook, "why": s.why
        }))
        .collect::<Vec<_>>());
    v["fillers"] = serde_json::to_value(&caps.report).unwrap_or_default();
    v
}

fn filler_summary(caps: &Captions) -> String {
    let r = &caps.report;
    if r.total == 0 {
        return "No filler words found.".into();
    }
    let mut counts: Vec<(&String, &usize)> = r.counts.iter().collect();
    counts.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let list: Vec<String> = counts.iter().map(|(k, v)| format!("{k} {v}")).collect();
    format!("Found {} ({} per minute): {}", r.total, r.per_minute, list.join(", "))
}

/// The one file to open: everything to paste into YouTube, in order.
pub fn render_youtube_txt(source_name: &str, p: &Package, caps: &Captions) -> String {
    let mut s = String::new();
    s.push_str("AIEN Rig YouTube helper\n");
    s.push_str(&format!("Made from: {source_name} ({} long)\n", fmt_clock(p.duration_secs)));
    s.push_str(&format!("Model: {}\n\n", p.model));

    s.push_str("TITLE IDEAS (pick one)\n");
    for (i, t) in p.titles.iter().enumerate() {
        s.push_str(&format!("{}. {}\n", i + 1, t));
    }

    s.push_str("\nDESCRIPTION (paste as is; the chapters at the bottom make YouTube show chapters)\n");
    s.push_str("--------\n");
    s.push_str(&p.full_description);
    s.push_str("\n--------\n");
    if !p.chapter_problems.is_empty() {
        s.push_str(&format!("Chapter check: YouTube will not show these chapters until fixed: {}\n", p.chapter_problems.join("; ")));
    } else if !p.chapters.is_empty() {
        s.push_str("Chapter check: passes YouTube's rules (starts at 00:00, 3 or more, each 10 seconds or longer).\n");
    }

    s.push_str("\nTAGS (paste into the Tags box)\n");
    s.push_str(&p.tags.join(", "));
    s.push('\n');

    s.push_str("\nSHORTS PICKS (clip these from the video)\n");
    if p.shorts.is_empty() {
        s.push_str("None found.\n");
    }
    for (i, c) in p.shorts.iter().enumerate() {
        s.push_str(&format!("{}. {} ({} seconds)\n", i + 1, c.range(), c.seconds().round()));
        s.push_str(&format!("   Hook: \"{}\"\n", c.hook));
        if !c.why.is_empty() {
            s.push_str(&format!("   Why: {}\n", c.why));
        }
    }

    s.push_str("\nFILLER WORDS\n");
    s.push_str(&filler_summary(caps));
    s.push_str(&format!(
        "\n{CLEAN_SRT} has them taken out (upload this one to YouTube as your captions).\n{MARKED_SRT} shows them in {{curly brackets}}; {FILLERS_TXT} lists every one with its time, for cutting.\n"
    ));

    if !p.warnings.is_empty() {
        s.push_str("\nPLEASE CHECK\n");
        for w in &p.warnings {
            s.push_str(&format!("- {w}\n"));
        }
    }
    s
}

fn render_fillers(caps: &Captions, tr: &Transcript) -> String {
    let mut s = String::new();
    s.push_str(&filler_summary(caps));
    s.push('\n');
    if tr.timing_estimated {
        s.push_str("Times are estimates: the input had no timestamps.\n");
    }
    if !caps.report.hits.is_empty() {
        s.push('\n');
    }
    for h in &caps.report.hits {
        s.push_str(&format!("{}  {}\n", fmt_clock(h.time), h.text));
    }
    s
}
