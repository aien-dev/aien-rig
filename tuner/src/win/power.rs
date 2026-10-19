//! Active Windows power plan.

use anyhow::{bail, Result};
use windows::core::GUID;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::System::Power::{PowerGetActiveScheme, PowerReadFriendlyName, PowerSetActiveScheme};

pub fn guid_to_string(g: &GUID) -> String {
    format!("{g:?}").to_ascii_lowercase()
}

pub fn parse_guid(s: &str) -> Result<GUID> {
    let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() != 32 {
        bail!("not a GUID: {s}");
    }
    Ok(GUID::from_u128(u128::from_str_radix(&hex, 16)?))
}

pub fn active_scheme() -> Result<String> {
    unsafe {
        let mut p: *mut GUID = std::ptr::null_mut();
        let rc = PowerGetActiveScheme(None, &mut p);
        if rc.0 != 0 || p.is_null() {
            bail!("PowerGetActiveScheme failed ({})", rc.0);
        }
        let s = guid_to_string(&*p);
        let _ = LocalFree(Some(HLOCAL(p as _)));
        Ok(s)
    }
}

/// Does this PC offer the plan? Windows hides High performance on many
/// newer PCs (Modern Standby), and switching to a missing plan fails.
pub fn scheme_exists(guid: &str) -> bool {
    let Ok(g) = parse_guid(guid) else { return false };
    let mut size = 0u32;
    let rc = unsafe { PowerReadFriendlyName(None, Some(&g), None, None, None, &mut size) };
    rc.0 == 0 || rc.0 == 234 // ERROR_SUCCESS or ERROR_MORE_DATA
}

pub fn set_active_scheme(guid: &str) -> Result<()> {
    let g = parse_guid(guid)?;
    let rc = unsafe { PowerSetActiveScheme(None, Some(&g)) };
    if rc.0 != 0 {
        bail!("Windows refused to switch the power plan (error {}). The plan may be hidden on this PC.", rc.0);
    }
    Ok(())
}
