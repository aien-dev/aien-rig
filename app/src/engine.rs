//! Starting, stopping and checking the engine (aien-serve-http).
//!
//! On the brother's PC the engine runs inside WSL2 (docs/WINDOWS-GPU-PATH.md).
//! The app starts it with `wsl.exe`, records the Linux process id in
//! `~/.aien-rig/engine.pid` inside WSL, and stops exactly that process.

use crate::config::Config;
use crate::http;
use crate::tools::no_window;
use serde_json::{json, Value};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub const PID_FILE: &str = "~/.aien-rig/engine.pid";
pub const LOG_FILE: &str = "~/.aien-rig/engine.log";

#[derive(Default)]
pub struct Engine {
    child: Option<Child>,
    pub last_error: Option<String>,
}

/// Shell script run inside WSL (or by `sh` in "command" mode off Windows).
pub fn start_script(command: &str) -> String {
    format!(
        "mkdir -p ~/.aien-rig && echo $$ > {PID_FILE} && exec {command} >> {LOG_FILE} 2>&1"
    )
}

pub fn stop_script() -> String {
    format!(
        "if [ -f {PID_FILE} ]; then kill $(cat {PID_FILE}) 2>/dev/null; rm -f {PID_FILE}; fi"
    )
}

/// `wsl.exe [-d distro] -- bash -lc <script>`
pub fn wsl_args(distro: &str, script: &str) -> Vec<String> {
    let mut a = Vec::new();
    if !distro.is_empty() {
        a.push("-d".to_string());
        a.push(distro.to_string());
    }
    a.extend(["--", "bash", "-lc"].iter().map(|s| s.to_string()));
    a.push(script.to_string());
    a
}

/// "command" mode on Windows runs a native engine directly; the app then
/// stops it by ending that one process.
fn native_windows(cfg: &Config) -> bool {
    cfg!(windows) && cfg.engine_mode == "command"
}

fn shell_command(cfg: &Config, script: &str) -> Command {
    if cfg!(windows) {
        let mut c = Command::new("wsl.exe");
        c.args(wsl_args(&cfg.engine_distro, script));
        c
    } else {
        // Off Windows (development and tests) the same script runs under sh.
        let mut c = Command::new("sh");
        c.args(["-c", script]);
        c
    }
}

fn start_command(cfg: &Config) -> Command {
    if native_windows(cfg) {
        let mut parts = cfg.engine_command.split_whitespace();
        let mut c = Command::new(parts.next().unwrap_or("aien-serve-http.exe"));
        c.args(parts);
        c
    } else {
        shell_command(cfg, &start_script(&cfg.engine_command))
    }
}

/// What the engine reports at `/v1/models`, or None when it does not answer.
pub fn probe(url: &str) -> Option<Value> {
    let base = url.trim_end_matches('/');
    let r = http::call("GET", &format!("{base}/v1/models"), None, Duration::from_millis(1500)).ok()?;
    if r.status != 200 {
        return None;
    }
    serde_json::from_slice(&r.body).ok()
}

pub fn model_id(models: &Value) -> Option<String> {
    models["data"][0]["id"].as_str().map(|s| s.to_string())
}

impl Engine {
    pub fn managed_running(&mut self) -> bool {
        match self.child.as_mut() {
            Some(c) => match c.try_wait() {
                Ok(None) => true,
                _ => {
                    self.child = None;
                    false
                }
            },
            None => false,
        }
    }

    pub fn status(&mut self, cfg: &Config) -> Value {
        let models = probe(&cfg.engine_url);
        json!({
            "url": cfg.engine_url,
            "mode": cfg.engine_mode,
            "answering": models.is_some(),
            "model": models.as_ref().and_then(model_id),
            "started_by_app": self.managed_running(),
            "last_error": self.last_error,
        })
    }

    /// Starts the engine unless it already answers. Waits up to `wait` for it
    /// to answer.
    pub fn start(&mut self, cfg: &Config, wait: Duration) -> Result<String, String> {
        if probe(&cfg.engine_url).is_some() {
            return Ok("The engine is already running.".into());
        }
        if !self.managed_running() {
            let mut cmd = start_command(cfg);
            cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            no_window(&mut cmd);
            match cmd.spawn() {
                Ok(c) => self.child = Some(c),
                Err(e) => {
                    let msg = if cfg.engine_mode == "wsl" {
                        format!("Could not start WSL ({e}). Is WSL installed? See README-FIRST.txt.")
                    } else {
                        format!("Could not start the engine: {e}")
                    };
                    self.last_error = Some(msg.clone());
                    return Err(msg);
                }
            }
        }
        let t0 = Instant::now();
        while t0.elapsed() < wait {
            if probe(&cfg.engine_url).is_some() {
                self.last_error = None;
                return Ok("The engine is running.".into());
            }
            if !self.managed_running() {
                let msg = format!(
                    "The engine stopped right after starting. Its log is {LOG_FILE} inside WSL."
                );
                self.last_error = Some(msg.clone());
                return Err(msg);
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        let msg = "The engine was started but is not answering yet. It may still be loading the model.".to_string();
        self.last_error = Some(msg.clone());
        Err(msg)
    }

    /// Stops the engine this app started (by its pid file). Never touches
    /// other processes.
    pub fn stop(&mut self, cfg: &Config) -> Result<String, String> {
        if native_windows(cfg) {
            if let Some(mut c) = self.child.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
            return Ok("The engine is stopped.".into());
        }
        let mut cmd = shell_command(cfg, &stop_script());
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        no_window(&mut cmd);
        let stop_result = cmd
            .spawn()
            .and_then(|mut c| {
                let t0 = Instant::now();
                loop {
                    if c.try_wait()?.is_some() || t0.elapsed() > Duration::from_secs(15) {
                        let _ = c.kill();
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            });
        if let Some(mut c) = self.child.take() {
            let t0 = Instant::now();
            while t0.elapsed() < Duration::from_secs(10) {
                if let Ok(Some(_)) = c.try_wait() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let _ = c.kill();
            let _ = c.wait();
        }
        match stop_result {
            Ok(()) => Ok("The engine is stopped.".into()),
            Err(e) => {
                let msg = format!("Could not stop the engine: {e}");
                self.last_error = Some(msg.clone());
                Err(msg)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_and_args() {
        let s = start_script("aien-serve-http --engine omega");
        assert!(s.contains("echo $$ > ~/.aien-rig/engine.pid"));
        assert!(s.ends_with("exec aien-serve-http --engine omega >> ~/.aien-rig/engine.log 2>&1"));
        assert_eq!(
            wsl_args("Ubuntu-24.04", "x"),
            vec!["-d", "Ubuntu-24.04", "--", "bash", "-lc", "x"]
        );
        assert_eq!(wsl_args("", "x"), vec!["--", "bash", "-lc", "x"]);
        assert!(stop_script().contains("kill $(cat ~/.aien-rig/engine.pid)"));
    }

    #[test]
    fn model_id_from_models_reply() {
        let v = json!({"object":"list","data":[{"id":"qwen3-30b-a3b"}]});
        assert_eq!(model_id(&v).as_deref(), Some("qwen3-30b-a3b"));
        assert_eq!(model_id(&json!({})), None);
    }
}
