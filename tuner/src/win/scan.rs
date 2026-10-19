//! Gather everything `scan` reports into `ScanFacts`.

use super::{display, gpu, power, procs, registry};
use crate::findings::ScanFacts;
use crate::gamemode::DEFAULT_SERVER_EXES;
use crate::library::{self, Game};
use crate::tweaks::{GAMEBAR_PATH, GAMEDVR_PATH, HAGS_PATH};
use crate::undo::Hive;
use crate::dxsettings;
use std::path::{Path, PathBuf};

pub fn windows_build() -> Option<u32> {
    registry::string_opt(Hive::LocalMachine, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "CurrentBuildNumber")
        .and_then(|s| s.trim().parse().ok())
}

pub fn steam_root() -> Option<PathBuf> {
    registry::string_opt(Hive::CurrentUser, r"Software\Valve\Steam", "SteamPath")
        .or_else(|| registry::string_opt(Hive::LocalMachine, r"SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"))
        .map(|s| PathBuf::from(s.replace('/', "\\")))
}

pub fn games() -> Vec<Game> {
    let mut out = Vec::new();
    if let Some(root) = steam_root() {
        out.extend(library::discover_steam(&root));
    }
    let program_data = std::env::var("PROGRAMDATA").unwrap_or_else(|_| r"C:\ProgramData".into());
    out.extend(library::discover_epic(
        &Path::new(&program_data).join(r"Epic\EpicGamesLauncher\Data\Manifests"),
    ));
    for letter in b'C'..=b'Z' {
        let root = format!("{}:\\", letter as char);
        let p = Path::new(&root);
        if p.join(".GamingRoot").exists() {
            out.extend(library::discover_xbox(p));
        }
    }
    out
}

pub fn facts(with_hogs: bool, with_games: bool) -> ScanFacts {
    let mut s = ScanFacts {
        windows_build: windows_build(),
        gpus: gpu::scan_gpus(),
        hags: registry::dword_opt(Hive::LocalMachine, HAGS_PATH, "HwSchMode"),
        game_mode: registry::dword_opt(Hive::CurrentUser, GAMEBAR_PATH, "AutoGameModeEnabled"),
        game_dvr_enabled: registry::dword_opt(Hive::CurrentUser, r"System\GameConfigStore", "GameDVR_Enabled"),
        app_capture: registry::dword_opt(Hive::CurrentUser, GAMEDVR_PATH, "AppCaptureEnabled"),
        background_recording: registry::dword_opt(Hive::CurrentUser, GAMEDVR_PATH, "HistoricalCaptureEnabled"),
        dx_global_settings: registry::string_opt(Hive::CurrentUser, dxsettings::KEY_PATH, dxsettings::VALUE_NAME),
        displays: display::list(),
        ..Default::default()
    };
    match power::active_scheme() {
        Ok(g) => s.power_scheme = Some(g),
        Err(e) => s.notes.push(format!("power plan: {e:#}")),
    }
    let running = procs::list();
    s.aien_server_running = running
        .iter()
        .any(|(_, n)| DEFAULT_SERVER_EXES.iter().any(|w| w.eq_ignore_ascii_case(n)));
    if with_hogs {
        s.hogs = procs::hogs(1500);
    }
    if with_games {
        s.games = games();
    }
    s
}
