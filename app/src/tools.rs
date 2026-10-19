//! Finding and running the sibling programs (rig-tune.exe, rig-create.exe),
//! and keeping a receipt of every run.

use crate::config::{iso_utc, stamp, unix_now};
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const TUNE: &str = "rig-tune";
pub const CREATE: &str = "rig-create";

/// Hides the black console window that Windows opens for child programs.
pub fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Looks next to aien-rig.exe first (that is how the release zip is laid
/// out), then in `extra_dir`, then on PATH.
pub fn find_tool(name: &str, extra_dir: Option<&Path>) -> Option<PathBuf> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = extra_dir {
        dirs.push(d.to_path_buf());
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            dirs.push(d.to_path_buf());
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.into_iter().map(|d| d.join(&file)).find(|p| p.is_file())
}

#[derive(Debug, Clone)]
pub struct RunResult {
    pub program: String,
    pub args: Vec<String>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub seconds: f64,
    pub stdout: String,
    pub stderr: String,
}

impl RunResult {
    /// The tool's stdout as JSON, when it printed JSON.
    pub fn parsed(&self) -> Option<Value> {
        let t = self.stdout.trim();
        if t.starts_with('{') || t.starts_with('[') {
            serde_json::from_str(t).ok()
        } else {
            None
        }
    }
    pub fn ok(&self) -> bool {
        self.exit_code == Some(0) && !self.timed_out
    }
}

fn drain<R: Read + Send + 'static>(r: Option<R>) -> std::sync::mpsc::Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut s = Vec::new();
        if let Some(mut r) = r {
            let _ = r.read_to_end(&mut s);
        }
        let _ = tx.send(String::from_utf8_lossy(&s).into_owned());
    });
    rx
}

/// A grandchild that outlives a killed tool can keep the pipe open; do not
/// wait for it forever.
fn collect(rx: std::sync::mpsc::Receiver<String>, timed_out: bool) -> String {
    let wait = if timed_out { Duration::from_secs(2) } else { Duration::from_secs(30) };
    rx.recv_timeout(wait).unwrap_or_default()
}

/// Runs a program to the end or until `timeout`, capturing both outputs.
pub fn run(program: &Path, args: &[String], timeout: Duration) -> std::io::Result<RunResult> {
    let start = Instant::now();
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    no_window(&mut cmd);
    let mut child = cmd.spawn()?;
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let mut timed_out = false;
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break Some(st);
        }
        if start.elapsed() >= timeout {
            timed_out = true;
            let _ = child.kill();
            break child.wait().ok();
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let seconds = start.elapsed().as_secs_f64();
    Ok(RunResult {
        program: program.display().to_string(),
        args: args.to_vec(),
        exit_code: status.and_then(|s| s.code()),
        timed_out,
        seconds,
        stdout: collect(out, timed_out),
        stderr: collect(err, timed_out),
    })
}

/// Replaces `{game}` / `{file}` placeholders in an argument template.
pub fn fill(template: &[String], key: &str, value: &str) -> Vec<String> {
    let pat = format!("{{{key}}}");
    template.iter().map(|a| a.replace(&pat, value)).collect()
}

/// Writes `<data>/receipts/<stamp>-<tool>-<action>.json` and returns its name.
pub fn write_receipt(data_dir: &Path, tool: &str, action: &str, r: &RunResult) -> std::io::Result<String> {
    let dir = data_dir.join("receipts");
    std::fs::create_dir_all(&dir)?;
    let now = unix_now();
    let safe = |s: &str| -> String {
        s.chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
            .collect()
    };
    let base = format!("{}-{}-{}", stamp(now), safe(tool), safe(action));
    let mut name = format!("{base}.json");
    let mut n = 1;
    while dir.join(&name).exists() {
        n += 1;
        name = format!("{base}-{n}.json");
    }
    let body = json!({
        "receipt": "aien-rig tool run",
        "app_version": env!("CARGO_PKG_VERSION"),
        "time_utc": iso_utc(now),
        "tool": tool,
        "action": action,
        "program": r.program,
        "args": r.args,
        "exit_code": r.exit_code,
        "timed_out": r.timed_out,
        "seconds": (r.seconds * 1000.0).round() / 1000.0,
        "stdout_json": r.parsed(),
        "stdout": r.stdout,
        "stderr": r.stderr,
    });
    std::fs::write(dir.join(&name), serde_json::to_vec_pretty(&body)?)?;
    Ok(name)
}

/// Newest first.
pub fn list_receipts(data_dir: &Path, limit: usize) -> Vec<Value> {
    let dir = data_dir.join("receipts");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map(|it| {
            it.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".json"))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names.reverse();
    names
        .into_iter()
        .take(limit)
        .map(|n| {
            let summary = std::fs::read(dir.join(&n))
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .map(|v| {
                    json!({
                        "tool": v["tool"], "action": v["action"],
                        "time_utc": v["time_utc"], "exit_code": v["exit_code"],
                    })
                })
                .unwrap_or(Value::Null);
            json!({ "name": n, "summary": summary })
        })
        .collect()
}

/// Only plain file names inside the receipts folder can be read.
pub fn read_receipt(data_dir: &Path, name: &str) -> Option<Value> {
    let ok = !name.is_empty()
        && name.ends_with(".json")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        && !name.contains("..");
    if !ok {
        return None;
    }
    let bytes = std::fs::read(data_dir.join("receipts").join(name)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "aien-rig-{tag}-{}-{}",
            std::process::id(),
            unix_now()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn fills_placeholders() {
        let t = vec!["measure".to_string(), "{game}".into(), "--json".into()];
        assert_eq!(fill(&t, "game", "cs2"), vec!["measure", "cs2", "--json"]);
    }

    #[test]
    fn receipts_round_trip_and_reject_traversal() {
        let d = temp_dir("receipts");
        let r = RunResult {
            program: "x".into(),
            args: vec!["scan".into()],
            exit_code: Some(0),
            timed_out: false,
            seconds: 0.5,
            stdout: "{\"games\":[]}".into(),
            stderr: String::new(),
        };
        let a = write_receipt(&d, "rig-tune", "scan", &r).unwrap();
        let b = write_receipt(&d, "rig-tune", "scan", &r).unwrap();
        assert_ne!(a, b);
        let list = list_receipts(&d, 10);
        assert_eq!(list.len(), 2);
        let v = read_receipt(&d, &a).unwrap();
        assert_eq!(v["stdout_json"]["games"], json!([]));
        assert!(read_receipt(&d, "../config.json").is_none());
        assert!(read_receipt(&d, "..\\x.json").is_none());
        let _ = std::fs::remove_dir_all(d);
    }

    #[cfg(unix)]
    #[test]
    fn runs_with_timeout() {
        let sh = Path::new("/bin/sh");
        let r = run(sh, &["-c".into(), "echo hi; echo err >&2; exit 3".into()], Duration::from_secs(10)).unwrap();
        assert_eq!(r.exit_code, Some(3));
        assert_eq!(r.stdout.trim(), "hi");
        assert_eq!(r.stderr.trim(), "err");
        let r = run(sh, &["-c".into(), "exec sleep 5".into()], Duration::from_millis(300)).unwrap();
        assert!(r.timed_out);
        assert!(r.seconds < 4.0);
    }
}
