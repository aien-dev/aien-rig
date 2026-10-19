//! `rig-tune game-mode on|off`: pause the AIEN Rig AI server while gaming.
//!
//! Contract with the AIEN Rig app and AI server (written down here so both
//! sides agree):
//! 1. Cooperative: rig-tune writes %LOCALAPPDATA%\AIEN-Rig\game-mode.json with
//!    `"active": true`. The server should watch this file and, while active,
//!    finish or cancel its current request, stop taking new ones and ideally
//!    free its video memory. `"active": false` means resume.
//! 2. Guaranteed: rig-tune also suspends every thread of the server processes
//!    it finds (names in `DEFAULT_SERVER_EXES`, or `--process`), so they use
//!    zero CPU and launch no GPU work even if the server ignores the file.
//!    Their video memory stays allocated while suspended. `game-mode off`
//!    resumes exactly the processes it suspended, after checking each PID
//!    still belongs to the same program.
//!
//! Pure logic here; the thread suspension is in win::procs.

use serde::{Deserialize, Serialize};

pub const DEFAULT_SERVER_EXES: [&str; 4] = [
    "aien-rig-server.exe",
    "aien-rig-engine.exe",
    "aien-engine.exe",
    "aien-serve-http.exe",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Paused {
    pub pid: u32,
    pub name: String,
    pub threads_suspended: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameModeState {
    pub active: bool,
    pub since: String,
    pub requested_by: String,
    pub paused: Vec<Paused>,
}

/// Processes (pid, exe name) that are the AI server.
pub fn find_servers(procs: &[(u32, String)], names: &[String]) -> Vec<(u32, String)> {
    procs
        .iter()
        .filter(|(_, n)| names.iter().any(|w| w.eq_ignore_ascii_case(n)))
        .cloned()
        .collect()
}

/// Which paused entries are safe to resume: the PID must still be running
/// the same program (PIDs get reused after a process exits).
pub fn safe_to_resume<'a>(state: &'a GameModeState, procs: &[(u32, String)]) -> Vec<&'a Paused> {
    state
        .paused
        .iter()
        .filter(|p| procs.iter().any(|(pid, n)| *pid == p.pid && n.eq_ignore_ascii_case(&p.name)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_servers_case_insensitively() {
        let procs = vec![(1, "AIEN-Rig-Server.exe".to_string()), (2, "game.exe".to_string())];
        let names: Vec<String> = DEFAULT_SERVER_EXES.iter().map(|s| s.to_string()).collect();
        assert_eq!(find_servers(&procs, &names), vec![(1, "AIEN-Rig-Server.exe".to_string())]);
    }

    #[test]
    fn resume_guards_against_pid_reuse() {
        let st = GameModeState {
            active: true,
            since: "t".into(),
            requested_by: "rig-tune".into(),
            paused: vec![
                Paused { pid: 10, name: "aien-rig-server.exe".into(), threads_suspended: 4 },
                Paused { pid: 11, name: "aien-engine.exe".into(), threads_suspended: 2 },
            ],
        };
        // pid 11 now belongs to something else
        let procs = vec![(10, "aien-rig-server.exe".to_string()), (11, "notepad.exe".to_string())];
        let ok = safe_to_resume(&st, &procs);
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].pid, 10);
    }
}
