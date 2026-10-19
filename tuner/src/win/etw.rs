//! Our own frame counter: a real-time ETW session that listens to the
//! present events Windows' graphics stack already emits.
//!
//! Providers (all built into Windows, no driver hooks, nothing injected into
//! the game, so anti-cheat has nothing to object to):
//! - Microsoft-Windows-DXGI, event 42 Present_Start (D3D10/11/12, and
//!   Vulkan/OpenGL games the NVIDIA driver presents through DXGI)
//! - Microsoft-Windows-D3D9, event 1 Present_Start (older games)
//! - Microsoft-Windows-DxgKrnl present events (any API, used as fallback)
//!
//! Needs administrator rights (Windows requires it for ETW sessions).

use crate::frames::{ids, PresentEvent, Source};
use anyhow::{bail, Result};
use std::sync::{Arc, Mutex};
use windows::core::{GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_SUCCESS};
use windows::Win32::System::Diagnostics::Etw::{
    CloseTrace, ControlTraceW, EnableTraceEx2, OpenTraceW, ProcessTrace, StartTraceW, CONTROLTRACE_HANDLE,
    EVENT_CONTROL_CODE_ENABLE_PROVIDER, EVENT_RECORD, EVENT_TRACE_CONTROL_STOP, EVENT_TRACE_LOGFILEW,
    EVENT_TRACE_LOGFILEW_0, EVENT_TRACE_LOGFILEW_1, EVENT_TRACE_PROPERTIES, EVENT_TRACE_REAL_TIME_MODE,
    PROCESSTRACE_HANDLE, PROCESS_TRACE_MODE_EVENT_RECORD, PROCESS_TRACE_MODE_REAL_TIME, TRACE_LEVEL_INFORMATION,
    WNODE_FLAG_TRACED_GUID,
};
use windows::Win32::System::Performance::QueryPerformanceFrequency;

pub const SESSION_NAME: &str = "AIEN-Rig-FrameCapture";

const DXGI: GUID = GUID::from_u128(0xCA11C036_0102_4A2D_A6AD_F03CFED5D3C9);
const D3D9: GUID = GUID::from_u128(0x783ACA0A_790E_4D7F_8451_AA850511C6B9);
const DXGKRNL: GUID = GUID::from_u128(0x802EC45A_1E99_4B83_9920_87C98277BA9D);

#[repr(C)]
struct Props {
    p: EVENT_TRACE_PROPERTIES,
    name: [u16; 64],
}

fn props() -> Box<Props> {
    let mut b: Box<Props> = Box::new(unsafe { std::mem::zeroed() });
    b.p.Wnode.BufferSize = std::mem::size_of::<Props>() as u32;
    b.p.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
    b.p.Wnode.ClientContext = 1; // timestamps in QPC units
    b.p.LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
    b.p.BufferSize = 64; // KB
    b.p.MinimumBuffers = 16;
    b.p.MaximumBuffers = 128;
    b.p.FlushTimer = 1;
    b.p.LoggerNameOffset = std::mem::offset_of!(Props, name) as u32;
    b
}

/// Stop a leftover session with our name (from a crash or Ctrl+C).
pub fn stop_session_by_name() {
    let name = super::wide(SESSION_NAME);
    let mut p = props();
    unsafe {
        let _ = ControlTraceW(
            CONTROLTRACE_HANDLE { Value: 0 },
            PCWSTR(name.as_ptr()),
            &mut p.p,
            EVENT_TRACE_CONTROL_STOP,
        );
    }
}

#[derive(Default)]
struct Collector {
    events: Mutex<Vec<PresentEvent>>,
}

unsafe extern "system" fn on_event(rec: *mut EVENT_RECORD) {
    let rec = unsafe { &*rec };
    let ctx = rec.UserContext as *const Collector;
    if ctx.is_null() {
        return;
    }
    let h = &rec.EventHeader;
    let id = h.EventDescriptor.Id;
    let source = if h.ProviderId == DXGI && id == ids::DXGI_PRESENT_START {
        Source::Dxgi
    } else if h.ProviderId == D3D9 && id == ids::D3D9_PRESENT_START {
        Source::D3d9
    } else if h.ProviderId == DXGKRNL && ids::DXGKRNL_PRESENT_IDS.contains(&id) {
        Source::DxgKrnl
    } else {
        return;
    };
    let ev = PresentEvent { ts: h.TimeStamp, pid: h.ProcessId, source, event_id: id };
    if let Ok(mut v) = unsafe { &*ctx }.events.lock() {
        if v.len() < 20_000_000 {
            v.push(ev);
        }
    }
}

