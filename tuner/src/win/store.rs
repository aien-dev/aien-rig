//! The real `Store`: reads and writes this PC's settings for the undo engine.

use super::{display, power, registry};
use crate::undo::{Setting, Store, Value};
use anyhow::{bail, Result};

pub struct WinStore;

impl Store for WinStore {
    fn read(&self, s: &Setting) -> Result<Value> {
        match s {
            Setting::RegDword { hive, path, name } => registry::read_dword(*hive, path, name),
            Setting::RegString { hive, path, name } => registry::read_string(*hive, path, name),
            Setting::PowerScheme => Ok(Value::Guid(power::active_scheme()?)),
            Setting::DisplayMode { device } => display::current_mode(device),
        }
    }

    fn write(&mut self, s: &Setting, v: &Value) -> Result<()> {
        match (s, v) {
            (Setting::RegDword { hive, path, name }, Value::Dword(_) | Value::Missing)
            | (Setting::RegString { hive, path, name }, Value::Str(_) | Value::Missing) => {
                registry::write(*hive, path, name, v)
            }
            (Setting::PowerScheme, Value::Guid(g)) => power::set_active_scheme(g),
            (Setting::DisplayMode { device }, Value::Display { .. }) => display::set_mode(device, v),
            _ => bail!("{v} does not fit {s}"),
        }
    }
}
