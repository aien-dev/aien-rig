//! The local web page and its JSON API. Listens on 127.0.0.1 only.
//!
//! Two guards keep other web sites in the same browser out:
//! - the Host header must be 127.0.0.1:<port> or localhost:<port>
//!   (stops DNS rebinding);
//! - every POST must carry `X-AIEN-Rig: 1`, a header a foreign page cannot
//!   send without a CORS preflight that this server never approves.

use crate::config::{iso_utc, unix_now, Config};
use crate::engine::{self, Engine};
use crate::games::{self, Change};
use crate::http::{self, Request, Response};
use crate::tools::{self, no_window};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const INDEX_HTML: &str = include_str!("ui/index.html");

#[derive(Default)]
pub struct GameState {
    pub current: Option<String>,
    /// Engine was answering when the game started, so restart it after.
    pub engine_was_running: bool,
    /// Game Mode value before the app changed it (Some(None) = was unset).
    pub saved_game_mode: Option<Option<u32>>,
}

pub struct App {
    pub port: u16,
    pub data_dir: PathBuf,
    /// Extra place to look for rig-tune / rig-create (tests, development).
    pub tools_dir: Option<PathBuf>,
    pub cfg: Mutex<Config>,
    pub engine: Mutex<Engine>,
    pub game: Mutex<GameState>,
    pub events: Mutex<VecDeque<Value>>,
    pub tool_busy: Mutex<()>,
}

impl App {
    pub fn new(port: u16, data_dir: PathBuf, cfg: Config) -> App {
        App {
            port,
            data_dir,
            tools_dir: None,
            cfg: Mutex::new(cfg),
            engine: Mutex::new(Engine::default()),
            game: Mutex::new(GameState::default()),
            events: Mutex::new(VecDeque::new()),
            tool_busy: Mutex::new(()),
        }
    }

    pub fn config(&self) -> Config {
        self.cfg.lock().unwrap().clone()
    }

    pub fn config_path(&self) -> PathBuf {
        self.data_dir.join("config.json")
    }

    pub fn log(&self, text: impl Into<String>) {
        let text = text.into();
        println!("[{}] {}", iso_utc(unix_now()), text);
        let mut ev = self.events.lock().unwrap();
        ev.push_back(json!({ "time_utc": iso_utc(unix_now()), "text": text }));
        while ev.len() > 200 {
            ev.pop_front();
        }
    }
}

pub fn serve(listener: TcpListener, app: Arc<App>) {
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let app = app.clone();
                std::thread::spawn(move || handle(&app, s));
            }
            Err(_) => continue,
        }
    }
}

fn handle(app: &App, mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
    let resp = match http::read_request(&mut stream) {
        Ok(req) => route(app, &req),
        Err(e) => Response::error(400, &e.to_string()),
    };
    let _ = http::write_response(&mut stream, &resp);
}

pub fn host_allowed(host: Option<&str>, port: u16) -> bool {
    match host {
        Some(h) => h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}"),
        None => false,
    }
}

fn body_json(req: &Request) -> Result<Value, Response> {
    if req.body.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(&req.body).map_err(|e| Response::error(400, &format!("bad JSON: {e}")))
}

pub fn route(app: &App, req: &Request) -> Response {
    if !host_allowed(req.header("host"), app.port) {
        return Response::error(403, "This page only answers on 127.0.0.1.");
    }
    if req.method == "POST" && req.header("x-aien-rig") != Some("1") {
        return Response::error(403, "Missing X-AIEN-Rig header.");
    }
    let body = match body_json(req) {
        Ok(b) => b,
        Err(r) => return r,
    };
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => Response::html(INDEX_HTML),
        ("GET", "/api/status") => Response::json(200, &status(app)),
        ("GET", "/api/config") => Response::json(200, &json!(app.config())),
        ("POST", "/api/config") => set_config(app, &body),
        ("GET", "/api/receipts") => Response::json(
            200,
            &json!({ "ok": true, "folder": app.data_dir.join("receipts").display().to_string(),
                     "receipts": tools::list_receipts(&app.data_dir, 100) }),
        ),
        ("GET", "/api/receipt") => {
            let name = req.query_param("name").unwrap_or_default();
            match tools::read_receipt(&app.data_dir, &name) {
                Some(v) => Response::json(200, &json!({ "ok": true, "receipt": v })),
                None => Response::error(404, "No such receipt."),
            }
        }
        ("POST", "/api/tune") => tune(app, &body),
        ("POST", "/api/create") => create(app, &body),
        ("POST", "/api/pick-file") => pick_file(&body),
        ("POST", "/api/engine") => engine_action(app, &body),
        ("POST", "/api/chat") => chat(app, &body),
        (_, p) if p.starts_with("/api/") => Response::error(405, "Unknown action."),
        _ => Response::error(404, "Not found."),
    }
}

