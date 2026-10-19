//! aien-rig: one Windows program for the RTX gaming PC.
//!
//! Opens a local page at http://127.0.0.1:18100 with three tabs:
//! Play (FPS tuner), Create (YouTube helper) and Chat (local engine).

mod config;
mod engine;
mod games;
mod http;
mod server;
mod tools;

use std::net::TcpListener;
use std::sync::Arc;

const DEFAULT_PORT: u16 = 18100;

fn usage() -> String {
    format!(
        "aien-rig {}\n\nUsage: aien-rig [--port N] [--no-browser]\n\n\
         Opens the AIEN Rig page in your browser. Keep this window open while you use it;\n\
         close it to quit.\n",
        env!("CARGO_PKG_VERSION")
    )
}

fn open_browser(url: &str) {
    let r = if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        tools::no_window(&mut c);
        c.spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    if r.is_err() {
        println!("Open this address in your browser: {url}");
    }
}

fn main() {
    let mut port = DEFAULT_PORT;
    let mut browser = true;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" => match args.next().and_then(|v| v.parse().ok()) {
                Some(p) => port = p,
                None => {
                    eprintln!("--port needs a number\n\n{}", usage());
                    std::process::exit(2);
                }
            },
            "--no-browser" => browser = false,
            "--version" | "-V" => {
                println!("aien-rig {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--help" | "-h" => {
                print!("{}", usage());
                return;
            }
            other => {
                eprintln!("unknown option {other}\n\n{}", usage());
                std::process::exit(2);
            }
        }
    }

    // Try the chosen port, then the next ten.
    let listener = (port..port.saturating_add(10))
        .find_map(|p| TcpListener::bind(("127.0.0.1", p)).ok())
        .unwrap_or_else(|| {
            eprintln!("Could not open a local port near {port}. Is AIEN Rig already running?");
            std::process::exit(1);
        });
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(port);

    let data_dir = config::data_dir();
    let _ = std::fs::create_dir_all(&data_dir);
    let cfg = config::Config::load(&data_dir.join("config.json"));
    let app = Arc::new(server::App::new(port, data_dir.clone(), cfg));

    // On window close or Ctrl+C: stop the engine if this app started it.
    {
        let app = app.clone();
        let _ = ctrlc::set_handler(move || {
            let cfg = app.config();
            if let Ok(mut eng) = app.engine.try_lock() {
                if eng.managed_running() {
                    let _ = eng.stop(&cfg);
                }
            }
            std::process::exit(0);
        });
    }

    {
        let app = app.clone();
        std::thread::spawn(move || server::watch_games(app));
    }

    let url = format!("http://127.0.0.1:{port}/");
    println!("AIEN Rig {} is running.", env!("CARGO_PKG_VERSION"));
    println!("Page:     {url}");
    println!("Data:     {}", data_dir.display());
    println!("Keep this window open while you use AIEN Rig. Close it to quit.");
    if browser {
        open_browser(&url);
    }
    server::serve(listener, app);
}