unsafe extern "system" fn on_ctrl(_ctrl: u32) -> windows::core::BOOL {
    stop_session_by_name();
    windows::core::BOOL(0) // let Windows end the process as usual
}

pub struct Capture {
    pub events: Vec<PresentEvent>,
    pub qpc_frequency: i64,
    pub events_lost: u32,
}

pub fn qpc_frequency() -> i64 {
    let mut f = 0i64;
    unsafe {
        let _ = QueryPerformanceFrequency(&mut f);
    }
    f
}

/// Run a capture session. `keep_going` is polled every 100 ms; return false
/// to stop early (for example when the game exits).
pub fn capture(seconds: u32, mut keep_going: impl FnMut() -> bool) -> Result<Capture> {
    let name = super::wide(SESSION_NAME);
    let mut p = props();
    let mut handle = CONTROLTRACE_HANDLE::default();
    unsafe {
        let _ = windows::Win32::System::Console::SetConsoleCtrlHandler(Some(on_ctrl), true);
        let mut rc = StartTraceW(&mut handle, PCWSTR(name.as_ptr()), &mut p.p);
        if rc == ERROR_ALREADY_EXISTS {
            stop_session_by_name();
            p = props();
            rc = StartTraceW(&mut handle, PCWSTR(name.as_ptr()), &mut p.p);
        }
        if rc != ERROR_SUCCESS {
            if rc.0 == 5 {
                bail!("Windows needs administrator rights to count frames. Right-click the terminal, choose \"Run as administrator\" and try again.");
            }
            bail!("could not start the frame-capture session (Windows error {})", rc.0);
        }
        for (guid, kw) in [
            (DXGI, ids::DXGI_KEYWORDS),
            (D3D9, ids::D3D9_KEYWORDS),
            (DXGKRNL, ids::DXGKRNL_KEYWORDS),
        ] {
            let rc = EnableTraceEx2(
                handle,
                &guid,
                EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
                TRACE_LEVEL_INFORMATION as u8,
                kw,
                0,
                0,
                None,
            );
            if rc != ERROR_SUCCESS {
                stop_session_by_name();
                bail!("could not enable a present-event provider (Windows error {})", rc.0);
            }
        }
    }

    let collector = Arc::new(Collector::default());
    let ctx_ptr = Arc::as_ptr(&collector) as usize;
    let mut logger_name = name.clone();
    let mut logfile = EVENT_TRACE_LOGFILEW {
        LoggerName: PWSTR(logger_name.as_mut_ptr()),
        Anonymous1: EVENT_TRACE_LOGFILEW_0 {
            ProcessTraceMode: PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD,
        },
        Anonymous2: EVENT_TRACE_LOGFILEW_1 { EventRecordCallback: Some(on_event) },
        Context: ctx_ptr as *mut _,
        ..Default::default()
    };
    let trace: PROCESSTRACE_HANDLE = unsafe { OpenTraceW(&mut logfile) };
    if trace.Value == u64::MAX {
        stop_session_by_name();
        bail!("could not open the frame-capture session for reading");
    }
    let reader = std::thread::spawn(move || unsafe {
        let _ = ProcessTrace(&[trace], None, None);
    });

    let start = std::time::Instant::now();
    while start.elapsed().as_secs_f64() < seconds as f64 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if !keep_going() {
            break;
        }
    }

    // Stopping the session flushes the last buffers to ProcessTrace, which
    // then returns.
    let mut stop = props();
    unsafe {
        let _ = ControlTraceW(handle, PCWSTR::null(), &mut stop.p, EVENT_TRACE_CONTROL_STOP);
    }
    let _ = reader.join();
    unsafe {
        let _ = CloseTrace(trace);
    }
    drop(logger_name);
    let events = std::mem::take(&mut *collector.events.lock().unwrap());
    Ok(Capture { events, qpc_frequency: qpc_frequency(), events_lost: stop.p.EventsLost + stop.p.RealTimeBuffersLost })
}
