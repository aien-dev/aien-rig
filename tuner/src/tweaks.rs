//! The complete list of changes `rig-tune apply` is allowed to make.
//!
//! Rules for anything added here:
//! - OS-level only. No overclocking, no voltage, no fan curves, no BIOS,
//!   no boot configuration (bcdedit), no driver files, no disabling services.
//! - Each change is one setting with a known old value, restored exactly by
//!   `rig-tune undo` (see undo.rs).
//! - Each change is asked about one at a time before it is made.

use crate::dxsettings;
use crate::findings::{ScanFacts, POWER_HIGH, POWER_SAVER, POWER_BALANCED};
use crate::undo::{Hive, Setting, Value};
use serde::Serialize;

pub const HAGS_PATH: &str = r"SYSTEM\CurrentControlSet\Control\GraphicsDrivers";
pub const GAMEBAR_PATH: &str = r"Software\Microsoft\GameBar";
pub const GAMEDVR_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\GameDVR";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Proposal {
    pub tweak: String,
    pub title: String,
    pub why: String,
    pub setting: Setting,
    pub new: Value,
    pub needs_admin: bool,
    pub needs_reboot: bool,
}

pub const ALL_TWEAKS: [&str; 6] = [
    "refresh-rate-max",
    "hags-on",
    "background-recording-off",
    "game-mode-on",
    "windowed-optimizations-on",
    "power-plan-high",
];

pub fn hags_setting() -> Setting {
    Setting::RegDword { hive: Hive::LocalMachine, path: HAGS_PATH.into(), name: "HwSchMode".into() }
}
pub fn game_mode_setting() -> Setting {
    Setting::RegDword { hive: Hive::CurrentUser, path: GAMEBAR_PATH.into(), name: "AutoGameModeEnabled".into() }
}
pub fn background_recording_setting() -> Setting {
    Setting::RegDword { hive: Hive::CurrentUser, path: GAMEDVR_PATH.into(), name: "HistoricalCaptureEnabled".into() }
}
pub fn dx_global_setting() -> Setting {
    Setting::RegString { hive: Hive::CurrentUser, path: dxsettings::KEY_PATH.into(), name: dxsettings::VALUE_NAME.into() }
}

