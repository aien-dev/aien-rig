//! Active Windows power plan.

use anyhow::{bail, Result};
use windows::core::GUID;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::System::Power::{PowerGetActiveScheme, PowerSetActiveScheme};

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

pub fn set_active_scheme(guid: &str) -> Result<()> {
    let g = parse_guid(guid)?;
    let rc = unsafe { PowerSetActiveScheme(None, Some(&g)) };
    if rc.0 != 0 {
        bail!("Windows refused to switch the power plan (error {}). The plan may be hidden on this PC.", rc.0);
    }
    Ok(())
}