fn status(app: &App) -> Value {
    let cfg = app.config();
    // The engine lock is held while it starts (up to two minutes); answer anyway.
    let engine = match app.engine.try_lock() {
        Ok(mut e) => e.status(&cfg),
        Err(_) => json!({
            "url": cfg.engine_url, "mode": cfg.engine_mode, "answering": false,
            "busy": true, "last_error": "The engine is starting or stopping...",
        }),
    };
    let game = app.game.lock().unwrap();
    let dir = app.tools_dir.as_deref();
    json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "data_folder": app.data_dir.display().to_string(),
        "tools": {
            "rig-tune": tools::find_tool(tools::TUNE, dir).map(|p| p.display().to_string()),
            "rig-create": tools::find_tool(tools::CREATE, dir).map(|p| p.display().to_string()),
        },
        "engine": engine,
        "game": {
            "running": game.current,
            "auto_game_mode": cfg.auto_game_mode,
            "watching": cfg.games,
        },
        "events": app.events.lock().unwrap().iter().rev().take(30).cloned().collect::<Vec<_>>(),
    })
}

fn set_config(app: &App, body: &Value) -> Response {
    let mut merged = json!(app.config());
    if let (Some(dst), Some(src)) = (merged.as_object_mut(), body.as_object()) {
        for (k, v) in src {
            if dst.contains_key(k) {
                dst.insert(k.clone(), v.clone());
            }
        }
    }
    let cfg: Config = match serde_json::from_value(merged) {
        Ok(c) => c,
        Err(e) => return Response::error(400, &format!("bad settings: {e}")),
    };
    if let Err(e) = cfg.validate() {
        return Response::error(400, &e);
    }
    if let Err(e) = cfg.save(&app.config_path()) {
        return Response::error(500, &format!("could not save settings: {e}"));
    }
    *app.cfg.lock().unwrap() = cfg.clone();
    app.log("Settings saved.");
    Response::json(200, &json!({ "ok": true, "config": cfg }))
}

/// Runs one tool and writes its receipt. The caller holds `tool_busy`.
fn run_one(app: &App, tool: &str, action: &str, args: Vec<String>) -> Result<(tools::RunResult, Option<String>), Response> {
    let Some(program) = tools::find_tool(tool, app.tools_dir.as_deref()) else {
        return Err(Response::error(
            404,
            &format!("{tool}.exe was not found next to aien-rig.exe. Unzip the whole release into one folder."),
        ));
    };
    let timeout = Duration::from_secs(app.config().tool_timeout_secs);
    app.log(format!("Running {tool} {}", args.join(" ")));
    let r = tools::run(&program, &args, timeout)
        .map_err(|e| Response::error(500, &format!("could not run {tool}: {e}")))?;
    let receipt = tools::write_receipt(&app.data_dir, tool, action, &r).ok();
    app.log(format!(
        "{tool} {action} finished: exit {:?}{} in {:.1} s",
        r.exit_code,
        if r.timed_out { " (stopped: too long)" } else { "" },
        r.seconds
    ));
    Ok((r, receipt))
}

fn run_reply(r: &tools::RunResult, receipt: Option<String>, result: Option<Value>) -> Response {
    Response::json(
        200,
        &json!({
            "ok": r.ok(),
            "exit_code": r.exit_code,
            "timed_out": r.timed_out,
            "seconds": r.seconds,
            "result": result.or_else(|| r.parsed()),
            "stdout": r.stdout,
            "stderr": r.stderr,
            "receipt": receipt,
        }),
    )
}

