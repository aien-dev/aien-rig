//! Measurement receipts: one JSON file per run, per game, in
//! %LOCALAPPDATA%\AIEN-Rig\receipts. A receipt holds the raw frame times, so
//! anyone can recompute the numbers, plus what was changed on the PC at the
//! time. Comparing two receipts gives the before/after. Pure logic.

use crate::frames::Source;
use crate::stats::FrameStats;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SCHEMA: &str = "aien-rig-tuner/receipt/v1";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SystemSnapshot {
    pub gpu: Option<String>,
    pub driver: Option<String>,
    pub windows_build: Option<u32>,
    pub display: Option<String>,
    pub power_plan: Option<String>,
    pub hags: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeenRow {
    pub source: Source,
    pub pid: u32,
    pub event_id: u16,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    pub schema: String,
    pub tool_version: String,
    pub game: String,
    pub exe: String,
    pub label: String,
    pub started_at: String,
    pub seconds_requested: u32,
    pub frame_source: Source,
    pub source_event_id: u16,
    pub pid: u32,
    pub qpc_frequency: i64,
    pub stats: FrameStats,
    pub stats_definition: String,
    /// [seconds since start, frame ms], at most 600 points, slowest frame per bucket.
    pub graph: Vec<[f64; 2]>,
    /// Every frame time in ms (3 decimals), so the stats can be recomputed.
    pub frame_times_ms: Vec<f64>,
    pub events_seen: Vec<SeenRow>,
    /// Tweaks from the undo log that were active during this run.
    pub active_tweaks: Vec<String>,
    pub system: SystemSnapshot,
    pub notes: Vec<String>,
}

/// "Cyberpunk 2077" -> "cyberpunk-2077"
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let t = out.trim_end_matches('-').to_string();
    if t.is_empty() {
        "game".into()
    } else {
        t
    }
}

pub fn file_name(game: &str, stamp: &str, label: &str) -> String {
    format!("{}-{}-{}.json", slug(game), stamp, slug(label))
}

pub fn write(dir: &Path, r: &Receipt, stamp: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(file_name(&r.game, stamp, &r.label));
    std::fs::write(&path, serde_json::to_string_pretty(r)?)?;
    Ok(path)
}

pub fn read(path: &Path) -> Result<Receipt> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// Receipts for one game, oldest first (file names sort by time).
pub fn list_for_game(dir: &Path, game: &str) -> Vec<PathBuf> {
    let prefix = format!("{}-", slug(game));
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            // slug, then an 8-digit date: avoids "doom" matching "doom-eternal-..."
            n.starts_with(&prefix)
                && n.ends_with(".json")
                && n[prefix.len()..].chars().take(8).all(|c| c.is_ascii_digit())
                && n.len() > prefix.len() + 8
        })
        .collect();
    v.sort();
    v
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Comparison {
    pub game: String,
    pub before: String,
    pub after: String,
    pub avg_fps: (f64, f64, f64),
    pub low_1pct_fps: (f64, f64, f64),
    pub low_01pct_fps: (f64, f64, f64),
    pub tweaks_added: Vec<String>,
    pub tweaks_removed: Vec<String>,
    pub warnings: Vec<String>,
    pub summary: String,
}

fn pct(a: f64, b: f64) -> f64 {
    if a > 0.0 {
        (b - a) / a * 100.0
    } else {
        0.0
    }
}

