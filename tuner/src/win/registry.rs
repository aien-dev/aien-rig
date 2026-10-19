//! Registry reads and writes for the tuner's settings.

use crate::undo::{Hive, Value};
use anyhow::{bail, Context, Result};
use windows_registry::{Key, CURRENT_USER, LOCAL_MACHINE};

const NOT_FOUND: i32 = 0x8007_0002u32 as i32; // HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)

fn root(h: Hive) -> &'static Key {
    match h {
        Hive::CurrentUser => CURRENT_USER,
        Hive::LocalMachine => LOCAL_MACHINE,
    }
}

fn is_missing(e: &windows::core::Error) -> bool {
    e.code().0 == NOT_FOUND
}

pub fn read_dword(h: Hive, path: &str, name: &str) -> Result<Value> {
    let key = match root(h).open(path) {
        Ok(k) => k,
        Err(e) if is_missing(&e) => return Ok(Value::Missing),
        Err(e) => return Err(e).with_context(|| format!("opening {h}\\{path}")),
    };
    match key.get_u32(name) {
        Ok(v) => Ok(Value::Dword(v)),
        Err(e) if is_missing(&e) => Ok(Value::Missing),
        Err(e) => Err(e).with_context(|| format!("reading {h}\\{path}\\{name} as a number")),
    }
}

pub fn read_string(h: Hive, path: &str, name: &str) -> Result<Value> {
    let key = match root(h).open(path) {
        Ok(k) => k,
        Err(e) if is_missing(&e) => return Ok(Value::Missing),
        Err(e) => return Err(e).with_context(|| format!("opening {h}\\{path}")),
    };
    match key.get_string(name) {
        Ok(v) => Ok(Value::Str(v)),
        Err(e) if is_missing(&e) => Ok(Value::Missing),
        Err(e) => Err(e).with_context(|| format!("reading {h}\\{path}\\{name} as text")),
    }
}

/// Convenience for scan: Some(n) or None when missing/unreadable.
pub fn dword_opt(h: Hive, path: &str, name: &str) -> Option<u32> {
    match read_dword(h, path, name) {
        Ok(Value::Dword(v)) => Some(v),
        _ => None,
    }
}

pub fn string_opt(h: Hive, path: &str, name: &str) -> Option<String> {
    match read_string(h, path, name) {
        Ok(Value::Str(v)) => Some(v),
        _ => None,
    }
}

pub fn write(h: Hive, path: &str, name: &str, v: &Value) -> Result<()> {
    match v {
        Value::Dword(n) => {
            let key = root(h).create(path).with_context(|| format!("opening {h}\\{path} for writing"))?;
            key.set_u32(name, *n).with_context(|| format!("writing {h}\\{path}\\{name}"))?;
        }
        Value::Str(s) => {
            let key = root(h).create(path).with_context(|| format!("opening {h}\\{path} for writing"))?;
            key.set_string(name, s).with_context(|| format!("writing {h}\\{path}\\{name}"))?;
        }
        Value::Missing => {
            let key = match root(h).options().read().write().open(path) {
                Ok(k) => k,
                Err(e) if is_missing(&e) => return Ok(()),
                Err(e) => return Err(e).with_context(|| format!("opening {h}\\{path} for writing")),
            };
            match key.remove_value(name) {
                Ok(()) => {}
                Err(e) if is_missing(&e) => {}
                Err(e) => return Err(e).with_context(|| format!("removing {h}\\{path}\\{name}")),
            }
        }
        other => bail!("{other} cannot be stored in the registry"),
    }
    Ok(())
}
