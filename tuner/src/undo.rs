//! The undo log and the apply/undo engine.
//!
//! Every change the tuner makes goes through `apply_change`, which:
//! 1. reads the current (old) value,
//! 2. writes a "pending" log entry with the old value to disk FIRST,
//! 3. makes the change and reads it back,
//! 4. marks the entry applied (or rolls back and drops it if the read-back
//!    does not match).
//!
//! So even if the PC loses power mid-change, the log already knows the old
//! value and `undo` can restore it. `undo` walks entries newest first and
//! refuses to overwrite a setting that someone changed after us, unless the
//! user passes --force. Pure logic over a `Store` trait; the Windows store
//! lives in `win::store`, and tests use an in-memory store.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Hive {
    #[serde(rename = "HKCU")]
    CurrentUser,
    #[serde(rename = "HKLM")]
    LocalMachine,
}

impl fmt::Display for Hive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Hive::CurrentUser => write!(f, "HKCU"),
            Hive::LocalMachine => write!(f, "HKLM"),
        }
    }
}

/// One OS setting the tuner can change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Setting {
    RegDword { hive: Hive, path: String, name: String },
    RegString { hive: Hive, path: String, name: String },
    /// The active Windows power plan (by GUID).
    PowerScheme,
    /// Display mode of one monitor (\\.\DISPLAY1 style device name).
    DisplayMode { device: String },
}

impl fmt::Display for Setting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Setting::RegDword { hive, path, name } | Setting::RegString { hive, path, name } => {
                write!(f, "{hive}\\{path}\\{name}")
            }
            Setting::PowerScheme => write!(f, "active power plan"),
            Setting::DisplayMode { device } => write!(f, "display mode of {device}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Value {
    /// The value did not exist. Undo deletes it again.
    Missing,
    Dword(u32),
    Str(String),
    /// Lowercase GUID text, for power plans.
    Guid(String),
    Display { width: u32, height: u32, hz: u32, bits: u32 },
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Missing => write!(f, "(not set)"),
            Value::Dword(v) => write!(f, "{v}"),
            Value::Str(s) => write!(f, "\"{s}\""),
            Value::Guid(g) => write!(f, "{g}"),
            Value::Display { width, height, hz, bits } => {
                write!(f, "{width}x{height} @ {hz} Hz ({bits}-bit)")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Pending,
    Applied,
    Undone,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    pub tweak: String,
    pub setting: Setting,
    pub old: Value,
    pub new: Value,
    pub state: State,
    pub applied_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undone_at: Option<String>,
    #[serde(default)]
    pub needs_reboot: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UndoLog {
    pub version: u32,
    pub entries: Vec<Entry>,
}

/// Reads and writes OS settings. Implemented for Windows in `win::store`.
pub trait Store {
    fn read(&self, s: &Setting) -> Result<Value>;
    fn write(&mut self, s: &Setting, v: &Value) -> Result<()>;
}

/// Something that persists the log. Lets tests count saves.
pub trait LogSink {
    fn save(&mut self, log: &UndoLog) -> Result<()>;
}

/// Saves to a JSON file, atomically (temp file then rename).
pub struct FileSink<'a>(pub &'a Path);

impl LogSink for FileSink<'_> {
    fn save(&mut self, log: &UndoLog) -> Result<()> {
        save_log(self.0, log)
    }
}

pub fn load_log(path: &Path) -> Result<UndoLog> {
    if !path.exists() {
        return Ok(UndoLog { version: 1, entries: vec![] });
    }
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let log: UndoLog = serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(log)
}

pub fn save_log(path: &Path, log: &UndoLog) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(log)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

impl UndoLog {
    fn next_id(&self) -> u64 {
        self.entries.iter().map(|e| e.id).max().unwrap_or(0) + 1
    }

