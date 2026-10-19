//! Settings saved as JSON in the data folder, plus small helpers for paths
//! and time stamps.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// "wsl": run the engine inside WSL2 (the plan for the RTX 5070, see
    /// docs/WINDOWS-GPU-PATH.md). "command": run `engine_command` directly.
    pub engine_mode: String,
    /// WSL distribution name. Empty means WSL's default distribution.
    pub engine_distro: String,
    /// Command line that starts the engine (inside WSL in "wsl" mode).
    pub engine_command: String,
    /// Where the engine answers (OpenAI-style API).
    pub engine_url: String,
    /// Program names that count as "a game is running".
    pub games: Vec<String>,
    /// While a game runs: pause the engine and make sure Windows Game Mode is on.
    pub auto_game_mode: bool,
    /// Arguments passed to rig-tune.exe for each button. `{game}` is replaced.
    pub tune_scan: Vec<String>,
    pub tune_measure: Vec<String>,
    pub tune_apply: Vec<String>,
    pub tune_undo: Vec<String>,
    /// Run by the game watcher when a game starts / ends (the tuner's own
    /// game mode: it pauses Windows-side AI server processes). Empty = skip.
    pub tune_game_mode_on: Vec<String>,
    pub tune_game_mode_off: Vec<String>,
    /// Arguments passed to rig-create.exe. `{file}`, `{server}` (engine
    /// address + /v1) and `{out}` (results folder) are replaced.
    pub create_args: Vec<String>,
    /// Arguments passed to rig-listen.exe (speech to text) for videos.
    /// `{file}` and `{out}` (transcript JSON path) are replaced.
    pub listen_args: Vec<String>,
    /// Longest a tool may run before it is stopped, in seconds.
    pub tool_timeout_secs: u64,
}

fn v(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

impl Default for Config {
    fn default() -> Self {
        Config {
            engine_mode: "wsl".into(),
            engine_distro: String::new(),
            engine_command: "aien-serve-http --engine omega --bind 127.0.0.1 --port 18120".into(),
            engine_url: "http://127.0.0.1:18120".into(),
            games: v(&[
                "cs2.exe",
                "FortniteClient-Win64-Shipping.exe",
                "VALORANT-Win64-Shipping.exe",
                "r5apex.exe",
                "RocketLeague.exe",
                "GTA5.exe",
                "eldenring.exe",
                "Overwatch.exe",
            ]),
            auto_game_mode: true,
            tune_scan: v(&["scan", "--json"]),
            tune_measure: v(&["measure", "{game}", "--json"]),
            tune_apply: v(&["apply", "{game}", "--json"]),
            tune_undo: v(&["undo", "--json"]),
            tune_game_mode_on: v(&["game-mode", "on"]),
            tune_game_mode_off: v(&["game-mode", "off"]),
            create_args: v(&["{file}", "--server", "{server}", "--out", "{out}"]),
            listen_args: v(&["{file}", "--out", "{out}"]),
            tool_timeout_secs: 1800,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Config {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, path)
    }

    /// Checks values that would make the app misbehave.
    pub fn validate(&self) -> Result<(), String> {
        if self.engine_mode != "wsl" && self.engine_mode != "command" {
            return Err("engine_mode must be \"wsl\" or \"command\"".into());
        }
        crate::http::parse_http_url(&self.engine_url).map_err(|e| format!("engine_url: {e}"))?;
        if self.engine_distro.chars().any(|c| !(c.is_ascii_alphanumeric() || "-_.".contains(c))) {
            return Err("engine_distro may only contain letters, digits, - _ .".into());
        }
        if self.tool_timeout_secs == 0 {
            return Err("tool_timeout_secs must be above 0".into());
        }
        Ok(())
    }
}

/// Folder for settings, receipts and logs, shared with rig-tune.exe so every
/// receipt ends up in one place.
/// Windows: %LOCALAPPDATA%\AIEN-Rig. Elsewhere: ~/.local/share/aien-rig.
/// AIEN_RIG_HOME overrides both.
pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("AIEN_RIG_HOME") {
        return PathBuf::from(p);
    }
    if cfg!(windows) {
        if let Some(p) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(p).join("AIEN-Rig");
        }
    }
    if let Some(p) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(p).join("aien-rig");
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        return PathBuf::from(home).join(".local").join("share").join("aien-rig");
    }
    PathBuf::from("aien-rig-data")
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// (year, month, day) from days since 1970-01-01 (Howard Hinnant's method).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// "2026-10-19T17:04:05Z"
pub fn iso_utc(secs: u64) -> String {
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let s = secs % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        (s / 60) % 60,
        s % 60
    )
}

/// "20261019-170405" for file names.
pub fn stamp(secs: u64) -> String {
    iso_utc(secs)
        .replace(['-', ':'], "")
        .replace('T', "-")
        .trim_end_matches('Z')
        .to_string()
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn time_formatting() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_792_429_445), "2026-10-19T17:04:05Z");
        assert_eq!(stamp(1_792_429_445), "20261019-170405");
        assert_eq!(iso_utc(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn config_round_trip_and_defaults_for_missing_fields() {
        let dir = std::env::temp_dir().join(format!("aien-rig-cfg-{}", std::process::id()));
        let path = dir.join("config.json");
        let mut c = Config::default();
        c.games = vec!["game.exe".into()];
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path), c);
        std::fs::write(&path, br#"{"engine_distro":"Ubuntu-24.04"}"#).unwrap();
        let c2 = Config::load(&path);
        assert_eq!(c2.engine_distro, "Ubuntu-24.04");
        assert_eq!(c2.engine_url, Config::default().engine_url);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn validation() {
        assert!(Config::default().validate().is_ok());
        let mut c = Config::default();
        c.engine_distro = "Ubuntu; rm -rf".into();
        assert!(c.validate().is_err());
        let mut c = Config::default();
        c.engine_url = "https://x".into();
        assert!(c.validate().is_err());
    }
}
