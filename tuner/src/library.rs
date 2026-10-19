//! Finding installed games: Steam, Epic Games and the Xbox app.
//! Parsing is pure; discovery walks the file system with std only, so it is
//! tested on every OS with temporary folders. The Windows layer just supplies
//! the roots (Steam path from the registry, drive letters).

use crate::vdf;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Launcher {
    Steam,
    Epic,
    Xbox,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Game {
    pub launcher: Launcher,
    pub name: String,
    pub id: String,
    pub install_dir: String,
    /// Likely game executables (file names), best guess first.
    pub exe_candidates: Vec<String>,
}

/// Steam library folders listed in steamapps/libraryfolders.vdf.
pub fn steam_library_paths(libraryfolders_vdf: &str) -> Vec<String> {
    let root = vdf::parse(libraryfolders_vdf);
    let mut out = Vec::new();
    if let Some(lf) = vdf::get(&root, "libraryfolders").and_then(|n| n.as_obj()) {
        for nodes in lf.values() {
            for n in nodes {
                if let Some(o) = n.as_obj() {
                    if let Some(p) = vdf::get_str(o, "path") {
                        out.push(p.to_string());
                    }
                } else if let Some(p) = n.as_str() {
                    // very old format: "1" "D:\\SteamLibrary"
                    if p.contains(['\\', '/']) {
                        out.push(p.to_string());
                    }
                }
            }
        }
    }
    out
}

/// (appid, name, installdir) from an appmanifest_*.acf.
pub fn parse_acf(text: &str) -> Option<(String, String, String)> {
    let root = vdf::parse(text);
    let st = vdf::get(&root, "AppState")?.as_obj()?;
    Some((
        vdf::get_str(st, "appid")?.to_string(),
        vdf::get_str(st, "name")?.to_string(),
        vdf::get_str(st, "installdir")?.to_string(),
    ))
}

/// Steam entries that are tools, not games.
pub fn is_steam_tool(appid: &str, name: &str) -> bool {
    const TOOL_IDS: [&str; 4] = ["228980", "1070560", "1391110", "1628350"];
    let n = name.to_ascii_lowercase();
    TOOL_IDS.contains(&appid)
        || n.contains("redistributable")
        || n.starts_with("proton")
        || n.contains("steam linux runtime")
        || n == "steamvr"
}

/// One Epic manifest (*.item JSON).
pub fn parse_epic_item(json: &str) -> Option<Game> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let name = v.get("DisplayName")?.as_str()?.to_string();
    let dir = v.get("InstallLocation")?.as_str()?.to_string();
    let id = v
        .get("AppName")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();
    let mut exes = Vec::new();
    if let Some(exe) = v.get("LaunchExecutable").and_then(|x| x.as_str()) {
        if let Some(file) = exe.rsplit(['\\', '/']).next() {
            if !file.is_empty() {
                exes.push(file.to_string());
            }
        }
    }
    // Skip add-ons and DLC entries that have no executable of their own.
    let is_app = v
        .get("bIsApplication")
        .and_then(|x| x.as_bool())
        .unwrap_or(true);
    if !is_app {
        return None;
    }
    Some(Game { launcher: Launcher::Epic, name, id, install_dir: dir, exe_candidates: exes })
}

/// Paths (relative to the drive root) listed in an Xbox app .GamingRoot file:
/// "RGBX", a little-endian u32 count, then that many NUL-terminated UTF-16LE
/// strings.
pub fn parse_gaming_root(bytes: &[u8]) -> Vec<String> {
    if bytes.len() < 8 || &bytes[0..4] != b"RGBX" {
        return Vec::new();
    }
    let count = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let (pairs, _) = bytes[8..].as_chunks::<2>();
    let units: Vec<u16> = pairs.iter().map(|c| u16::from_le_bytes(*c)).collect();
    units
        .split(|u| *u == 0)
        .filter(|s| !s.is_empty())
        .take(count.min(64))
        .map(String::from_utf16_lossy)
        .collect()
}

/// Rank executables in a game folder: drop launchers, crash reporters,
/// installers and anti-cheat services, then prefer the biggest files.
pub fn pick_exes(files: &[(String, u64)]) -> Vec<String> {
    const SKIP: [&str; 24] = [
        "unins", "setup", "install", "redist", "vcredist", "vc_redist", "dxsetup", "crash",
        "report", "launcher", "easyanticheat", "eac_", "beservice", "battleye", "helper",
        "updater", "uploader", "cefprocess", "webhelper", "dotnet", "ue4prereq", "prereq",
        "touchup", "cleanup",
    ];
    let mut v: Vec<&(String, u64)> = files
        .iter()
        .filter(|(name, _)| {
            let n = name.to_ascii_lowercase();
            n.ends_with(".exe") && !SKIP.iter().any(|s| n.contains(s))
        })
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut out: Vec<String> = Vec::new();
    for (name, _) in v {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(name)) {
            out.push(name.clone());
        }
        if out.len() == 3 {
            break;
        }
    }
    out
}