    pub fn active(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.state != State::Undone)
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum ApplyOutcome {
    /// Already at the wanted value; nothing written, nothing logged.
    AlreadySet,
    Applied(Entry),
}

#[allow(clippy::too_many_arguments)]
pub fn apply_change(
    log: &mut UndoLog,
    sink: &mut dyn LogSink,
    store: &mut dyn Store,
    tweak: &str,
    setting: &Setting,
    new: &Value,
    needs_reboot: bool,
    now: &str,
) -> Result<ApplyOutcome> {
    if *new == Value::Missing {
        bail!("refusing to delete a setting as a tweak; tweaks only set values");
    }
    let old = store.read(setting).with_context(|| format!("reading {setting}"))?;
    if old == *new {
        return Ok(ApplyOutcome::AlreadySet);
    }
    let entry = Entry {
        id: log.next_id(),
        tweak: tweak.to_string(),
        setting: setting.clone(),
        old: old.clone(),
        new: new.clone(),
        state: State::Pending,
        applied_at: now.to_string(),
        undone_at: None,
        needs_reboot,
    };
    log.entries.push(entry);
    sink.save(log).context("saving undo log before the change")?;
    let idx = log.entries.len() - 1;

    let result = store
        .write(setting, new)
        .and_then(|_| store.read(setting))
        .and_then(|back| {
            if back == *new {
                Ok(())
            } else {
                Err(anyhow!("read back {back} after writing {new}"))
            }
        });
    match result {
        Ok(()) => {
            log.entries[idx].state = State::Applied;
            sink.save(log)?;
            Ok(ApplyOutcome::Applied(log.entries[idx].clone()))
        }
        Err(e) => {
            // Put the old value back, then forget the entry.
            let rollback = store.write(setting, &old);
            log.entries.pop();
            sink.save(log)?;
            match rollback {
                Ok(()) => Err(e.context(format!("change to {setting} failed and was rolled back"))),
                Err(r) => Err(e.context(format!(
                    "change to {setting} failed AND rollback failed ({r}); old value was {old}"
                ))),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum UndoSelect {
    All,
    Id(u64),
    Tweak(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum UndoOutcome {
    Restored { id: u64, setting: Setting, value: Value },
    /// Someone (the user, a game, Windows) changed the setting after us.
    /// We leave it alone unless forced.
    ChangedSince { id: u64, setting: Setting, ours: Value, now: Value },
    Failed { id: u64, setting: Setting, error: String },
}

/// Undo entries, newest first. Stops touching a setting once it has been
/// restored in this run (older entries for the same setting are restored in
/// order, which is correct because each older entry's `new` equals the next
/// entry's `old`).
pub fn undo(
    log: &mut UndoLog,
    sink: &mut dyn LogSink,
    store: &mut dyn Store,
    select: &UndoSelect,
    force: bool,
    now: &str,
) -> Result<Vec<UndoOutcome>> {
    let mut out = Vec::new();
    let order: Vec<usize> = (0..log.entries.len()).rev().collect();
    for i in order {
        let e = &log.entries[i];
        if e.state == State::Undone {
            continue;
        }
        let picked = match select {
            UndoSelect::All => true,
            UndoSelect::Id(id) => e.id == *id,
            UndoSelect::Tweak(t) => &e.tweak == t,
        };
        if !picked {
            continue;
        }
        let (id, setting, old, new, state) =
            (e.id, e.setting.clone(), e.old.clone(), e.new.clone(), e.state);
        let current = match store.read(&setting) {
            Ok(v) => v,
            Err(err) => {
                out.push(UndoOutcome::Failed { id, setting, error: format!("{err:#}") });
                continue;
            }
        };
        // A pending entry may or may not have been written; restoring the old
        // value is right either way.
        let ours = current == new || current == old || state == State::Pending;
        if !ours && !force {
            out.push(UndoOutcome::ChangedSince { id, setting, ours: new, now: current });
            continue;
        }
        if current != old {
            if let Err(err) = store.write(&setting, &old) {
                out.push(UndoOutcome::Failed { id, setting, error: format!("{err:#}") });
                continue;
            }
        }
        log.entries[i].state = State::Undone;
        log.entries[i].undone_at = Some(now.to_string());
        sink.save(log)?;
        out.push(UndoOutcome::Restored { id, setting, value: old });
    }
    Ok(out)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    pub struct MemStore {
        pub values: HashMap<String, Value>,
        pub fail_write: bool,
        pub lie_on_readback: bool,
    }

    impl Store for MemStore {
        fn read(&self, s: &Setting) -> Result<Value> {
            Ok(self.values.get(&s.to_string()).cloned().unwrap_or(Value::Missing))
        }
        fn write(&mut self, s: &Setting, v: &Value) -> Result<()> {
            if self.fail_write {
                bail!("access denied");
            }
            let v = if self.lie_on_readback { Value::Dword(12345) } else { v.clone() };
            if v == Value::Missing {
                self.values.remove(&s.to_string());
            } else {
                self.values.insert(s.to_string(), v);
            }
            Ok(())
        }
    }

    #[derive(Default)]
    pub struct MemSink {
        pub saves: Vec<UndoLog>,
    }
    impl LogSink for MemSink {
        fn save(&mut self, log: &UndoLog) -> Result<()> {
            self.saves.push(log.clone());
            Ok(())
        }
    }

    fn hags() -> Setting {
        Setting::RegDword {
            hive: Hive::LocalMachine,
            path: r"SYSTEM\CurrentControlSet\Control\GraphicsDrivers".into(),
            name: "HwSchMode".into(),
        }
    }
    fn dvr() -> Setting {
        Setting::RegDword {
            hive: Hive::CurrentUser,
            path: r"Software\Microsoft\Windows\CurrentVersion\GameDVR".into(),
            name: "HistoricalCaptureEnabled".into(),
        }
    }

    #[test]
    fn apply_then_undo_restores_exactly_including_missing() {
        let mut st = MemStore::default();
        st.values.insert(hags().to_string(), Value::Dword(1));
        let mut log = UndoLog::default();
        let mut sink = MemSink::default();
        let a = apply_change(&mut log, &mut sink, &mut st, "hags-on", &hags(), &Value::Dword(2), true, "t1").unwrap();
        assert!(matches!(a, ApplyOutcome::Applied(_)));
        let b = apply_change(&mut log, &mut sink, &mut st, "dvr-off", &dvr(), &Value::Dword(0), false, "t2").unwrap();
        assert!(matches!(b, ApplyOutcome::Applied(_)));
        // The pending entry hit disk before the change was made.
        assert_eq!(sink.saves[0].entries[0].state, State::Pending);
        assert_eq!(sink.saves[0].entries[0].old, Value::Dword(1));
        assert_eq!(st.read(&hags()).unwrap(), Value::Dword(2));

        let out = undo(&mut log, &mut sink, &mut st, &UndoSelect::All, false, "t3").unwrap();
        assert_eq!(out.len(), 2);
        // newest first
        assert!(matches!(&out[0], UndoOutcome::Restored { id: 2, .. }));
        assert_eq!(st.read(&hags()).unwrap(), Value::Dword(1));
        assert_eq!(st.read(&dvr()).unwrap(), Value::Missing);
        assert!(log.entries.iter().all(|e| e.state == State::Undone));
        // undo again is a no-op
        assert!(undo(&mut log, &mut sink, &mut st, &UndoSelect::All, false, "t4").unwrap().is_empty());
    }

    #[test]
    fn already_set_logs_nothing() {
        let mut st = MemStore::default();
        st.values.insert(hags().to_string(), Value::Dword(2));
        let mut log = UndoLog::default();
        let mut sink = MemSink::default();
        let a = apply_change(&mut log, &mut sink, &mut st, "hags-on", &hags(), &Value::Dword(2), true, "t").unwrap();
        assert_eq!(a, ApplyOutcome::AlreadySet);
        assert!(log.entries.is_empty());
        assert!(sink.saves.is_empty());
    }

    #[test]
    fn failed_write_is_rolled_back_and_not_logged() {
        let mut st = MemStore { fail_write: true, ..Default::default() };
        let mut log = UndoLog::default();
        let mut sink = MemSink::default();
        let r = apply_change(&mut log, &mut sink, &mut st, "x", &dvr(), &Value::Dword(0), false, "t");
        assert!(r.is_err());
        assert!(log.entries.is_empty());
    }

    #[test]
    fn bad_readback_rolls_back() {
        let mut st = MemStore { lie_on_readback: true, ..Default::default() };
        let mut log = UndoLog::default();
        let mut sink = MemSink::default();
        let r = apply_change(&mut log, &mut sink, &mut st, "x", &dvr(), &Value::Dword(0), false, "t");
        assert!(r.is_err());
        assert!(log.entries.is_empty());
    }

    #[test]
    fn undo_leaves_settings_changed_by_someone_else_unless_forced() {
        let mut st = MemStore::default();
        let mut log = UndoLog::default();
        let mut sink = MemSink::default();
        apply_change(&mut log, &mut sink, &mut st, "dvr-off", &dvr(), &Value::Dword(0), false, "t").unwrap();
        st.values.insert(dvr().to_string(), Value::Dword(7)); // user changed it
        let out = undo(&mut log, &mut sink, &mut st, &UndoSelect::All, false, "t").unwrap();
        assert!(matches!(out[0], UndoOutcome::ChangedSince { .. }));
        assert_eq!(st.read(&dvr()).unwrap(), Value::Dword(7));
        let out = undo(&mut log, &mut sink, &mut st, &UndoSelect::All, true, "t").unwrap();
        assert!(matches!(out[0], UndoOutcome::Restored { .. }));
        assert_eq!(st.read(&dvr()).unwrap(), Value::Missing);
    }

    #[test]
    fn stacked_changes_to_one_setting_unwind_in_order() {
        let mut st = MemStore::default();
        st.values.insert(dvr().to_string(), Value::Dword(1));
        let mut log = UndoLog::default();
        let mut sink = MemSink::default();
        apply_change(&mut log, &mut sink, &mut st, "a", &dvr(), &Value::Dword(2), false, "t").unwrap();
        apply_change(&mut log, &mut sink, &mut st, "b", &dvr(), &Value::Dword(3), false, "t").unwrap();
        undo(&mut log, &mut sink, &mut st, &UndoSelect::All, false, "t").unwrap();
        assert_eq!(st.read(&dvr()).unwrap(), Value::Dword(1));
    }

    #[test]
    fn undo_one_by_id_or_tweak() {
        let mut st = MemStore::default();
        let mut log = UndoLog::default();
        let mut sink = MemSink::default();
        apply_change(&mut log, &mut sink, &mut st, "hags-on", &hags(), &Value::Dword(2), true, "t").unwrap();
        apply_change(&mut log, &mut sink, &mut st, "dvr-off", &dvr(), &Value::Dword(0), false, "t").unwrap();
        let out = undo(&mut log, &mut sink, &mut st, &UndoSelect::Tweak("hags-on".into()), false, "t").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(st.read(&hags()).unwrap(), Value::Missing);
        assert_eq!(st.read(&dvr()).unwrap(), Value::Dword(0));
        let out = undo(&mut log, &mut sink, &mut st, &UndoSelect::Id(2), false, "t").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(st.read(&dvr()).unwrap(), Value::Missing);
    }

    #[test]
    fn pending_entry_from_a_crash_is_still_undone() {
        let mut st = MemStore::default();
        st.values.insert(dvr().to_string(), Value::Dword(9)); // half-applied unknown state
        let mut log = UndoLog {
            version: 1,
            entries: vec![Entry {
                id: 1,
                tweak: "dvr-off".into(),
                setting: dvr(),
                old: Value::Dword(1),
                new: Value::Dword(0),
                state: State::Pending,
                applied_at: "t".into(),
                undone_at: None,
                needs_reboot: false,
            }],
        };
        let mut sink = MemSink::default();
        let out = undo(&mut log, &mut sink, &mut st, &UndoSelect::All, false, "t").unwrap();
        assert!(matches!(out[0], UndoOutcome::Restored { .. }));
        assert_eq!(st.read(&dvr()).unwrap(), Value::Dword(1));
    }

    #[test]
    fn log_round_trips_through_json_file() {
        let dir = std::env::temp_dir().join(format!("rig-tune-test-{}", std::process::id()));
        let path = dir.join("undo-log.json");
        let mut st = MemStore::default();
        let mut log = load_log(&path).unwrap();
        let mut sink = FileSink(&path);
        apply_change(&mut log, &mut sink, &mut st, "power", &Setting::PowerScheme,
            &Value::Guid("8c5e7fda-e8bf-4a96-9a85-cf3a7d2d6bfa".into()), false, "t").unwrap();
        let back = load_log(&path).unwrap();
        assert_eq!(back, log);
        assert_eq!(back.entries[0].old, Value::Missing);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_to_delete_as_a_tweak() {
        let mut st = MemStore::default();
        let mut log = UndoLog::default();
        let mut sink = MemSink::default();
        assert!(apply_change(&mut log, &mut sink, &mut st, "x", &dvr(), &Value::Missing, false, "t").is_err());
    }
}
