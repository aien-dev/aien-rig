//! Processes: list, CPU sampling, per-process video memory (PDH counters),
//! and pausing/resuming the AI server's threads.

use super::from_wide;
use crate::findings::ProcessHog;
use std::collections::HashMap;
use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, FILETIME};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, Thread32First, Thread32Next, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
    PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_LARGE, PDH_HCOUNTER, PDH_HQUERY,
};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, OpenThread, ResumeThread, SuspendThread, PROCESS_QUERY_LIMITED_INFORMATION,
    THREAD_SUSPEND_RESUME,
};

/// (pid, exe file name) for every running process.
pub fn list() -> Vec<(u32, String)> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snap, &mut e).is_ok() {
            loop {
                out.push((e.th32ProcessID, from_wide(&e.szExeFile)));
                if Process32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    out
}

pub fn pids_named(exe: &str) -> Vec<u32> {
    let want = exe.to_ascii_lowercase();
    let want_exe = if want.ends_with(".exe") { want.clone() } else { format!("{want}.exe") };
    list()
        .into_iter()
        .filter(|(_, n)| {
            let n = n.to_ascii_lowercase();
            n == want || n == want_exe
        })
        .map(|(p, _)| p)
        .collect()
}

fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

fn cpu_time_100ns(pid: u32) -> Option<u64> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let r = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u);
        let _ = CloseHandle(h);
        r.ok()?;
        Some(ft(k) + ft(u))
    }
}

/// Per-process dedicated video memory in MB, from the
/// "GPU Process Memory(*)\Dedicated Usage" performance counter.
pub fn gpu_memory_mb() -> HashMap<u32, u64> {
    let mut out = HashMap::new();
    unsafe {
        let mut q = PDH_HQUERY::default();
        if PdhOpenQueryW(windows::core::PCWSTR::null(), 0, &mut q) != 0 {
            return out;
        }
        let mut c = PDH_HCOUNTER::default();
        if PdhAddEnglishCounterW(q, w!(r"\GPU Process Memory(*)\Dedicated Usage"), 0, &mut c) == 0
            && PdhCollectQueryData(q) == 0
        {
            let mut size = 0u32;
            let mut count = 0u32;
            let _ = PdhGetFormattedCounterArrayW(c, PDH_FMT_LARGE, &mut size, &mut count, None);
            if size > 0 {
                let mut buf = vec![0u8; size as usize];
                let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
                if PdhGetFormattedCounterArrayW(c, PDH_FMT_LARGE, &mut size, &mut count, Some(items)) == 0 {
                    for i in 0..count as usize {
                        let it = &*items.add(i);
                        let name = it.szName.to_string().unwrap_or_default();
                        // instance names look like pid_1234_luid_0x..._phys_0
                        if let Some(pid) = name
                            .strip_prefix("pid_")
                            .and_then(|r| r.split('_').next())
                            .and_then(|p| p.parse::<u32>().ok())
                        {
                            let bytes = it.FmtValue.Anonymous.largeValue.max(0) as u64;
                            *out.entry(pid).or_insert(0) += bytes / (1024 * 1024);
                        }
                    }
                }
            }
        }
        let _ = PdhCloseQuery(q);
    }
    out
}

/// Sample CPU use of every process over `window_ms`, plus video memory.
pub fn hogs(window_ms: u64) -> Vec<ProcessHog> {
    let procs = list();
    let before: HashMap<u32, u64> = procs.iter().filter_map(|(p, _)| cpu_time_100ns(*p).map(|t| (*p, t))).collect();
    let t0 = std::time::Instant::now();
    std::thread::sleep(std::time::Duration::from_millis(window_ms));
    let elapsed_100ns = t0.elapsed().as_nanos() as f64 / 100.0;
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;
    let mem = gpu_memory_mb();
    let mut out: Vec<ProcessHog> = procs
        .iter()
        .filter(|(p, _)| *p != 0)
        .map(|(pid, name)| {
            let cpu = match (before.get(pid), cpu_time_100ns(*pid)) {
                (Some(a), Some(b)) if b >= *a => (b - a) as f64 / (elapsed_100ns * cores) * 100.0,
                _ => 0.0,
            };
            ProcessHog { name: name.clone(), pid: *pid, cpu_pct: (cpu * 10.0).round() / 10.0, gpu_mem_mb: mem.get(pid).copied() }
        })
        .filter(|h| h.cpu_pct >= 1.0 || h.gpu_mem_mb.unwrap_or(0) >= 200)
        .collect();
    out.sort_by(|a, b| b.cpu_pct.partial_cmp(&a.cpu_pct).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(25);
    out
}

fn threads_of(pid: u32) -> Vec<u32> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) else { return out };
        let mut e = THREADENTRY32 { dwSize: std::mem::size_of::<THREADENTRY32>() as u32, ..Default::default() };
        if Thread32First(snap, &mut e).is_ok() {
            loop {
                if e.th32OwnerProcessID == pid {
                    out.push(e.th32ThreadID);
                }
                if Thread32Next(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// Suspend every thread of `pid`. Returns how many were suspended.
pub fn suspend(pid: u32) -> u32 {
    let mut n = 0;
    for tid in threads_of(pid) {
        unsafe {
            if let Ok(h) = OpenThread(THREAD_SUSPEND_RESUME, false, tid) {
                if SuspendThread(h) != u32::MAX {
                    n += 1;
                }
                let _ = CloseHandle(h);
            }
        }
    }
    n
}

/// Resume every thread of `pid` once (undoing one `suspend`).
pub fn resume(pid: u32) -> u32 {
    let mut n = 0;
    for tid in threads_of(pid) {
        unsafe {
            if let Ok(h) = OpenThread(THREAD_SUSPEND_RESUME, false, tid) {
                if ResumeThread(h) != u32::MAX {
                    n += 1;
                }
                let _ = CloseHandle(h);
            }
        }
    }
    n
}
