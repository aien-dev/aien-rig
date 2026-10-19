//! rig-tune: the AIEN Rig FPS tuner command line.

#[cfg(windows)]
use aien_rig_tuner::stats;
use aien_rig_tuner::{paths, receipt, timefmt, undo, VERSION};
use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use std::io::{BufRead, Write};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "rig-tune", version = VERSION, about = "AIEN Rig FPS tuner: measure every game, apply only safe changes, undo anything.")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Check this PC for things that cost frames, with a plain-English fix for each.
    Scan {
        /// Print machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
        /// Skip the 1.5 second background-app sample and the game library.
        #[arg(long)]
        quick: bool,
    },
    /// List installed Steam, Epic and Xbox games and the executable to measure.
    Games {
        #[arg(long)]
        json: bool,
    },
    /// Count a running game's frames for N seconds and save a receipt.
    Measure {
        /// The game's executable, e.g. Cyberpunk2077.exe
        exe: String,
        #[arg(long, default_value_t = 60)]
        seconds: u32,
        /// A word saved with the receipt, e.g. before / after.
        #[arg(long, default_value = "run")]
        label: String,
        /// Game name for the receipt (default: looked up from the library).
        #[arg(long)]
        game: Option<String>,
        /// How long to wait for the game to start, in seconds.
        #[arg(long, default_value_t = 120)]
        wait: u32,
        #[arg(long)]
        json: bool,
    },
    /// Compare two receipts of a game (default: the first and the newest).
    Compare {
        game: Option<String>,
        #[arg(long)]
        before: Option<PathBuf>,
        #[arg(long)]
        after: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// List saved receipts.
    Receipts { game: Option<String> },
    /// Make the safe changes scan suggested, asking before each one.
    Apply {
        /// Only these tweaks (comma separated ids).
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// Show what would change, change nothing.
        #[arg(long)]
        dry_run: bool,
        /// Do not ask (for scripted use; every change is still logged for undo).
        #[arg(long)]
        yes: bool,
    },
    /// Put back what apply changed, exactly as it was.
    Undo {
        /// Undo one change by its number (see --list).
        #[arg(long)]
        id: Option<u64>,
        /// Undo every change made by one tweak.
        #[arg(long)]
        tweak: Option<String>,
        /// Also restore settings that were changed again after rig-tune changed them.
        #[arg(long)]
        force: bool,
        /// Show the change log and change nothing.
        #[arg(long)]
        list: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Pause (on) or resume (off) the AIEN Rig AI server so it never costs frames.
    GameMode {
        #[arg(value_parser = ["on", "off", "status"])]
        state: String,
        /// Server executable names (default: the AIEN Rig server names).
        #[arg(long = "process")]
        processes: Vec<String>,
    },
    /// Test helper: present frames at a fixed rate with a software renderer.
    #[command(hide = true)]
    ProbePresent {
        #[arg(long, default_value_t = 20)]
        seconds: u32,
        #[arg(long, default_value_t = 120)]
        fps: u32,
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[allow(dead_code)]
fn ask(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

#[allow(dead_code)]
fn now() -> String {
    timefmt::rfc3339(timefmt::now_unix())
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("rig-tune: {e:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.cmd {
        Cmd::Compare { game, before, after, json } => cmd_compare(game, before, after, json),
        Cmd::Receipts { game } => cmd_receipts(game),
        Cmd::Undo { list: true, .. } => cmd_undo_list(),
        other => platform(other),
    }
}

fn cmd_receipts(game: Option<String>) -> Result<()> {
    let dir = paths::receipts_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        println!("No receipts yet ({}).", dir.display());
        return Ok(());
    };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    if let Some(g) = game {
        files = receipt::list_for_game(&dir, &g);
    }
    files.sort();
    for f in files {
        match receipt::read(&f) {
            Ok(r) => println!(
                "{}  {:<28} {:<8} avg {:>6.1} fps  1% low {:>6.1}  0.1% low {:>6.1}  {}",
                r.started_at,
                r.game,
                r.label,
                r.stats.avg_fps,
                r.stats.low_1pct_fps,
                r.stats.low_01pct_fps,
                f.file_name().unwrap_or_default().to_string_lossy()
            ),
            Err(e) => println!("(unreadable) {}: {e:#}", f.display()),
        }
    }
    Ok(())
}

fn cmd_compare(game: Option<String>, before: Option<PathBuf>, after: Option<PathBuf>, json: bool) -> Result<()> {
    let (a, b) = match (before, after, game) {
        (Some(a), Some(b), _) => (a, b),
        (_, _, Some(g)) => {
            let list = receipt::list_for_game(&paths::receipts_dir(), &g);
            if list.len() < 2 {
                bail!(
                    "need at least two receipts for {g} (found {}). Run `rig-tune measure` before and after a change.",
                    list.len()
                );
            }
            (list[0].clone(), list[list.len() - 1].clone())
        }
        _ => bail!("give a game name, or --before FILE --after FILE"),
    };
    let (ra, rb) = (receipt::read(&a)?, receipt::read(&b)?);
    let c = receipt::compare(&ra, &rb);
    if json {
        println!("{}", serde_json::to_string_pretty(&c)?);
        return Ok(());
    }
    println!("{}", c.summary);
    println!("  before: {} ({}, label {})", c.before, a.display(), ra.label);
    println!("  after:  {} ({}, label {})", c.after, b.display(), rb.label);
    println!("  {:<12} {:>9} {:>9} {:>8}", "", "before", "after", "change");
    for (name, t) in [("average", c.avg_fps), ("1% low", c.low_1pct_fps), ("0.1% low", c.low_01pct_fps)] {
        println!("  {:<12} {:>9.1} {:>9.1} {:>+7.1}%", name, t.0, t.1, t.2);
    }
    if !c.tweaks_added.is_empty() {
        println!("  changes made between the runs: {}", c.tweaks_added.join(", "));
    }
    if !c.tweaks_removed.is_empty() {
        println!("  changes undone between the runs: {}", c.tweaks_removed.join(", "));
    }
    for w in &c.warnings {
        println!("  note: {w}");
    }
    Ok(())
}

fn cmd_undo_list() -> Result<()> {
    let log = undo::load_log(&paths::undo_log())?;
    if log.entries.is_empty() {
        println!("Nothing has been changed by rig-tune on this PC.");
        return Ok(());
    }
    for e in &log.entries {
        println!(
            "#{:<3} {:<9} {:<26} {}: {} -> {}  ({}{})",
            e.id,
            format!("{:?}", e.state).to_ascii_lowercase(),
            e.tweak,
            e.setting,
            e.old,
            e.new,
            e.applied_at,
            e.undone_at.as_ref().map(|u| format!(", undone {u}")).unwrap_or_default()
        );
    }
    println!("Log file: {}", paths::undo_log().display());
    Ok(())
}

#[cfg(not(windows))]
fn platform(_cmd: Cmd) -> Result<()> {
    bail!("this command runs on Windows only (compare, receipts and undo --list work everywhere)")
}

#[cfg(windows)]
fn platform(cmd: Cmd) -> Result<()> {
    windows_cmds::run(cmd)
}

#[cfg(windows)]
mod windows_cmds {
    use super::*;
    use aien_rig_tuner::findings::{self, Status};
    use aien_rig_tuner::frames;
    use aien_rig_tuner::gamemode::{self, GameModeState, Paused};
    use aien_rig_tuner::receipt::{Receipt, SeenRow, SystemSnapshot};
    use aien_rig_tuner::tweaks;
    use aien_rig_tuner::undo::{ApplyOutcome, FileSink, State, UndoOutcome, UndoSelect, Value};
    use aien_rig_tuner::win::{elevation, etw, present_probe, procs, scan, store::WinStore};

    pub fn run(cmd: Cmd) -> Result<()> {
        match cmd {
            Cmd::Scan { json, quick } => cmd_scan(json, quick),
            Cmd::Games { json } => cmd_games(json),
            Cmd::Measure { exe, seconds, label, game, wait, json } => {
                cmd_measure(&exe, seconds, &label, game, wait, json)
            }
            Cmd::Apply { only, dry_run, yes } => cmd_apply(only, dry_run, yes),
            Cmd::Undo { id, tweak, force, yes, .. } => cmd_undo(id, tweak, force, yes),
            Cmd::GameMode { state, processes } => cmd_game_mode(&state, processes),
            Cmd::ProbePresent { seconds, fps, out } => {
                let r = present_probe::run(seconds, fps)?;
                let text = serde_json::to_string_pretty(&r)?;
                if let Some(p) = out {
                    std::fs::write(p, &text)?;
                }
                println!("{text}");
                Ok(())
            }
            Cmd::Compare { .. } | Cmd::Receipts { .. } => unreachable!(),
        }
    }

    fn cmd_scan(json: bool, quick: bool) -> Result<()> {
        if !json {
            println!("Scanning this PC{}...", if quick { "" } else { " (takes a few seconds)" });
        }
        let facts = scan::facts(!quick, !quick);
        let found = findings::evaluate(&facts);
        if json {
            let v = serde_json::json!({
                "tool_version": VERSION,
                "scanned_at": now(),
                "facts": facts,
                "findings": found,
            });
            println!("{}", serde_json::to_string_pretty(&v)?);
            return Ok(());
        }
        for f in &found {
            let tag = match f.status {
                Status::Good => "  OK ",
                Status::Tip => " TIP ",
                Status::Problem => " FIX ",
                Status::Unknown => "  ?  ",
            };
            println!("[{tag}] {}: {}", f.area, f.what);
            if f.status != Status::Good {
                println!("        -> {}", f.fix);
            }
        }
        for n in &facts.notes {
            println!("[  ?  ] could not read: {n}");
        }
        let n = found.iter().filter(|f| f.tweak.is_some()).count();
        if n > 0 {
            println!(
                "\n{n} of these can be changed for you, one at a time, with `rig-tune apply` (and put back with `rig-tune undo`)."
            );
        }
        Ok(())
    }

    fn cmd_games(json: bool) -> Result<()> {
        let games = scan::games();
        if json {
            println!("{}", serde_json::to_string_pretty(&games)?);
            return Ok(());
        }
        if games.is_empty() {
            println!("No Steam, Epic or Xbox games found.");
        }
        for g in games {
            println!(
                "{:<6} {:<40} measure with: {}",
                format!("{:?}", g.launcher),
                g.name,
                g.exe_candidates.first().cloned().unwrap_or_else(|| "(exe not found)".into())
            );
        }
        Ok(())
    }

    fn game_name_for(exe: &str) -> String {
        let want = exe.to_ascii_lowercase();
        for g in scan::games() {
            if g.exe_candidates.iter().any(|e| e.to_ascii_lowercase() == want) {
                return g.name;
            }
        }
        let lower = exe.to_ascii_lowercase();
        if lower.ends_with(".exe") {
            exe[..exe.len() - 4].to_string()
        } else {
            exe.to_string()
        }
    }

    fn snapshot() -> SystemSnapshot {
        let f = scan::facts(false, false);
        let gpu = f.gpus.iter().find(|g| g.is_nvidia()).or(f.gpus.first());
        let d = f.displays.iter().find(|d| d.primary).or(f.displays.first());
        SystemSnapshot {
            gpu: gpu.map(|g| g.name.clone()),
            driver: gpu.and_then(|g| {
                g.nvml
                    .as_ref()
                    .and_then(|n| n.driver_version.clone())
                    .or(g.dxgi_driver_version.clone())
            }),
            windows_build: f.windows_build,
            display: d.map(|d| format!("{}x{} @ {} Hz", d.width, d.height, d.hz)),
            power_plan: f.power_scheme.as_deref().map(|g| findings::power_plan_name(g).to_string()),
            hags: f.hags,
        }
    }

    fn cmd_measure(exe: &str, seconds: u32, label: &str, game: Option<String>, wait: u32, json: bool) -> Result<()> {
        if !elevation::is_admin() {
            bail!("counting frames needs administrator rights. Right-click Terminal, choose \"Run as administrator\", and run this again.");
        }
        let mut pids = procs::pids_named(exe);
        if pids.is_empty() {
            eprintln!("Waiting up to {wait} s for {exe} to start...");
            let t = std::time::Instant::now();
            while pids.is_empty() && t.elapsed().as_secs() < wait as u64 {
                std::thread::sleep(std::time::Duration::from_millis(500));
                pids = procs::pids_named(exe);
            }
            if pids.is_empty() {
                bail!("{exe} is not running. Start the game first (`rig-tune games` shows the exe name for each game).");
            }
        }
        let game = game.unwrap_or_else(|| game_name_for(exe));
        let system = snapshot();
        eprintln!("Measuring {game} ({exe}, pid {pids:?}) for {seconds} s. Play normally; keep the game in front.");
        let started = timefmt::now_unix();
        let exe_owned = exe.to_string();
        let mut last_check = std::time::Instant::now();
        let cap = etw::capture(seconds, || {
            if last_check.elapsed().as_secs() >= 2 {
                last_check = std::time::Instant::now();
                return !procs::pids_named(&exe_owned).is_empty();
            }
            true
        })?;
        // Include processes that started during the run.
        for p in procs::pids_named(exe) {
            if !pids.contains(&p) {
                pids.push(p);
            }
        }
        let Some(sel) = frames::select(&cap.events, &pids) else {
            let mut by_pid: std::collections::BTreeMap<u32, usize> = Default::default();
            for e in &cap.events {
                *by_pid.entry(e.pid).or_default() += 1;
            }
            let names = procs::list();
            let mut top: Vec<(u32, usize)> = by_pid.into_iter().collect();
            top.sort_by_key(|x| std::cmp::Reverse(x.1));
            let hint: Vec<String> = top
                .iter()
                .take(5)
                .map(|(p, n)| {
                    let name = names
                        .iter()
                        .find(|(q, _)| q == p)
                        .map(|(_, n)| n.clone())
                        .unwrap_or_else(|| "?".into());
                    format!("{name} (pid {p}): {n} present events")
                })
                .collect();
            bail!(
                "no frames from {exe} in {seconds} s. Is the game on screen (not paused in a menu that stops rendering)? Programs that did show frames: {}",
                if hint.is_empty() { "none".into() } else { hint.join("; ") }
            );
        };
        let ft = frames::frame_times_ms(&sel.timestamps, cap.qpc_frequency);
        let Some(st) = stats::compute(&ft) else { bail!("too few frames to measure") };
        let log = undo::load_log(&paths::undo_log()).unwrap_or_default();
        let mut active: Vec<String> = log
            .entries
            .iter()
            .filter(|e| e.state == State::Applied)
            .map(|e| e.tweak.clone())
            .collect();
        active.sort();
        active.dedup();
        let mut notes = Vec::new();
        if cap.events_lost > 0 {
            notes.push(format!(
                "Windows dropped {} trace events or buffers during the run; numbers may be slightly off.",
                cap.events_lost
            ));
        }
        if log.entries.iter().any(|e| e.state == State::Applied && e.needs_reboot) {
            notes.push("A change that needs a restart (GPU scheduling) is in the log; it only counts after the PC restarts.".into());
        }
        let r = Receipt {
            schema: receipt::SCHEMA.into(),
            tool_version: VERSION.into(),
            game: game.clone(),
            exe: exe.into(),
            label: label.into(),
            started_at: timefmt::rfc3339(started),
            seconds_requested: seconds,
            frame_source: sel.source,
            source_event_id: sel.event_id,
            pid: sel.pid,
            qpc_frequency: cap.qpc_frequency,
            stats: st.clone(),
            stats_definition: stats::STATS_DEFINITION.into(),
            graph: stats::graph(&ft, 600),
            frame_times_ms: ft.iter().map(|v| stats::round3(*v)).collect(),
            events_seen: sel
                .seen
                .iter()
                .map(|(s, p, i, c)| SeenRow { source: *s, pid: *p, event_id: *i, count: *c })
                .collect(),
            active_tweaks: active,
            system,
            notes,
        };
        let path = receipt::write(&paths::receipts_dir(), &r, &timefmt::file_stamp(started))?;
        if json {
            println!("{}", serde_json::json!({ "receipt": path, "stats": st }));
        } else {
            println!(
                "{game}: {:.1} fps average, 1% low {:.1}, 0.1% low {:.1} ({} frames over {:.1} s, counted from {:?} events).",
                st.avg_fps, st.low_1pct_fps, st.low_01pct_fps, st.frames, st.seconds, sel.source
            );
            println!("Receipt: {}", path.display());
        }
        Ok(())
    }

    fn cmd_apply(only: Vec<String>, dry_run: bool, yes: bool) -> Result<()> {
        for o in &only {
            if !tweaks::ALL_TWEAKS.contains(&o.as_str()) {
                bail!("unknown tweak {o}. Known: {}", tweaks::ALL_TWEAKS.join(", "));
            }
        }
        let facts = scan::facts(false, false);
        let mut props = tweaks::propose(&facts);
        if !only.is_empty() {
            props.retain(|p| only.contains(&p.tweak));
        }
        if props.is_empty() {
            println!("Nothing to change: every setting rig-tune knows about is already where it should be.");
            return Ok(());
        }
        let admin = elevation::is_admin();
        let log_path = paths::undo_log();
        let mut log = undo::load_log(&log_path)?;
        let mut store = WinStore;
        let mut changed = 0;
        let mut reboot = false;
        for p in props {
            println!(
                "\n{}\n  why: {}\n  change: {}: {} -> {}",
                p.title,
                p.why,
                p.setting,
                store_read(&store, &p.setting),
                p.new
            );
            if p.needs_admin && !admin {
                println!("  skipped: needs administrator rights (run the terminal as administrator).");
                continue;
            }
            if dry_run {
                continue;
            }
            if !yes && !ask("  Make this change?") {
                println!("  left as is.");
                continue;
            }
            let mut sink = FileSink(&log_path);
            match undo::apply_change(&mut log, &mut sink, &mut store, &p.tweak, &p.setting, &p.new, p.needs_reboot, &now()) {
                Ok(ApplyOutcome::AlreadySet) => println!("  already set."),
                Ok(ApplyOutcome::Applied(e)) => {
                    changed += 1;
                    reboot |= e.needs_reboot;
                    println!("  done (undo with `rig-tune undo --id {}`).", e.id);
                    if matches!(p.new, Value::Display { .. }) && !yes && !confirm_display(&mut log, &log_path, e.id)? {
                        changed -= 1;
                    }
                }
                Err(err) => println!("  not changed: {err:#}"),
            }
        }
        if dry_run {
            println!("\nDry run: nothing was changed.");
        } else {
            println!("\n{changed} change(s) made and logged in {}.", log_path.display());
            if reboot {
                println!("Restart the PC for GPU scheduling to take effect.");
            }
            if changed > 0 {
                println!("Measure a game again (`rig-tune measure <exe> --label after`) and compare with `rig-tune compare <game>`.");
            }
        }
        Ok(())
    }

    /// After a display change, keep it only if the user can see and confirm it.
    fn confirm_display(log: &mut undo::UndoLog, log_path: &std::path::Path, id: u64) -> Result<bool> {
        println!("  Can you see this? Press Enter within 15 seconds to keep the new refresh rate.");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = std::io::stdin().lock().read_line(&mut s);
            let _ = tx.send(());
        });
        if rx.recv_timeout(std::time::Duration::from_secs(15)).is_ok() {
            println!("  kept.");
            return Ok(true);
        }
        println!("  no answer: putting the old mode back.");
        let mut sink = FileSink(log_path);
        let mut store = WinStore;
        let out = undo::undo(log, &mut sink, &mut store, &UndoSelect::Id(id), true, &now())?;
        print_undo(&out);
        Ok(false)
    }

    fn store_read(store: &WinStore, s: &undo::Setting) -> String {
        use undo::Store;
        store
            .read(s)
            .map(|v| v.to_string())
            .unwrap_or_else(|e| format!("(unreadable: {e})"))
    }

    fn print_undo(out: &[UndoOutcome]) {
        for o in out {
            match o {
                UndoOutcome::Restored { id, setting, value } => println!("  #{id} restored {setting} to {value}"),
                UndoOutcome::ChangedSince { id, setting, ours, now } => println!(
                    "  #{id} left alone: {setting} is now {now}, not {ours} as rig-tune set it. Someone changed it since; use --force to restore anyway."
                ),
                UndoOutcome::Failed { id, setting, error } => println!("  #{id} FAILED for {setting}: {error}"),
            }
        }
    }

    fn cmd_undo(id: Option<u64>, tweak: Option<String>, force: bool, yes: bool) -> Result<()> {
        let log_path = paths::undo_log();
        let mut log = undo::load_log(&log_path)?;
        let sel = match (id, tweak) {
            (Some(i), _) => UndoSelect::Id(i),
            (None, Some(t)) => UndoSelect::Tweak(t),
            (None, None) => UndoSelect::All,
        };
        let pending: Vec<&undo::Entry> = log.active().collect();
        if pending.is_empty() {
            println!("Nothing to undo.");
            return Ok(());
        }
        if !yes {
            for e in &pending {
                println!("#{} {}: {} -> back to {}", e.id, e.tweak, e.setting, e.old);
            }
            if !ask("Undo the selected change(s)?") {
                return Ok(());
            }
        }
        let machine_wide = log.active().any(|e| {
            matches!(
                &e.setting,
                undo::Setting::RegDword { hive: undo::Hive::LocalMachine, .. }
                    | undo::Setting::RegString { hive: undo::Hive::LocalMachine, .. }
            )
        });
        if machine_wide && !elevation::is_admin() {
            println!("Note: some changes are machine-wide and need administrator rights to undo; those will fail without them.");
        }
        let mut sink = FileSink(&log_path);
        let mut store = WinStore;
        let out = undo::undo(&mut log, &mut sink, &mut store, &sel, force, &now())?;
        if out.is_empty() {
            println!("Nothing matched.");
        }
        print_undo(&out);
        if out.iter().any(|o| matches!(o, UndoOutcome::Failed { .. })) {
            bail!("some changes could not be undone; they stay in the log so you can retry");
        }
        Ok(())
    }

    fn cmd_game_mode(state: &str, processes: Vec<String>) -> Result<()> {
        let names: Vec<String> = if processes.is_empty() {
            gamemode::DEFAULT_SERVER_EXES.iter().map(|s| s.to_string()).collect()
        } else {
            processes
        };
        let path = paths::game_mode_state();
        let current: GameModeState = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let save = |s: &GameModeState| -> Result<()> {
            if let Some(d) = path.parent() {
                std::fs::create_dir_all(d)?;
            }
            std::fs::write(&path, serde_json::to_string_pretty(s)?)?;
            Ok(())
        };
        match state {
            "status" => {
                println!("Game mode is {}.", if current.active { "ON (AI server paused)" } else { "off" });
                for p in &current.paused {
                    println!("  paused: {} (pid {}, {} threads)", p.name, p.pid, p.threads_suspended);
                }
                Ok(())
            }
            "on" => {
                if current.active {
                    println!("Game mode is already on.");
                    return Ok(());
                }
                let mut st = GameModeState { active: true, since: now(), requested_by: "rig-tune".into(), paused: vec![] };
                save(&st)?; // tell a cooperative server first
                for (pid, name) in gamemode::find_servers(&procs::list(), &names) {
                    let n = procs::suspend(pid);
                    println!("Paused {name} (pid {pid}, {n} threads).");
                    st.paused.push(Paused { pid, name, threads_suspended: n });
                }
                save(&st)?;
                if st.paused.is_empty() {
                    println!("Game mode on. The AI server is not running, so there was nothing to pause.");
                } else {
                    println!("Game mode on: the AI server uses no CPU or GPU time until `rig-tune game-mode off`. Its video memory stays reserved.");
                }
                Ok(())
            }
            "off" => {
                let running = procs::list();
                for p in gamemode::safe_to_resume(&current, &running) {
                    let n = procs::resume(p.pid);
                    println!("Resumed {} (pid {}, {n} threads).", p.name, p.pid);
                }
                save(&GameModeState { active: false, since: now(), requested_by: "rig-tune".into(), paused: vec![] })?;
                println!("Game mode off.");
                Ok(())
            }
            _ => unreachable!(),
        }
    }
}
