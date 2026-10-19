//! AIEN Rig FPS tuner.
//!
//! Everything that decides something (statistics, which frames count, what to
//! change, how to undo it) is pure Rust in the modules below and is unit
//! tested on every OS. The `win` module is the thin Windows layer: ETW frame
//! capture, registry, power plans, display modes, NVML and process control.

pub mod dxsettings;
pub mod findings;
pub mod frames;
pub mod gamemode;
pub mod library;
pub mod paths;
pub mod receipt;
pub mod stats;
pub mod timefmt;
pub mod tweaks;
pub mod undo;
pub mod vdf;

#[cfg(windows)]
pub mod win;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
