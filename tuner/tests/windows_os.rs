//! Windows-only checks against the real OS (run in CI on windows-latest).
//! They only touch a scratch key, HKCU\Software\AIEN-Rig-Test, and remove it.
#![cfg(windows)]

use aien_rig_tuner::undo::{self, ApplyOutcome, Hive, LogSink, Setting, Store, UndoLog, UndoOutcome, UndoSelect, Value};
use aien_rig_tuner::win::{display, gpu, power, store::WinStore};

const TEST_KEY: &str = r"Software\AIEN-Rig-Test";

struct NoSink;
impl LogSink for NoSink {
    fn save(&mut self, _log: &UndoLog) -> anyhow::Result<()> {
        Ok(())
    }
}

#[test]
fn registry_apply_and_undo_restore_exactly() {
    let _ = windows_registry::CURRENT_USER.remove_tree(TEST_KEY);
    let mut store = WinStore;
    let dword = Setting::RegDword { hive: Hive::CurrentUser, path: TEST_KEY.into(), name: "Number".into() };
    let text = Setting::RegString { hive: Hive::CurrentUser, path: TEST_KEY.into(), name: "Text".into() };
    assert_eq!(store.read(&dword).unwrap(), Value::Missing);

    // A value that already exists...
    windows_registry::CURRENT_USER.create(TEST_KEY).unwrap().set_string("Text", "A=1;").unwrap();
    let mut log = UndoLog::default();
    let mut sink = NoSink;
    let a = undo::apply_change(&mut log, &mut sink, &mut store, "t1", &dword, &Value::Dword(2), false, "now").unwrap();
    assert!(matches!(a, ApplyOutcome::Applied(_)));
    let b = undo::apply_change(&mut log, &mut sink, &mut store, "t2", &text, &Value::Str("A=1;B=2;".into()), false, "now").unwrap();
    assert!(matches!(b, ApplyOutcome::Applied(_)));
    assert_eq!(store.read(&dword).unwrap(), Value::Dword(2));

    let out = undo::undo(&mut log, &mut sink, &mut store, &UndoSelect::All, false, "later").unwrap();
    assert!(out.iter().all(|o| matches!(o, UndoOutcome::Restored { .. })), "{out:?}");
    // ...goes back to its old text, and a value that did not exist is gone again.
    assert_eq!(store.read(&text).unwrap(), Value::Str("A=1;".into()));
    assert_eq!(store.read(&dword).unwrap(), Value::Missing);
    windows_registry::CURRENT_USER.remove_tree(TEST_KEY).unwrap();
}

#[test]
fn power_plan_guid_round_trip() {
    let g = "8c5e7fda-e8bf-4a96-9a85-cf3a7d2d6bfa";
    let parsed = power::parse_guid(g).unwrap();
    assert_eq!(power::guid_to_string(&parsed), g);
    let active = power::active_scheme().unwrap();
    assert_eq!(active.len(), 36, "{active}");
}

#[test]
fn hardware_reads_do_not_panic() {
    let gpus = gpu::scan_gpus();
    let displays = display::list();
    println!("gpus: {gpus:?}");
    println!("displays: {displays:?}");
}
