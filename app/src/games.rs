//! Noticing when a game runs, and what to do about it.
//!
//! While a listed game runs, the app (1) stops the engine so the game gets
//! all of the GPU memory, and (2) makes sure Windows Game Mode is on. When the
//! game exits it puts both back the way they were.

use crate::tools::no_window;
use std::process::Command;

/// Lower-case program names of running processes.
pub fn running_programs() -> Vec<String> {
    let out = if cfg!(windows) {
        let mut c = Command::new("tasklist");
        c.args(["/FO", "CSV", "/NH"]);
        no_window(&mut c);
        c.output()
    } else {
        Command::new("ps").args(["-A", "-o", "comm="]).output()
    };
    match out {
        Ok(o) => parse_programs(&String::from_utf8_lossy(&o.stdout), cfg!(windows)),
        Err(_) => Vec::new(),
    }
}

/// `tasklist /FO CSV /NH` lines look like `"cs2.exe","1234","Console","1","1,234 K"`.
pub fn parse_programs(text: &str, csv: bool) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() {
                return None;
            }
            let name = if csv {
                l.strip_prefix('"')?.split('"').next()?.to_string()
            } else {
                l.rsplit('/').next()?.to_string()
            };
            Some(name.to_ascii_lowercase())
        })
        .collect()
}

/// The first listed game that is running.
pub fn find_game(games: &[String], running: &[String]) -> Option<String> {
    games
        .iter()
        .find(|g| {
            let g = g.trim().to_ascii_lowercase();
            !g.is_empty() && running.contains(&g)
        })
        .cloned()
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Started(String),
    Ended(String),
}

/// What changed between two looks at the process list.
pub fn changes(before: Option<&str>, now: Option<&str>) -> Vec<Change> {
    match (before, now) {
        (None, Some(n)) => vec![Change::Started(n.into())],
        (Some(b), None) => vec![Change::Ended(b.into())],
        (Some(b), Some(n)) if !b.eq_ignore_ascii_case(n) => {
            vec![Change::Ended(b.into()), Change::Started(n.into())]
        }
        _ => vec![],
    }
}

const GAMEBAR_KEY: &str = r"HKCU\Software\Microsoft\GameBar";
const GAMEMODE_VALUE: &str = "AutoGameModeEnabled";

/// Reads Windows Game Mode. `Ok(None)` means the value is not set, which
/// Windows 11 treats as on.
pub fn read_game_mode() -> Result<Option<u32>, String> {
    if !cfg!(windows) {
        return Err("Game Mode exists only on Windows".into());
    }
    let mut c = Command::new("reg");
    c.args(["query", GAMEBAR_KEY, "/v", GAMEMODE_VALUE]);
    no_window(&mut c);
    let o = c.output().map_err(|e| e.to_string())?;
    if !o.status.success() {
        return Ok(None);
    }
    Ok(parse_reg_dword(&String::from_utf8_lossy(&o.stdout)))
}

/// Finds `REG_DWORD    0x1` in `reg query` output.
pub fn parse_reg_dword(text: &str) -> Option<u32> {
    text.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        while let Some(tok) = it.next() {
            if tok == "REG_DWORD" {
                let v = it.next()?;
                return u32::from_str_radix(v.trim_start_matches("0x"), 16).ok();
            }
        }
        None
    })
}

/// Sets the value, or removes it when `value` is None.
pub fn write_game_mode(value: Option<u32>) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("Game Mode exists only on Windows".into());
    }
    let mut c = Command::new("reg");
    match value {
        Some(v) => c.args([
            "add",
            GAMEBAR_KEY,
            "/v",
            GAMEMODE_VALUE,
            "/t",
            "REG_DWORD",
            "/d",
            &v.to_string(),
            "/f",
        ]),
        None => c.args(["delete", GAMEBAR_KEY, "/v", GAMEMODE_VALUE, "/f"]),
    };
    no_window(&mut c);
    let o = c.output().map_err(|e| e.to_string())?;
    if o.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tasklist_and_ps() {
        let t = "\"System Idle Process\",\"0\",\"Services\",\"0\",\"8 K\"\r\n\"cs2.exe\",\"4242\",\"Console\",\"1\",\"2,345,678 K\"\r\n";
        assert_eq!(parse_programs(t, true), vec!["system idle process", "cs2.exe"]);
        let p = "/usr/bin/bash\nsshd\n";
        assert_eq!(parse_programs(p, false), vec!["bash", "sshd"]);
    }

    #[test]
    fn finds_games_case_insensitively() {
        let games = vec!["CS2.exe".to_string(), "r5apex.exe".into()];
        let running = vec!["explorer.exe".to_string(), "cs2.exe".into()];
        assert_eq!(find_game(&games, &running).as_deref(), Some("CS2.exe"));
        assert_eq!(find_game(&games, &["explorer.exe".into()]), None);
        assert_eq!(find_game(&["".into()], &["".into()]), None);
    }

    #[test]
    fn change_detection() {
        assert_eq!(changes(None, None), vec![]);
        assert_eq!(changes(None, Some("a")), vec![Change::Started("a".into())]);
        assert_eq!(changes(Some("a"), Some("a")), vec![]);
        assert_eq!(changes(Some("a"), None), vec![Change::Ended("a".into())]);
        assert_eq!(
            changes(Some("a"), Some("b")),
            vec![Change::Ended("a".into()), Change::Started("b".into())]
        );
    }

    #[test]
    fn parses_reg_output() {
        let t = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\GameBar\r\n    AutoGameModeEnabled    REG_DWORD    0x0\r\n\r\n";
        assert_eq!(parse_reg_dword(t), Some(0));
        assert_eq!(parse_reg_dword("    X    REG_DWORD    0x1"), Some(1));
        assert_eq!(parse_reg_dword("nothing"), None);
    }
}