/// Collect (file name, size) of every .exe under `dir`, up to `depth` levels.
pub fn list_exes(dir: &Path, depth: usize) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    walk(dir, depth, &mut out);
    out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<(String, u64)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for ent in rd.flatten() {
        let Ok(ft) = ent.file_type() else { continue };
        if ft.is_dir() {
            if depth > 0 {
                walk(&ent.path(), depth - 1, out);
            }
        } else if ft.is_file() {
            let name = ent.file_name().to_string_lossy().to_string();
            if name.to_ascii_lowercase().ends_with(".exe") {
                let size = ent.metadata().map(|m| m.len()).unwrap_or(0);
                out.push((name, size));
            }
        }
        if out.len() > 2000 {
            return;
        }
    }
}

pub fn discover_steam(steam_root: &Path) -> Vec<Game> {
    let mut libs = vec![steam_root.to_path_buf()];
    let lf = steam_root.join("steamapps").join("libraryfolders.vdf");
    if let Ok(text) = std::fs::read_to_string(&lf) {
        for p in steam_library_paths(&text) {
            let p = PathBuf::from(p);
            if !libs.iter().any(|l| same_path(l, &p)) {
                libs.push(p);
            }
        }
    }
    let mut games = Vec::new();
    for lib in libs {
        let apps = lib.join("steamapps");
        let Ok(rd) = std::fs::read_dir(&apps) else { continue };
        let mut manifests: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let n = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                n.starts_with("appmanifest_") && n.ends_with(".acf")
            })
            .collect();
        manifests.sort();
        for m in manifests {
            let Ok(text) = std::fs::read_to_string(&m) else { continue };
            let Some((id, name, installdir)) = parse_acf(&text) else { continue };
            if is_steam_tool(&id, &name) {
                continue;
            }
            let dir = apps.join("common").join(&installdir);
            let exes = pick_exes(&list_exes(&dir, 3));
            games.push(Game {
                launcher: Launcher::Steam,
                name,
                id,
                install_dir: dir.to_string_lossy().to_string(),
                exe_candidates: exes,
            });
        }
    }
    games
}

pub fn discover_epic(manifest_dir: &Path) -> Vec<Game> {
    let Ok(rd) = std::fs::read_dir(manifest_dir) else { return vec![] };
    let mut paths: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("item")).unwrap_or(false))
        .collect();
    paths.sort();
    let mut games = Vec::new();
    for p in paths {
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        if let Some(mut g) = parse_epic_item(&text) {
            if g.exe_candidates.is_empty() {
                g.exe_candidates = pick_exes(&list_exes(Path::new(&g.install_dir), 3));
            }
            games.push(g);
        }
    }
    games
}

/// `drive_root` like "C:\". Reads .GamingRoot, then lists game folders.
pub fn discover_xbox(drive_root: &Path) -> Vec<Game> {
    let Ok(bytes) = std::fs::read(drive_root.join(".GamingRoot")) else { return vec![] };
    let mut games = Vec::new();
    for rel in parse_gaming_root(&bytes) {
        let rel = rel.trim_start_matches(['\\', '/']);
        let base = drive_root.join(rel);
        let Ok(rd) = std::fs::read_dir(&base) else { continue };
        let mut dirs: Vec<PathBuf> = rd.flatten().filter(|e| e.path().is_dir()).map(|e| e.path()).collect();
        dirs.sort();
        for d in dirs {
            let name = d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let content = d.join("Content");
            let search = if content.is_dir() { content } else { d.clone() };
            games.push(Game {
                launcher: Launcher::Xbox,
                id: name.clone(),
                name,
                install_dir: d.to_string_lossy().to_string(),
                exe_candidates: pick_exes(&list_exes(&search, 3)),
            });
        }
    }
    games
}

