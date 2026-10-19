//! Windows layer: thin wrappers over the OS. Decisions live in the pure
//! modules; these files only read and write the machine.

pub mod display;
pub mod elevation;
pub mod etw;
pub mod gpu;
pub mod power;
pub mod present_probe;
pub mod procs;
pub mod registry;
pub mod scan;
pub mod store;

/// NUL-terminated UTF-16 for Win32 calls.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// UTF-16 buffer up to the first NUL.
pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