fn busy() -> Response {
    Response::error(409, "Another tool run is still going. Wait for it to finish.")
}

fn run_tool(app: &App, tool: &str, action: &str, args: Vec<String>) -> Response {
    let Ok(_busy) = app.tool_busy.try_lock() else {
        return busy();
    };
    match run_one(app, tool, action, args) {
        Ok((r, receipt)) => run_reply(&r, receipt, None),
        Err(resp) => resp,
    }
}

fn tune(app: &App, body: &Value) -> Response {
    let cfg = app.config();
    let action = body["action"].as_str().unwrap_or("");
    let game = body["game"].as_str().unwrap_or("").trim().to_string();
    let template = match action {
        "scan" => &cfg.tune_scan,
        "measure" => &cfg.tune_measure,
        "apply" => &cfg.tune_apply,
        "undo" => &cfg.tune_undo,
        _ => return Response::error(400, "action must be scan, measure, apply or undo"),
    };
    let needs_game = template.iter().any(|a| a.contains("{game}"));
    if needs_game && game.is_empty() {
        return Response::error(400, "Pick a game first (run Scan to list them).");
    }
    run_tool(app, tools::TUNE, action, tools::fill(template, "game", &game))
}

const MEDIA_EXTS: [&str; 12] = [
    "mp4", "mov", "mkv", "webm", "avi", "m4v", "mp3", "wav", "m4a", "flac", "ogg", "aac",
];

pub fn is_media(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| MEDIA_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Video or audio: rig-listen makes a transcript, then rig-create.
/// Caption file: rig-create directly. rig-create writes result.json into
/// the results folder; the page shows that.
fn create(app: &App, body: &Value) -> Response {
    let path = body["path"].as_str().unwrap_or("").trim().trim_matches('"').to_string();
    if path.is_empty() {
        return Response::error(400, "Pick a video or caption file first.");
    }
    let file = std::path::Path::new(&path);
    if !file.is_file() {
        return Response::error(404, &format!("File not found: {path}"));
    }
    let cfg = app.config();
    let stem: String = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .take(40)
        .collect();
    let out_dir = app
        .data_dir
        .join("create")
        .join(format!("{}-{stem}", crate::config::stamp(unix_now())));
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        return Response::error(500, &format!("could not make the results folder: {e}"));
    }
    let Ok(_busy) = app.tool_busy.try_lock() else {
        return busy();
    };

    let mut transcript = path.clone();
    if is_media(&path) {
        if tools::find_tool(tools::LISTEN, app.tools_dir.as_deref()).is_none() {
            return Response::error(
                404,
                "Videos need the speech program (rig-listen.exe), which is not in this download yet. \
                 Use the video's caption file instead: in YouTube Studio open Subtitles, then download the .srt or .vtt.",
            );
        }
        let tjson = out_dir.join("transcript.json").display().to_string();
        let args = tools::fill(&tools::fill(&cfg.listen_args, "file", &path), "out", &tjson);
        let (r, receipt) = match run_one(app, tools::LISTEN, "transcribe", args) {
            Ok(x) => x,
            Err(resp) => return resp,
        };
        if !r.ok() {
            return run_reply(&r, receipt, None);
        }
        transcript = tjson;
    }

    let server = format!("{}/v1", cfg.engine_url.trim_end_matches('/'));
    let out = out_dir.display().to_string();
    let args = tools::fill(
        &tools::fill(&tools::fill(&cfg.create_args, "file", &transcript), "server", &server),
        "out",
        &out,
    );
    let (r, receipt) = match run_one(app, tools::CREATE, "create", args) {
        Ok(x) => x,
        Err(resp) => return resp,
    };
    let result = std::fs::read(out_dir.join("result.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .map(|mut v| {
            v["results_folder"] = json!(out);
            v
        });
    run_reply(&r, receipt, result)
}

