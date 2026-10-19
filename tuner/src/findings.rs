//! What `scan` reads, and the plain-English verdict for each item.
//! The Windows layer fills `ScanFacts`; `evaluate` is pure and tested here.

use crate::dxsettings;
use crate::library::Game;
use serde::{Deserialize, Serialize};

pub const POWER_BALANCED: &str = "381b4222-f694-41f0-9685-ff5bb260df2e";
pub const POWER_HIGH: &str = "8c5e7fda-e8bf-4a96-9a85-cf3a7d2d6bfa";
pub const POWER_SAVER: &str = "a1841308-3bce-4d2e-b8c3-6b0e5ee2d0d5";
pub const POWER_ULTIMATE: &str = "e9a42b02-d5df-448d-aa00-03f14749eb61";

pub fn power_plan_name(guid: &str) -> &'static str {
    match guid.to_ascii_lowercase().as_str() {
        POWER_BALANCED => "Balanced",
        POWER_HIGH => "High performance",
        POWER_SAVER => "Power saver",
        POWER_ULTIMATE => "Ultimate Performance",
        _ => "a custom plan",
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NvmlFacts {
    pub driver_version: Option<String>,
    pub bar1_total_mb: Option<u64>,
    pub pcie_gen_current: Option<u32>,
    pub pcie_gen_max: Option<u32>,
    pub pcie_width_current: Option<u32>,
    pub pcie_width_max: Option<u32>,
    pub temperature_c: Option<u32>,
    pub gpu_util_pct: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GpuFacts {
    pub name: String,
    pub vendor_id: u32,
    pub dedicated_vram_mb: u64,
    /// Driver version as Windows reports it (DXGI user-mode driver version).
    pub dxgi_driver_version: Option<String>,
    pub nvml: Option<NvmlFacts>,
}

impl GpuFacts {
    pub fn is_nvidia(&self) -> bool {
        self.vendor_id == 0x10de
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DisplayFacts {
    /// \\.\DISPLAY1
    pub device: String,
    /// Graphics adapter driving this output, e.g. "NVIDIA GeForce RTX 5070".
    pub adapter: String,
    pub primary: bool,
    pub width: u32,
    pub height: u32,
    pub hz: u32,
    pub bits: u32,
    /// Highest refresh rate this output offers at the current resolution.
    pub max_hz_at_current_res: u32,
    /// Highest refresh rate at any resolution, with that resolution.
    pub max_hz_any: Option<(u32, u32, u32)>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProcessHog {
    pub name: String,
    pub pid: u32,
    /// Share of the whole CPU (all cores) over the sample window, 0-100.
    pub cpu_pct: f64,
    pub gpu_mem_mb: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScanFacts {
    pub windows_build: Option<u32>,
    pub gpus: Vec<GpuFacts>,
    /// HKLM\SYSTEM\CurrentControlSet\Control\GraphicsDrivers\HwSchMode (2 = on).
    pub hags: Option<u32>,
    /// HKCU\Software\Microsoft\GameBar\AutoGameModeEnabled (missing = on).
    pub game_mode: Option<u32>,
    /// HKCU\System\GameConfigStore\GameDVR_Enabled
    pub game_dvr_enabled: Option<u32>,
    /// HKCU\Software\Microsoft\Windows\CurrentVersion\GameDVR\AppCaptureEnabled
    pub app_capture: Option<u32>,
    /// ...\GameDVR\HistoricalCaptureEnabled ("Record what happened").
    pub background_recording: Option<u32>,
    pub power_scheme: Option<String>,
    /// Whether the High performance plan exists on this PC (None = unknown).
    #[serde(default)]
    pub power_high_available: Option<bool>,
    pub dx_global_settings: Option<String>,
    pub displays: Vec<DisplayFacts>,
    pub hogs: Vec<ProcessHog>,
    pub games: Vec<Game>,
    pub aien_server_running: bool,
    /// Things we could not read, in plain English.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Good,
    Tip,
    Problem,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub area: String,
    pub status: Status,
    pub what: String,
    pub fix: String,
    /// If `rig-tune apply` can make this change for you, its id.
    pub tweak: Option<String>,
}

fn f(area: &str, status: Status, what: String, fix: &str, tweak: Option<&str>) -> Finding {
    Finding {
        area: area.into(),
        status,
        what,
        fix: fix.into(),
        tweak: tweak.map(|t| t.into()),
    }
}

/// Processes that are part of Windows or are needed to show frames at all.
pub fn is_system_process(name: &str) -> bool {
    const SYS: [&str; 16] = [
        "system", "idle", "system idle process", "registry", "memory compression", "smss.exe",
        "csrss.exe", "wininit.exe", "services.exe", "lsass.exe", "svchost.exe", "dwm.exe",
        "explorer.exe", "fontdrvhost.exe", "rig-tune.exe", "secure system",
    ];
    let n = name.to_ascii_lowercase();
    SYS.contains(&n.as_str())
}

pub fn evaluate(s: &ScanFacts) -> Vec<Finding> {
    let mut out = Vec::new();
    let nvidia = s.gpus.iter().find(|g| g.is_nvidia());

    // GPU and driver
    match nvidia {
        Some(g) => {
            let drv = g
                .nvml
                .as_ref()
                .and_then(|n| n.driver_version.clone())
                .or_else(|| g.dxgi_driver_version.clone())
                .unwrap_or_else(|| "unknown".into());
            out.push(f(
                "Graphics card",
                Status::Good,
                format!("{} with {} MB of video memory, driver {}.", g.name, g.dedicated_vram_mb, drv),
                "Keep the driver current with the NVIDIA app (Drivers tab). No clean install needed.",
                None,
            ));
        }
        None => out.push(f(
            "Graphics card",
            Status::Unknown,
            format!(
                "No NVIDIA graphics card found. Adapters seen: {}.",
                if s.gpus.is_empty() {
                    "none".to_string()
                } else {
                    s.gpus.iter().map(|g| g.name.clone()).collect::<Vec<_>>().join(", ")
                }
            ),
            "If an RTX card is installed, reinstall the NVIDIA driver from nvidia.com and run scan again.",
            None,
        )),
    }

    // Resizable BAR and PCIe link (NVML)
    if let Some(n) = nvidia.and_then(|g| g.nvml.as_ref()) {
        match n.bar1_total_mb {
            Some(mb) if mb > 256 => out.push(f(
                "Resizable BAR",
                Status::Good,
                format!("On (the CPU can reach {mb} MB of video memory at once)."),
                "Nothing to do.",
                None,
            )),
            Some(mb) => out.push(f(
                "Resizable BAR",
                Status::Problem,
                format!("Off (the CPU window into video memory is only {mb} MB)."),
                "Turn on \"Above 4G Decoding\" and \"Re-Size BAR Support\" in the motherboard BIOS. rig-tune never changes the BIOS; do this yourself or with help, then run scan again.",
                None,
            )),
            None => {}
        }
        if let (Some(cur), Some(max)) = (n.pcie_width_current, n.pcie_width_max) {
            if cur < max {
                out.push(f(
                    "PCIe slot",
                    Status::Problem,
                    format!("The card runs at x{cur} lanes but supports x{max}."),
                    "Make sure the card sits in the top full-length slot (usually the one closest to the CPU) and that no M.2 drive shares its lanes. Check the motherboard manual.",
                    None,
                ));
            } else {
                out.push(f(
                    "PCIe slot",
                    Status::Good,
                    format!(
                        "Full x{cur} link{}.",
                        n.pcie_gen_current.map(|g| format!(", currently PCIe gen {g} (it drops at idle to save power, that is normal)")).unwrap_or_default()
                    ),
                    "Nothing to do.",
                    None,
                ));
            }
        }
    } else if nvidia.is_some() {
        out.push(f(
            "Resizable BAR",
            Status::Unknown,
            "Could not read it (nvml.dll from the NVIDIA driver did not load).".into(),
            "Open the NVIDIA app or NVIDIA Control Panel > System Information and look for \"Resizable BAR: Yes\".",
            None,
        ));
    }

    // Monitors
    for d in &s.displays {
        let label = if d.primary { format!("{} (main screen)", d.device) } else { d.device.clone() };
        if nvidia.is_some() && !d.adapter.is_empty() && !d.adapter.to_ascii_lowercase().contains("nvidia") {
            out.push(f(
                "Monitor cable",
                Status::Problem,
                format!("{label} is driven by \"{}\", not the NVIDIA card.", d.adapter),
                "Plug the monitor cable into the ports on the graphics card (lower down on the back of the PC), not the motherboard ports. This can be the single biggest FPS fix.",
                None,
            ));
        }
        if d.max_hz_at_current_res > d.hz {
            out.push(f(
                "Refresh rate",
                Status::Problem,
                format!(
                    "{label} runs at {} Hz but supports {} Hz at {}x{}.",
                    d.hz, d.max_hz_at_current_res, d.width, d.height
                ),
                "Raise it to the maximum. rig-tune apply can do this (undo puts it back).",
                Some("refresh-rate-max"),
            ));
        } else {
            out.push(f(
                "Refresh rate",
                Status::Good,
                format!("{label} runs at its highest refresh rate: {} Hz at {}x{}.", d.hz, d.width, d.height),
                "Nothing to do.",
                None,
            ));
        }
        if let Some((w, h, hz)) = d.max_hz_any {
            if hz > d.max_hz_at_current_res && (w, h) != (d.width, d.height) {
                out.push(f(
                    "Refresh rate",
                    Status::Tip,
                    format!("{label} can do {hz} Hz, but only at {w}x{h}."),
                    "If the cable is HDMI, try DisplayPort (or an HDMI 2.1 cable): older cables cap the refresh rate at high resolutions.",
                    None,
                ));
            }
        }
    }
    if s.displays.is_empty() {
        out.push(f("Refresh rate", Status::Unknown, "Could not read any monitor.".into(), "Check Settings > System > Display > Advanced display.", None));
    }

    // VRR / G-SYNC: no public API to read it.
    out.push(f(
        "G-SYNC / VRR",
        Status::Unknown,
        "rig-tune cannot read the G-SYNC switch (NVIDIA keeps it private).".into(),
        "Open NVIDIA Control Panel > Display > Set up G-SYNC and tick \"Enable G-SYNC, G-SYNC Compatible\" for full screen and windowed mode. Also turn on Settings > System > Display > Graphics > Variable refresh rate.",
        None,
    ));

    // HAGS
    match s.hags {
        Some(2) => out.push(f("GPU scheduling", Status::Good, "Hardware-accelerated GPU scheduling is on.".into(), "Nothing to do.", None)),
        other => out.push(f(
            "GPU scheduling",
            Status::Problem,
            format!(
                "Hardware-accelerated GPU scheduling is {}.",
                if other == Some(1) { "off" } else { "not set (Windows default)" }
            ),
            "Turn it on. NVIDIA DLSS Frame Generation on RTX 40/50 cards needs it. rig-tune apply can do this; it takes effect after a restart.",
            Some("hags-on"),
        )),
    }

    // Game Mode
    if s.game_mode == Some(0) {
        out.push(f("Game Mode", Status::Tip, "Windows Game Mode is off.".into(),
            "Turn it on so Windows gives the game priority and holds back updates while you play. rig-tune apply can do this.", Some("game-mode-on")));
    } else {
        out.push(f("Game Mode", Status::Good, "Windows Game Mode is on.".into(), "Nothing to do.", None));
    }

    // Background recording
    if s.background_recording == Some(1) {
        out.push(f(
            "Background recording",
            Status::Problem,
            "Xbox Game Bar is recording gameplay in the background all the time (\"Record what happened\").".into(),
            "Turn it off unless you use it; it costs frames in every game. rig-tune apply can do this. You can still record clips by hand with Win+Alt+R.",
            Some("background-recording-off"),
        ));
    } else {
        out.push(f("Background recording", Status::Good, "Background recording is off.".into(), "Nothing to do.", None));
    }

    // Power plan
    let high_ok = s.power_high_available != Some(false);
    match s.power_scheme.as_deref() {
        Some(g) if g.eq_ignore_ascii_case(POWER_SAVER) => out.push(f(
            "Power plan",
            Status::Problem,
            "Windows is on the Power saver plan, which slows the CPU.".into(),
            if high_ok {
                "Switch to High performance. rig-tune apply can do this."
            } else {
                "Switch to Balanced (High performance is not offered on this PC). rig-tune apply can do this."
            },
            Some("power-plan-high"),
        )),
        Some(g) if g.eq_ignore_ascii_case(POWER_BALANCED) && !high_ok => out.push(f(
            "Power plan",
            Status::Good,
            "Windows is on Balanced; this PC does not offer the High performance plan.".into(),
            "Nothing to do. Settings > System > Power > Power mode \"Best performance\" is the closest switch if you want it.",
            None,
        )),
        Some(g) if g.eq_ignore_ascii_case(POWER_BALANCED) => out.push(f(
            "Power plan",
            Status::Tip,
            "Windows is on the Balanced plan.".into(),
            "High performance keeps CPU clocks up between frames. On many desktops the gain is small, so measure a game before and after. rig-tune apply can switch it.",
            Some("power-plan-high"),
        )),
        Some(g) => out.push(f("Power plan", Status::Good, format!("Windows is on {}.", power_plan_name(g)), "Nothing to do.", None)),
        None => out.push(f("Power plan", Status::Unknown, "Could not read the power plan.".into(), "Check Control Panel > Power Options.", None)),
    }

    // Windowed game optimizations
    let swap = s
        .dx_global_settings
        .as_deref()
        .and_then(|v| dxsettings::get(v, dxsettings::SWAP_EFFECT_UPGRADE));
    if swap.as_deref() == Some("1") {
        out.push(f("Windowed games", Status::Good, "\"Optimizations for windowed games\" is on.".into(), "Nothing to do.", None));
    } else {
        out.push(f(
            "Windowed games",
            Status::Tip,
            format!(
                "\"Optimizations for windowed games\" is {}.",
                if swap.as_deref() == Some("0") { "off" } else { "not set (Windows default)" }
            ),
            "Turn it on: lower input lag and working G-SYNC in borderless windowed games. rig-tune apply can do this.",
            Some("windowed-optimizations-on"),
        ));
    }

    // Background hogs
    let hogs: Vec<&ProcessHog> = s
        .hogs
        .iter()
        .filter(|h| !is_system_process(&h.name))
        .filter(|h| h.cpu_pct >= 5.0 || h.gpu_mem_mb.unwrap_or(0) >= 500)
        .collect();
    if hogs.is_empty() {
        out.push(f("Background apps", Status::Good, "Nothing in the background is using much CPU or video memory.".into(), "Nothing to do.", None));
    }
    for h in hogs {
        let mut parts = Vec::new();
        if h.cpu_pct >= 5.0 {
            parts.push(format!("{:.0}% of the CPU", h.cpu_pct));
        }
        if let Some(mb) = h.gpu_mem_mb.filter(|mb| *mb >= 500) {
            parts.push(format!("{mb} MB of video memory"));
        }
        out.push(f(
            "Background apps",
            Status::Tip,
            format!("{} (pid {}) is using {}.", h.name, h.pid, parts.join(" and ")),
            "Close it before playing if you do not need it.",
            None,
        ));
    }
    if s.aien_server_running {
        out.push(f(
            "AIEN Rig AI server",
            Status::Tip,
            "The AIEN Rig AI server is running.".into(),
            "Run `rig-tune game-mode on` before playing so it pauses; `rig-tune game-mode off` resumes it.",
            None,
        ));
    }

    out.push(f(
        "Games",
        if s.games.is_empty() { Status::Unknown } else { Status::Good },
        format!("Found {} installed games (Steam, Epic, Xbox).", s.games.len()),
        "Run `rig-tune games` to list them with the executable to measure.",
        None,
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rtx() -> GpuFacts {
        GpuFacts {
            name: "NVIDIA GeForce RTX 5070".into(),
            vendor_id: 0x10de,
            dedicated_vram_mb: 12227,
            dxgi_driver_version: Some("32.0.15.8180".into()),
            nvml: Some(NvmlFacts {
                driver_version: Some("581.80".into()),
                bar1_total_mb: Some(16384),
                pcie_width_current: Some(16),
                pcie_width_max: Some(16),
                pcie_gen_current: Some(1),
                pcie_gen_max: Some(5),
                ..Default::default()
            }),
        }
    }

    fn display(hz: u32, max: u32, adapter: &str) -> DisplayFacts {
        DisplayFacts {
            device: r"\\.\DISPLAY1".into(),
            adapter: adapter.into(),
            primary: true,
            width: 2560,
            height: 1440,
            hz,
            bits: 32,
            max_hz_at_current_res: max,
            max_hz_any: Some((2560, 1440, max)),
        }
    }

    fn find<'a>(v: &'a [Finding], area: &str) -> Vec<&'a Finding> {
        v.iter().filter(|x| x.area == area).collect()
    }

    #[test]
    fn healthy_pc_has_no_problems() {
        let s = ScanFacts {
            gpus: vec![rtx()],
            hags: Some(2),
            game_mode: None,
            background_recording: Some(0),
            power_scheme: Some(POWER_HIGH.into()),
            dx_global_settings: Some("SwapEffectUpgradeEnable=1;".into()),
            displays: vec![display(165, 165, "NVIDIA GeForce RTX 5070")],
            ..Default::default()
        };
        let v = evaluate(&s);
        assert!(v.iter().all(|x| x.status != Status::Problem), "{v:#?}");
        assert!(v.iter().all(|x| x.tweak.is_none()));
    }

    #[test]
    fn sixty_hz_on_a_165_hz_monitor_is_flagged() {
        let s = ScanFacts { gpus: vec![rtx()], displays: vec![display(60, 165, "NVIDIA GeForce RTX 5070")], ..Default::default() };
        let v = evaluate(&s);
        let r = find(&v, "Refresh rate");
        assert_eq!(r[0].status, Status::Problem);
        assert_eq!(r[0].tweak.as_deref(), Some("refresh-rate-max"));
        assert!(r[0].what.contains("60 Hz") && r[0].what.contains("165 Hz"));
    }

    #[test]
    fn monitor_on_motherboard_port_is_flagged() {
        let s = ScanFacts { gpus: vec![rtx()], displays: vec![display(144, 144, "Intel(R) UHD Graphics 770")], ..Default::default() };
        let v = evaluate(&s);
        assert_eq!(find(&v, "Monitor cable")[0].status, Status::Problem);
    }

    #[test]
    fn rebar_off_and_narrow_link() {
        let mut g = rtx();
        let n = g.nvml.as_mut().unwrap();
        n.bar1_total_mb = Some(256);
        n.pcie_width_current = Some(8);
        let v = evaluate(&ScanFacts { gpus: vec![g], ..Default::default() });
        let rb = find(&v, "Resizable BAR")[0];
        assert_eq!(rb.status, Status::Problem);
        assert!(rb.tweak.is_none(), "BIOS changes are never automated");
        assert_eq!(find(&v, "PCIe slot")[0].status, Status::Problem);
    }

    #[test]
    fn os_settings_map_to_tweaks() {
        let s = ScanFacts {
            gpus: vec![rtx()],
            hags: Some(1),
            game_mode: Some(0),
            background_recording: Some(1),
            power_scheme: Some(POWER_SAVER.into()),
            dx_global_settings: None,
            ..Default::default()
        };
        let v = evaluate(&s);
        let tweaks: Vec<&str> = v.iter().filter_map(|x| x.tweak.as_deref()).collect();
        for t in ["hags-on", "game-mode-on", "background-recording-off", "power-plan-high", "windowed-optimizations-on"] {
            assert!(tweaks.contains(&t), "missing {t}");
        }
    }

    #[test]
    fn hogs_skip_windows_itself() {
        let s = ScanFacts {
            hogs: vec![
                ProcessHog { name: "dwm.exe".into(), pid: 1, cpu_pct: 9.0, gpu_mem_mb: Some(800) },
                ProcessHog { name: "chrome.exe".into(), pid: 2, cpu_pct: 12.0, gpu_mem_mb: Some(900) },
                ProcessHog { name: "tiny.exe".into(), pid: 3, cpu_pct: 0.5, gpu_mem_mb: None },
            ],
            ..Default::default()
        };
        let v = evaluate(&s);
        let h = find(&v, "Background apps");
        assert_eq!(h.len(), 1);
        assert!(h[0].what.contains("chrome.exe") && h[0].what.contains("12%") && h[0].what.contains("900 MB"));
    }
}