fn same_path(a: &Path, b: &Path) -> bool {
    let n = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase();
    n(a) == n(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rig-tune-lib-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn acf_and_tools() {
        let acf = r#""AppState" { "appid" "1091500" "name" "Cyberpunk 2077" "installdir" "Cyberpunk 2077" }"#;
        assert_eq!(
            parse_acf(acf),
            Some(("1091500".into(), "Cyberpunk 2077".into(), "Cyberpunk 2077".into()))
        );
        assert!(is_steam_tool("228980", "Steamworks Common Redistributables"));
        assert!(is_steam_tool("1", "Proton 9.0"));
        assert!(!is_steam_tool("1091500", "Cyberpunk 2077"));
    }

    #[test]
    fn epic_item() {
        let j = r#"{"DisplayName":"Fortnite","InstallLocation":"C:\\Program Files\\Epic Games\\Fortnite","AppName":"Fortnite","LaunchExecutable":"FortniteGame/Binaries/Win64/FortniteClient-Win64-Shipping.exe","bIsApplication":true}"#;
        let g = parse_epic_item(j).unwrap();
        assert_eq!(g.name, "Fortnite");
        assert_eq!(g.exe_candidates, vec!["FortniteClient-Win64-Shipping.exe".to_string()]);
        let dlc = r#"{"DisplayName":"X DLC","InstallLocation":"C:\\x","bIsApplication":false}"#;
        assert!(parse_epic_item(dlc).is_none());
        assert!(parse_epic_item("not json").is_none());
    }

    #[test]
    fn gaming_root_bytes() {
        // Bytes as found on real drives: RGBX 01000000 "XboxGames\0" (UTF-16LE)
        let mut b = b"RGBX".to_vec();
        b.extend(1u32.to_le_bytes());
        for u in "XboxGames".encode_utf16() {
            b.extend(u.to_le_bytes());
        }
        b.extend([0, 0]);
        assert_eq!(parse_gaming_root(&b), vec!["XboxGames".to_string()]);
        assert!(parse_gaming_root(b"nope").is_empty());
    }

    #[test]
    fn exe_ranking() {
        let files = vec![
            ("UnityCrashHandler64.exe".to_string(), 2_000_000),
            ("Game.exe".to_string(), 600_000),
            ("Launcher.exe".to_string(), 90_000_000),
            ("GameAssembly-Win64-Shipping.exe".to_string(), 80_000_000),
            ("unins000.exe".to_string(), 3_000_000),
            ("start_protected_game.exe".to_string(), 1_000),
        ];
        let p = pick_exes(&files);
        assert_eq!(p[0], "GameAssembly-Win64-Shipping.exe");
        assert_eq!(p[1], "Game.exe");
        assert_eq!(p.len(), 3);
    }

    #[test]
    fn discovers_steam_epic_xbox_from_folders() {
        // Steam
        let steam = tmp("steam");
        let lib2 = tmp("steamlib2");
        std::fs::create_dir_all(steam.join("steamapps/common/Half-Life 2")).unwrap();
        std::fs::write(steam.join("steamapps/common/Half-Life 2/hl2.exe"), vec![0u8; 100]).unwrap();
        std::fs::write(
            steam.join("steamapps/appmanifest_220.acf"),
            r#""AppState" { "appid" "220" "name" "Half-Life 2" "installdir" "Half-Life 2" }"#,
        )
        .unwrap();
        std::fs::write(
            steam.join("steamapps/appmanifest_228980.acf"),
            r#""AppState" { "appid" "228980" "name" "Steamworks Common Redistributables" "installdir" "Steamworks Shared" }"#,
        )
        .unwrap();
        std::fs::create_dir_all(lib2.join("steamapps/common/Portal 2")).unwrap();
        std::fs::write(lib2.join("steamapps/common/Portal 2/portal2.exe"), vec![0u8; 10]).unwrap();
        std::fs::write(
            lib2.join("steamapps/appmanifest_620.acf"),
            r#""AppState" { "appid" "620" "name" "Portal 2" "installdir" "Portal 2" }"#,
        )
        .unwrap();
        let vdf = format!(
            "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} }}",
            steam.to_string_lossy().replace('\\', "\\\\"),
            lib2.to_string_lossy().replace('\\', "\\\\")
        );
        std::fs::write(steam.join("steamapps/libraryfolders.vdf"), vdf).unwrap();
        let g = discover_steam(&steam);
        let names: Vec<&str> = g.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, vec!["Half-Life 2", "Portal 2"]);
        assert_eq!(g[0].exe_candidates, vec!["hl2.exe".to_string()]);

        // Epic
        let epic = tmp("epic");
        std::fs::write(
            epic.join("A1.item"),
            r#"{"DisplayName":"Alan Wake 2","InstallLocation":"E:\\AW2","AppName":"Dahlia","LaunchExecutable":"AlanWake2.exe"}"#,
        )
        .unwrap();
        let e = discover_epic(&epic);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].exe_candidates, vec!["AlanWake2.exe".to_string()]);

        // Xbox
        let drive = tmp("xbox");
        let mut b = b"RGBX".to_vec();
        b.extend(1u32.to_le_bytes());
        for u in "XboxGames".encode_utf16() {
            b.extend(u.to_le_bytes());
        }
        b.extend([0, 0]);
        std::fs::write(drive.join(".GamingRoot"), b).unwrap();
        std::fs::create_dir_all(drive.join("XboxGames/Forza Horizon 5/Content")).unwrap();
        std::fs::write(drive.join("XboxGames/Forza Horizon 5/Content/ForzaHorizon5.exe"), vec![0u8; 5]).unwrap();
        let x = discover_xbox(&drive);
        assert_eq!(x.len(), 1);
        assert_eq!(x[0].name, "Forza Horizon 5");
        assert_eq!(x[0].exe_candidates, vec!["ForzaHorizon5.exe".to_string()]);

        for d in [steam, lib2, epic, drive] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}
