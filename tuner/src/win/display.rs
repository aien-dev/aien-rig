//! Monitors: current mode, best refresh rate, and changing the mode.

use super::{from_wide, wide};
use crate::findings::DisplayFacts;
use crate::undo::Value;
use anyhow::{bail, Result};
use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    ChangeDisplaySettingsExW, EnumDisplayDevicesW, EnumDisplaySettingsW, CDS_TEST, CDS_UPDATEREGISTRY,
    DEVMODEW, DISPLAY_DEVICEW, DISP_CHANGE_SUCCESSFUL, DM_BITSPERPEL, DM_DISPLAYFREQUENCY,
    DM_PELSHEIGHT, DM_PELSWIDTH, ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_MODE,
};

const ATTACHED_TO_DESKTOP: u32 = 0x1;
const PRIMARY_DEVICE: u32 = 0x4;

fn devmode() -> DEVMODEW {
    DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() }
}

pub fn current_mode(device: &str) -> Result<Value> {
    let name = wide(device);
    let mut dm = devmode();
    let ok = unsafe { EnumDisplaySettingsW(PCWSTR(name.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm) };
    if !ok.as_bool() {
        bail!("could not read the display mode of {device}");
    }
    Ok(Value::Display {
        width: dm.dmPelsWidth,
        height: dm.dmPelsHeight,
        hz: dm.dmDisplayFrequency,
        bits: dm.dmBitsPerPel,
    })
}

/// All modes the driver offers for this output: (width, height, hz, bits).
pub fn modes(device: &str) -> Vec<(u32, u32, u32, u32)> {
    let name = wide(device);
    let mut out = Vec::new();
    let mut i = 0u32;
    loop {
        let mut dm = devmode();
        let ok = unsafe { EnumDisplaySettingsW(PCWSTR(name.as_ptr()), ENUM_DISPLAY_SETTINGS_MODE(i), &mut dm) };
        if !ok.as_bool() || i > 10_000 {
            break;
        }
        out.push((dm.dmPelsWidth, dm.dmPelsHeight, dm.dmDisplayFrequency, dm.dmBitsPerPel));
        i += 1;
    }
    out
}

pub fn list() -> Vec<DisplayFacts> {
    let mut out = Vec::new();
    for i in 0..16u32 {
        let mut dd = DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
        let ok = unsafe { EnumDisplayDevicesW(PCWSTR::null(), i, &mut dd, 0) };
        if !ok.as_bool() {
            break;
        }
        if dd.StateFlags.0 & ATTACHED_TO_DESKTOP == 0 {
            continue;
        }
        let device = from_wide(&dd.DeviceName);
        let Ok(Value::Display { width, height, hz, bits }) = current_mode(&device) else { continue };
        let all = modes(&device);
        let max_here = all
            .iter()
            .filter(|m| m.0 == width && m.1 == height && m.3 == bits)
            .map(|m| m.2)
            .max()
            .unwrap_or(hz);
        let max_any = all.iter().filter(|m| m.3 == bits).max_by_key(|m| (m.2, m.0 * m.1)).map(|m| (m.0, m.1, m.2));
        out.push(DisplayFacts {
            device,
            adapter: from_wide(&dd.DeviceString),
            primary: dd.StateFlags.0 & PRIMARY_DEVICE != 0,
            width,
            height,
            hz,
            bits,
            max_hz_at_current_res: max_here,
            max_hz_any: max_any,
        });
    }
    out
}

/// Change the mode. Tests it with the driver first (CDS_TEST), and only
/// allows modes the driver itself lists for this output.
pub fn set_mode(device: &str, v: &Value) -> Result<()> {
    let Value::Display { width, height, hz, bits } = *v else { bail!("not a display mode: {v}") };
    if !modes(device).contains(&(width, height, hz, bits)) {
        bail!("{device} does not list {v} as a supported mode; refusing to set it");
    }
    let name = wide(device);
    let mut dm = devmode();
    dm.dmPelsWidth = width;
    dm.dmPelsHeight = height;
    dm.dmDisplayFrequency = hz;
    dm.dmBitsPerPel = bits;
    dm.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY | DM_BITSPERPEL;
    unsafe {
        let t = ChangeDisplaySettingsExW(PCWSTR(name.as_ptr()), Some(&dm), None, CDS_TEST, None);
        if t != DISP_CHANGE_SUCCESSFUL {
            bail!("the graphics driver rejected {v} for {device} in a test (code {})", t.0);
        }
        let r = ChangeDisplaySettingsExW(PCWSTR(name.as_ptr()), Some(&dm), None, CDS_UPDATEREGISTRY, None);
        if r != DISP_CHANGE_SUCCESSFUL {
            bail!("changing {device} to {v} failed (code {})", r.0);
        }
    }
    Ok(())
}
