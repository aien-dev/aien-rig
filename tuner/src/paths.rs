//! Where rig-tune keeps its files.
//! Windows: %LOCALAPPDATA%\AIEN-Rig. Override with AIEN_RIG_HOME (tests, CI).

use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("AIEN_RIG_HOME") {
        return PathBuf::from(p);
    }
    if let Some(p) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(p).join("AIEN-Rig");
    }
    if let Some(h) = std::env::var_os("HOME") {
        return PathBuf::from(h).join(".local").join("share").join("aien-rig");
    }
    PathBuf::from("AIEN-Rig")
}

pub fn receipts_dir() -> PathBuf {
    data_dir().join("receipts")
}

pub fn undo_log() -> PathBuf {
    data_dir().join("undo-log.json")
}

pub fn game_mode_state() -> PathBuf {
    data_dir().join("game-mode.json")
}