fn pick_file(body: &Value) -> Response {
    if !cfg!(windows) {
        return Response::error(501, "The file picker works on Windows. Type the full path instead.");
    }
    let filter = match body["kind"].as_str().unwrap_or("") {
        "captions" => "Captions (*.srt;*.vtt;*.txt)|*.srt;*.vtt;*.txt|All files (*.*)|*.*",
        _ => "Videos (*.mp4;*.mov;*.mkv;*.webm;*.avi)|*.mp4;*.mov;*.mkv;*.webm;*.avi|All files (*.*)|*.*",
    };
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; \
         $f = New-Object System.Windows.Forms.Form; $f.TopMost = $true; \
         $d = New-Object System.Windows.Forms.OpenFileDialog; $d.Filter = '{filter}'; \
         [Console]::OutputEncoding = [System.Text.Encoding]::UTF8; \
         if ($d.ShowDialog($f) -eq [System.Windows.Forms.DialogResult]::OK) {{ $d.FileName }}"
    );
    let mut c = Command::new("powershell");
    c.args(["-NoProfile", "-STA", "-Command", &script]);
    no_window(&mut c);
    match c.output() {
        Ok(o) => {
            let p = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if p.is_empty() {
                Response::json(200, &json!({ "ok": false, "error": "No file picked." }))
            } else {
                Response::json(200, &json!({ "ok": true, "path": p }))
            }
        }
        Err(e) => Response::error(500, &format!("could not open the file picker: {e}")),
    }
}

fn engine_action(app: &App, body: &Value) -> Response {
    let cfg = app.config();
    let mut eng = app.engine.lock().unwrap();
    let r = match body["action"].as_str().unwrap_or("") {
        "start" => eng.start(&cfg, Duration::from_secs(120)),
        "stop" => eng.stop(&cfg),
        _ => return Response::error(400, "action must be start or stop"),
    };
    drop(eng);
    match r {
        Ok(msg) => {
            app.log(&msg);
            Response::json(200, &json!({ "ok": true, "message": msg }))
        }
        Err(msg) => {
            app.log(&msg);
            Response::json(200, &json!({ "ok": false, "error": msg }))
        }
    }
}