/// Build the list of proposed changes for this PC, biggest expected win first.
/// Only settings that are not already at the good value are proposed.
pub fn propose(s: &ScanFacts) -> Vec<Proposal> {
    let mut out = Vec::new();
    for d in &s.displays {
        if d.max_hz_at_current_res > d.hz {
            out.push(Proposal {
                tweak: "refresh-rate-max".into(),
                title: format!("Raise {} from {} Hz to {} Hz", d.device, d.hz, d.max_hz_at_current_res),
                why: "The game can only show as many frames per second as the monitor refreshes. Same resolution, only the refresh rate changes.".into(),
                setting: Setting::DisplayMode { device: d.device.clone() },
                new: Value::Display { width: d.width, height: d.height, hz: d.max_hz_at_current_res, bits: d.bits },
                needs_admin: false,
                needs_reboot: false,
            });
        }
    }
    if s.hags != Some(2) {
        out.push(Proposal {
            tweak: "hags-on".into(),
            title: "Turn on hardware-accelerated GPU scheduling".into(),
            why: "Lets the graphics card manage its own work queue. Required for DLSS Frame Generation. Takes effect after a restart.".into(),
            setting: hags_setting(),
            new: Value::Dword(2),
            needs_admin: true,
            needs_reboot: true,
        });
    }
    if s.background_recording == Some(1) {
        out.push(Proposal {
            tweak: "background-recording-off".into(),
            title: "Turn off Xbox Game Bar background recording".into(),
            why: "Constant background video capture costs frames in every game. Manual clips (Win+Alt+R) still work.".into(),
            setting: background_recording_setting(),
            new: Value::Dword(0),
            needs_admin: false,
            needs_reboot: false,
        });
    }
    if s.game_mode == Some(0) {
        out.push(Proposal {
            tweak: "game-mode-on".into(),
            title: "Turn on Windows Game Mode".into(),
            why: "Windows gives the game priority and holds back updates and notifications while you play.".into(),
            setting: game_mode_setting(),
            new: Value::Dword(1),
            needs_admin: false,
            needs_reboot: false,
        });
    }
    let current = s.dx_global_settings.clone().unwrap_or_default();
    if dxsettings::get(&current, dxsettings::SWAP_EFFECT_UPGRADE).as_deref() != Some("1") {
        out.push(Proposal {
            tweak: "windowed-optimizations-on".into(),
            title: "Turn on optimizations for windowed games".into(),
            why: "Borderless windowed games get lower input lag and working G-SYNC. Other graphics settings in the same registry value are kept.".into(),
            setting: dx_global_setting(),
            new: Value::Str(dxsettings::set(&current, dxsettings::SWAP_EFFECT_UPGRADE, "1")),
            needs_admin: false,
            needs_reboot: false,
        });
    }
    if let Some(p) = s.power_scheme.as_deref() {
        let high_ok = s.power_high_available != Some(false);
        let saver = p.eq_ignore_ascii_case(POWER_SAVER);
        if saver && !high_ok {
            out.push(Proposal {
                tweak: "power-plan-high".into(),
                title: "Switch the power plan from Power saver to Balanced".into(),
                why: "Power saver holds the CPU back. This PC does not offer High performance, so Balanced is the best plan available.".into(),
                setting: Setting::PowerScheme,
                new: Value::Guid(POWER_BALANCED.into()),
                needs_admin: false,
                needs_reboot: false,
            });
        } else if high_ok && (saver || p.eq_ignore_ascii_case(POWER_BALANCED)) {
            out.push(Proposal {
                tweak: "power-plan-high".into(),
                title: "Switch the power plan to High performance".into(),
                why: "Keeps CPU clocks up between frames. Big help if you were on Power saver; often small on Balanced, so measure before and after.".into(),
                setting: Setting::PowerScheme,
                new: Value::Guid(POWER_HIGH.into()),
                needs_admin: false,
                needs_reboot: false,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::findings::DisplayFacts;

    #[test]
    fn proposes_only_what_is_not_already_good() {
        let good = ScanFacts {
            hags: Some(2),
            game_mode: Some(1),
            background_recording: Some(0),
            power_scheme: Some(POWER_HIGH.into()),
            dx_global_settings: Some("SwapEffectUpgradeEnable=1;".into()),
            displays: vec![DisplayFacts { hz: 144, max_hz_at_current_res: 144, ..Default::default() }],
            ..Default::default()
        };
        assert!(propose(&good).is_empty());
    }

    #[test]
    fn refresh_rate_first_and_values_exact() {
        let s = ScanFacts {
            hags: Some(1),
            dx_global_settings: Some("VRROptimizeEnable=1;".into()),
            power_scheme: Some(POWER_BALANCED.into()),
            displays: vec![DisplayFacts {
                device: r"\\.\DISPLAY1".into(),
                width: 2560,
                height: 1440,
                hz: 60,
                bits: 32,
                max_hz_at_current_res: 165,
                ..Default::default()
            }],
            ..Default::default()
        };
        let p = propose(&s);
        assert_eq!(p[0].tweak, "refresh-rate-max");
        assert_eq!(p[0].new, Value::Display { width: 2560, height: 1440, hz: 165, bits: 32 });
        let w = p.iter().find(|x| x.tweak == "windowed-optimizations-on").unwrap();
        assert_eq!(w.new, Value::Str("VRROptimizeEnable=1;SwapEffectUpgradeEnable=1;".into()));
        let h = p.iter().find(|x| x.tweak == "hags-on").unwrap();
        assert!(h.needs_admin && h.needs_reboot);
        assert!(p.iter().all(|x| ALL_TWEAKS.contains(&x.tweak.as_str())));
    }

    #[test]
    fn power_plan_respects_what_the_pc_offers() {
        let mut s = ScanFacts { power_scheme: Some(POWER_BALANCED.into()), power_high_available: Some(false), ..Default::default() };
        assert!(!propose(&s).iter().any(|p| p.tweak == "power-plan-high"));
        s.power_scheme = Some(POWER_SAVER.into());
        let p = propose(&s);
        let pp = p.iter().find(|p| p.tweak == "power-plan-high").unwrap();
        assert_eq!(pp.new, Value::Guid(POWER_BALANCED.into()));
        s.power_high_available = Some(true);
        let p = propose(&s);
        assert_eq!(p.iter().find(|p| p.tweak == "power-plan-high").unwrap().new, Value::Guid(POWER_HIGH.into()));
    }
}