pub fn compare(before: &Receipt, after: &Receipt) -> Comparison {
    let tri = |a: f64, b: f64| (a, b, pct(a, b));
    let avg = tri(before.stats.avg_fps, after.stats.avg_fps);
    let l1 = tri(before.stats.low_1pct_fps, after.stats.low_1pct_fps);
    let l01 = tri(before.stats.low_01pct_fps, after.stats.low_01pct_fps);
    let added: Vec<String> = after.active_tweaks.iter().filter(|t| !before.active_tweaks.contains(t)).cloned().collect();
    let removed: Vec<String> = before.active_tweaks.iter().filter(|t| !after.active_tweaks.contains(t)).cloned().collect();
    let mut warnings = Vec::new();
    if before.frame_source != after.frame_source {
        warnings.push("The two runs counted frames from different sources; compare with care.".into());
    }
    if before.system.driver != after.system.driver {
        warnings.push("The graphics driver changed between the runs.".into());
    }
    if before.system.display != after.system.display {
        warnings.push("The monitor mode changed between the runs (expected if refresh-rate-max was applied).".into());
    }
    let (s1, s2) = (before.stats.seconds, after.stats.seconds);
    if s1 < 20.0 || s2 < 20.0 {
        warnings.push("At least one run is shorter than 20 seconds; longer runs give steadier numbers.".into());
    }
    if avg.2.abs() < 3.0 {
        warnings.push("The change is small. Run each measurement three times in the same spot of the game to tell it apart from normal run-to-run variation.".into());
    }
    let dir = |p: f64| if p >= 0.0 { "up" } else { "down" };
    let summary = format!(
        "{}: average FPS {} {:.1}% ({:.1} -> {:.1}), 1% low {} {:.1}% ({:.1} -> {:.1}).",
        before.game,
        dir(avg.2), avg.2.abs(), avg.0, avg.1,
        dir(l1.2), l1.2.abs(), l1.0, l1.1
    );
    Comparison {
        game: before.game.clone(),
        before: before.started_at.clone(),
        after: after.started_at.clone(),
        avg_fps: avg,
        low_1pct_fps: l1,
        low_01pct_fps: l01,
        tweaks_added: added,
        tweaks_removed: removed,
        warnings,
        summary,
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::stats;

    pub fn sample(game: &str, label: &str, ft_ms: f64, tweaks: &[&str]) -> Receipt {
        let ft = vec![ft_ms; 3000];
        Receipt {
            schema: SCHEMA.into(),
            tool_version: "test".into(),
            game: game.into(),
            exe: "game.exe".into(),
            label: label.into(),
            started_at: "2026-10-19T17:00:00Z".into(),
            seconds_requested: 30,
            frame_source: Source::Dxgi,
            source_event_id: 42,
            pid: 1,
            qpc_frequency: 10_000_000,
            stats: stats::compute(&ft).unwrap(),
            stats_definition: stats::STATS_DEFINITION.into(),
            graph: stats::graph(&ft, 600),
            frame_times_ms: ft,
            events_seen: vec![],
            active_tweaks: tweaks.iter().map(|s| s.to_string()).collect(),
            system: SystemSnapshot::default(),
            notes: vec![],
        }
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Cyberpunk 2077"), "cyberpunk-2077");
        assert_eq!(slug("  Tom Clancy's: Rainbow Six® Siege "), "tom-clancy-s-rainbow-six-siege");
        assert_eq!(slug("???"), "game");
        assert_eq!(file_name("Doom", "20261019-170000", "before"), "doom-20261019-170000-before.json");
    }

    #[test]
    fn compare_reports_percent_and_tweaks() {
        let a = sample("Doom", "before", 10.0, &[]);
        let b = sample("Doom", "after", 8.0, &["hags-on"]);
        let c = compare(&a, &b);
        assert!((c.avg_fps.2 - 25.0).abs() < 1e-9);
        assert_eq!(c.tweaks_added, vec!["hags-on".to_string()]);
        assert!(c.summary.contains("up 25.0%"));
        assert!(c.summary.contains("100.0 -> 125.0"));
        let same = compare(&a, &a);
        assert!(same.warnings.iter().any(|w| w.contains("small")));
    }

    #[test]
    fn write_list_read_round_trip() {
        let dir = std::env::temp_dir().join(format!("rig-tune-receipts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = sample("Doom", "before", 10.0, &[]);
        let b = sample("Doom", "after", 9.0, &["x"]);
        let other = sample("Doom Eternal", "before", 9.0, &[]);
        write(&dir, &a, "20261019-170000").unwrap();
        write(&dir, &b, "20261019-171000").unwrap();
        write(&dir, &other, "20261019-172000").unwrap();
        let l = list_for_game(&dir, "Doom");
        assert_eq!(l.len(), 2);
        assert_eq!(read(&l[1]).unwrap(), b);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