/// Sends the conversation to the engine and returns its reply. Retries on
/// 503 (the engine runs one generation at a time) for up to a minute.
pub fn chat(app: &App, body: &Value) -> Response {
    let cfg = app.config();
    let Some(messages) = body["messages"].as_array() else {
        return Response::error(400, "messages must be a list");
    };
    let Some(models) = engine::probe(&cfg.engine_url) else {
        return Response::error(
            503,
            "The engine is not running. Press Start engine. (On the RTX 5070 the engine needs the WSL2 GPU path, which is not finished yet: see docs/WINDOWS-GPU-PATH.md.)",
        );
    };
    let model = engine::model_id(&models).unwrap_or_else(|| "default".into());
    let req = json!({
        "model": model,
        "messages": messages,
        "max_tokens": body["max_tokens"].as_u64().unwrap_or(1024).min(4096),
        "temperature": body["temperature"].as_f64().unwrap_or(0.7),
        "stream": false,
    });
    let url = format!("{}/v1/chat/completions", cfg.engine_url.trim_end_matches('/'));
    let payload = serde_json::to_vec(&req).unwrap_or_default();
    let t0 = std::time::Instant::now();
    loop {
        let r = match http::call("POST", &url, Some(&payload), Duration::from_secs(600)) {
            Ok(r) => r,
            Err(e) => return Response::error(502, &format!("The engine did not answer: {e}")),
        };
        if r.status == 503 && t0.elapsed() < Duration::from_secs(60) {
            let wait = r
                .header("retry-after")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(1)
                .clamp(1, 10);
            std::thread::sleep(Duration::from_secs(wait));
            continue;
        }
        let v: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
        if r.status != 200 {
            let msg = v["error"]["message"]
                .as_str()
                .or(v["error"].as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| String::from_utf8_lossy(&r.body).into_owned());
            return Response::error(502, &format!("Engine said {}: {msg}", r.status));
        }
        let reply = v["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string();
        return Response::json(
            200,
            &json!({
                "ok": true,
                "reply": reply,
                "finish_reason": v["choices"][0]["finish_reason"],
                "usage": v["usage"],
                "model": model,
            }),
        );
    }
}

/// Background loop: every 5 seconds, look for a listed game.
pub fn watch_games(app: Arc<App>) {
    loop {
        let cfg = app.config();
        let running = games::running_programs();
        let now = games::find_game(&cfg.games, &running);
        let before = app.game.lock().unwrap().current.clone();
        for change in games::changes(before.as_deref(), now.as_deref()) {
            on_change(&app, &cfg, change);
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

fn on_change(app: &App, cfg: &Config, change: Change) {
    match change {
        Change::Started(name) => {
            app.log(format!("Game started: {name}"));
            let mut st = GameState { current: Some(name), ..Default::default() };
            if cfg.auto_game_mode {
                let mut eng = app.engine.lock().unwrap();
                if engine::probe(&cfg.engine_url).is_some() || eng.managed_running() {
                    st.engine_was_running = true;
                    match eng.stop(cfg) {
                        Ok(_) => app.log("Engine paused so the game gets the GPU memory."),
                        Err(e) => app.log(format!("Could not pause the engine: {e}")),
                    }
                }
                drop(eng);
                tuner_game_mode(app, &cfg.tune_game_mode_on, "on");
                if cfg!(windows) {
                    match games::read_game_mode() {
                        Ok(Some(0)) => match games::write_game_mode(Some(1)) {
                            Ok(()) => {
                                st.saved_game_mode = Some(Some(0));
                                app.log("Windows Game Mode turned on.");
                            }
                            Err(e) => app.log(format!("Could not turn on Game Mode: {e}")),
                        },
                        Ok(_) => app.log("Windows Game Mode is already on."),
                        Err(e) => app.log(format!("Could not read Game Mode: {e}")),
                    }
                }
            }
            *app.game.lock().unwrap() = st;
        }
        Change::Ended(name) => {
            app.log(format!("Game closed: {name}"));
            let st = std::mem::take(&mut *app.game.lock().unwrap());
            if cfg.auto_game_mode {
                tuner_game_mode(app, &cfg.tune_game_mode_off, "off");
            }
            if let Some(prev) = st.saved_game_mode {
                match games::write_game_mode(prev) {
                    Ok(()) => app.log("Windows Game Mode set back to how it was."),
                    Err(e) => app.log(format!("Could not set Game Mode back: {e}")),
                }
            }
            if st.engine_was_running {
                let r = app.engine.lock().unwrap().start(cfg, Duration::from_secs(120));
                match r {
                    Ok(m) => app.log(m),
                    Err(e) => app.log(format!("Could not restart the engine: {e}")),
                }
            }
        }
    }
}

/// Best effort: tell rig-tune about game mode (it pauses Windows-side AI
/// server processes). Skipped when rig-tune is absent or busy.
fn tuner_game_mode(app: &App, args: &[String], what: &str) {
    if args.is_empty() || tools::find_tool(tools::TUNE, app.tools_dir.as_deref()).is_none() {
        return;
    }
    let Ok(_busy) = app.tool_busy.try_lock() else {
        app.log(format!("Skipped rig-tune game-mode {what}: another tool is running."));
        return;
    };
    let _ = run_one(app, tools::TUNE, &format!("game-mode-{what}"), args.to_vec());
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aien-rig-srv-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn start_app(tag: &str, cfg: Config, tools_dir: Option<PathBuf>) -> (Arc<App>, u16) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let mut app = App::new(port, temp_dir(&format!("{tag}-data")), cfg);
        app.tools_dir = tools_dir;
        let app = Arc::new(app);
        let a2 = app.clone();
        std::thread::spawn(move || serve(l, a2));
        (app, port)
    }

    fn raw(port: u16, req: &str) -> (u16, Value, String) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).unwrap();
        let r = http::parse_client_response(&out).unwrap();
        let text = String::from_utf8_lossy(&r.body).into_owned();
        (r.status, serde_json::from_str(&text).unwrap_or(Value::Null), text)
    }

    fn post(port: u16, path: &str, body: &Value) -> (u16, Value) {
        let b = body.to_string();
        let (s, v, _) = raw(
            port,
            &format!(
                "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-AIEN-Rig: 1\r\nContent-Length: {}\r\n\r\n{b}",
                b.len()
            ),
        );
        (s, v)
    }

    #[test]
    fn serves_page_and_guards_host_and_header() {
        let (_app, port) = start_app("guards", Config::default(), None);
        let (s, _, text) = raw(port, &format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"));
        assert_eq!(s, 200);
        assert!(text.contains("AIEN Rig"));
        let (s, _, _) = raw(port, "GET /api/status HTTP/1.1\r\nHost: evil.example:80\r\n\r\n");
        assert_eq!(s, 403);
        let (s, _, _) = raw(
            port,
            &format!("POST /api/tune HTTP/1.1\r\nHost: localhost:{port}\r\nContent-Length: 2\r\n\r\n{{}}"),
        );
        assert_eq!(s, 403);
        let (s, v, _) = raw(port, &format!("GET /api/status HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n"));
        assert_eq!(s, 200);
        assert_eq!(v["ok"], json!(true));
    }

    #[test]
    fn tune_reports_missing_tool_and_bad_action() {
        let (_app, port) = start_app("missing", Config::default(), Some(temp_dir("empty-tools")));
        let (s, v) = post(port, "/api/tune", &json!({"action":"dance"}));
        assert_eq!(s, 400, "{v}");
        let (s, v) = post(port, "/api/tune", &json!({"action":"measure"}));
        assert_eq!(s, 400, "{v}");
        // rig-tune may exist on PATH on a dev machine; only check when absent.
        if tools::find_tool(tools::TUNE, None).is_none() {
            let (s, v) = post(port, "/api/tune", &json!({"action":"scan"}));
            assert_eq!(s, 404, "{v}");
            assert!(v["error"].as_str().unwrap().contains("rig-tune"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn tune_runs_tool_and_keeps_receipt() {
        use std::os::unix::fs::PermissionsExt;
        let tools_dir = temp_dir("tools");
        let script = tools_dir.join("rig-tune");
        std::fs::write(&script, "#!/bin/sh\necho '{\"action\":\"'$1'\",\"game\":\"'$2'\"}'\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (app, port) = start_app("tools", Config::default(), Some(tools_dir));
        let (s, v) = post(port, "/api/tune", &json!({"action":"measure","game":"cs2"}));
        assert_eq!(s, 200, "{v}");
        assert_eq!(v["ok"], json!(true));
        assert_eq!(v["result"]["action"], json!("measure"));
        assert_eq!(v["result"]["game"], json!("cs2"));
        let name = v["receipt"].as_str().unwrap().to_string();
        assert!(tools::read_receipt(&app.data_dir, &name).is_some());
        let (s, _, text) = raw(port, &format!("GET /api/receipts HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"));
        assert_eq!(s, 200);
        assert!(text.contains(&name));
    }

    #[cfg(unix)]
    #[test]
    fn create_runs_creator_and_reads_result_json() {
        use std::os::unix::fs::PermissionsExt;
        let tools_dir = temp_dir("create-tools");
        let script = tools_dir.join("rig-create");
        // Fake rig-create: args are <file> --server <url> --out <dir>.
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf '{\"titles\":[\"T1\"],\"server\":\"%s\"}' \"$3\" > \"$5/result.json\"\necho done\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let captions = tools_dir.join("clip.srt");
        std::fs::write(&captions, "1\n00:00:00,000 --> 00:00:01,000\nhi\n").unwrap();
        let (_app, port) = start_app("create", Config::default(), Some(tools_dir.clone()));
        let (s, v) = post(port, "/api/create", &json!({"path": captions.display().to_string()}));
        assert_eq!(s, 200, "{v}");
        assert_eq!(v["result"]["titles"], json!(["T1"]));
        assert_eq!(v["result"]["server"], json!("http://127.0.0.1:18120/v1"));
        assert!(v["result"]["results_folder"].as_str().is_some());

        let video = tools_dir.join("clip.mp4");
        std::fs::write(&video, b"not really a video").unwrap();
        if tools::find_tool(tools::LISTEN, None).is_none() {
            let (s, v) = post(port, "/api/create", &json!({"path": video.display().to_string()}));
            assert_eq!(s, 404, "{v}");
            assert!(v["error"].as_str().unwrap().contains("rig-listen"));
        }
        let (s, _) = post(port, "/api/create", &json!({"path": "/no/such/file.srt"}));
        assert_eq!(s, 404);
    }

    #[test]
    fn media_detection() {
        assert!(is_media("C:\\Videos\\Clip.MP4"));
        assert!(is_media("a.mkv"));
        assert!(!is_media("a.srt"));
        assert!(!is_media("noext"));
    }

    #[test]
    fn config_update_validates_and_saves() {
        let (app, port) = start_app("config", Config::default(), None);
        let (s, v) = post(port, "/api/config", &json!({"games":["a.exe"],"unknown":1}));
        assert_eq!(s, 200, "{v}");
        assert_eq!(app.config().games, vec!["a.exe"]);
        assert_eq!(Config::load(&app.config_path()).games, vec!["a.exe"]);
        let (s, _) = post(port, "/api/config", &json!({"engine_mode":"magic"}));
        assert_eq!(s, 400);
    }

    /// A stand-in engine: answers /v1/models, then 503 once, then a reply
    /// in chunked encoding.
    fn fake_engine() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let mut chat_calls = 0;
            for s in l.incoming() {
                let mut s = s.unwrap();
                let req = http::read_request(&mut s).unwrap();
                let reply = if req.path == "/v1/models" {
                    let b = r#"{"object":"list","data":[{"id":"test-model"}]}"#;
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{b}", b.len())
                } else {
                    chat_calls += 1;
                    if chat_calls == 1 {
                        "HTTP/1.1 503 Busy\r\nRetry-After: 1\r\nContent-Length: 0\r\n\r\n".to_string()
                    } else {
                        let v: Value = serde_json::from_slice(&req.body).unwrap();
                        assert_eq!(v["model"], json!("test-model"));
                        let b = json!({"choices":[{"message":{"content":"hello back"},"finish_reason":"stop"}]}).to_string();
                        format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{b}\r\n0\r\n\r\n", b.len())
                    }
                };
                s.write_all(reply.as_bytes()).unwrap();
            }
        });
        port
    }

    #[test]
    fn chat_goes_through_engine_with_retry() {
        let eport = fake_engine();
        let mut cfg = Config::default();
        cfg.engine_url = format!("http://127.0.0.1:{eport}");
        let (_app, port) = start_app("chat", cfg, None);
        let (s, v) = post(port, "/api/chat", &json!({"messages":[{"role":"user","content":"hi"}]}));
        assert_eq!(s, 200, "{v}");
        assert_eq!(v["reply"], json!("hello back"));
        assert_eq!(v["model"], json!("test-model"));
    }

    #[test]
    fn chat_says_when_engine_is_down() {
        let mut cfg = Config::default();
        cfg.engine_url = "http://127.0.0.1:9".into();
        let (_app, port) = start_app("chat-down", cfg, None);
        let (s, v) = post(port, "/api/chat", &json!({"messages":[]}));
        assert_eq!(s, 503);
        assert!(v["error"].as_str().unwrap().contains("not running"));
    }

    #[test]
    fn game_end_restores_state() {
        let app = App::new(1, temp_dir("game"), Config { engine_url: "http://127.0.0.1:9".into(), ..Config::default() });
        let cfg = app.config();
        on_change(&app, &cfg, Change::Started("cs2.exe".into()));
        assert_eq!(app.game.lock().unwrap().current.as_deref(), Some("cs2.exe"));
        assert!(!app.game.lock().unwrap().engine_was_running);
        on_change(&app, &cfg, Change::Ended("cs2.exe".into()));
        assert!(app.game.lock().unwrap().current.is_none());
        let ev = app.events.lock().unwrap();
        assert!(ev.iter().any(|e| e["text"] == json!("Game started: cs2.exe")));
        assert!(ev.iter().any(|e| e["text"] == json!("Game closed: cs2.exe")));
    }

    #[test]
    fn host_check() {
        assert!(host_allowed(Some("127.0.0.1:5"), 5));
        assert!(host_allowed(Some("localhost:5"), 5));
        assert!(!host_allowed(Some("127.0.0.1:6"), 5));
        assert!(!host_allowed(Some("attacker.test:5"), 5));
        assert!(!host_allowed(None, 5));
    }
}
