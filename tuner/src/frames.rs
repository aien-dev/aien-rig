//! Turning raw ETW present events into one clean frame-time series.
//!
//! The ETW collector (Windows only) records every present event it sees as a
//! `PresentEvent`. This module decides which events count as frames for the
//! game, without double counting, and converts timestamps into frame times.
//! Pure logic, tested on every platform.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where a present event came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Microsoft-Windows-DXGI Present_Start (event 42). Direct3D 10/11/12 and
    /// Vulkan/OpenGL games that the NVIDIA driver layers on a DXGI swap chain.
    Dxgi,
    /// Microsoft-Windows-D3D9 Present_Start (event 1). Older Direct3D 9 games.
    D3d9,
    /// Microsoft-Windows-DxgKrnl kernel present events. Catches every API,
    /// used only when the user-mode providers saw nothing for the game.
    DxgKrnl,
}

pub mod ids {
    pub const DXGI_PROVIDER: &str = "CA11C036-0102-4A2D-A6AD-F03CFED5D3C9";
    pub const D3D9_PROVIDER: &str = "783ACA0A-790E-4D7F-8451-AA850511C6B9";
    pub const DXGKRNL_PROVIDER: &str = "802EC45A-1E99-4B83-9920-87C98277BA9D";
    pub const DXGI_PRESENT_START: u16 = 42;
    pub const D3D9_PRESENT_START: u16 = 1;
    pub const DXGKRNL_BLIT_INFO: u16 = 166;
    pub const DXGKRNL_FLIP_INFO: u16 = 168;
    pub const DXGKRNL_PRESENTHISTORY_START: u16 = 171;
    pub const DXGKRNL_PRESENT_INFO: u16 = 184;
    pub const DXGKRNL_PRESENTHISTORYDETAILED_START: u16 = 215;
    /// Keywords, from the providers' manifests.
    pub const DXGI_KEYWORDS: u64 = 0x8000_0000_0000_0002;
    pub const D3D9_KEYWORDS: u64 = 0x8000_0000_0000_0002;
    pub const DXGKRNL_KEYWORDS: u64 = 0x4000_0000_0800_0001;
    /// DxgKrnl events that each mark one present. A single present can raise
    /// several of these, so we only ever count one id per process.
    pub const DXGKRNL_PRESENT_IDS: [u16; 5] = [
        DXGKRNL_PRESENT_INFO,
        DXGKRNL_PRESENTHISTORYDETAILED_START,
        DXGKRNL_PRESENTHISTORY_START,
        DXGKRNL_FLIP_INFO,
        DXGKRNL_BLIT_INFO,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentEvent {
    /// Raw QPC timestamp from the ETW event header.
    pub ts: i64,
    pub pid: u32,
    pub source: Source,
    pub event_id: u16,
}

/// The frames chosen for the game: which source/process/event they came from
/// and their timestamps in ascending order.
#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    pub source: Source,
    pub pid: u32,
    pub event_id: u16,
    pub timestamps: Vec<i64>,
    /// Counts of every (source, pid, event id) we saw for the game's processes,
    /// written into the receipt so a reader can check the choice.
    pub seen: Vec<(Source, u32, u16, usize)>,
}

/// Pick the frame series for the game.
///
/// Rules: only events from `pids` count. Prefer DXGI, then D3D9, then DxgKrnl
/// (first source with at least 2 events). Within that source pick the single
/// (pid, event id) with the most events, so a frame is never counted twice and
/// two windows are never interleaved.
pub fn select(events: &[PresentEvent], pids: &[u32]) -> Option<Selection> {
    let mut counts: BTreeMap<(Source, u32, u16), Vec<i64>> = BTreeMap::new();
    for e in events {
        if !pids.contains(&e.pid) {
            continue;
        }
        let wanted = match e.source {
            Source::Dxgi => e.event_id == ids::DXGI_PRESENT_START,
            Source::D3d9 => e.event_id == ids::D3D9_PRESENT_START,
            Source::DxgKrnl => ids::DXGKRNL_PRESENT_IDS.contains(&e.event_id),
        };
        if wanted {
            counts.entry((e.source, e.pid, e.event_id)).or_default().push(e.ts);
        }
    }
    let seen: Vec<(Source, u32, u16, usize)> = counts
        .iter()
        .map(|((s, p, i), v)| (*s, *p, *i, v.len()))
        .collect();
    for source in [Source::Dxgi, Source::D3d9, Source::DxgKrnl] {
        let best = counts
            .iter()
            .filter(|((s, _, _), v)| *s == source && v.len() >= 2)
            .max_by(|a, b| {
                a.1.len().cmp(&b.1.len()).then_with(|| {
                    // tie-break: prefer the earlier id in DXGKRNL_PRESENT_IDS order
                    let ra = rank(a.0 .2);
                    let rb = rank(b.0 .2);
                    rb.cmp(&ra)
                })
            });
        if let Some(((s, pid, id), ts)) = best {
            let mut ts = ts.clone();
            ts.sort_unstable();
            return Some(Selection {
                source: *s,
                pid: *pid,
                event_id: *id,
                timestamps: ts,
                seen,
            });
        }
    }
    None
}

fn rank(id: u16) -> usize {
    ids::DXGKRNL_PRESENT_IDS
        .iter()
        .position(|x| *x == id)
        .unwrap_or(usize::MAX)
}

/// Convert ascending QPC timestamps into frame times in milliseconds.
pub fn frame_times_ms(timestamps: &[i64], qpc_freq: i64) -> Vec<f64> {
    if qpc_freq <= 0 {
        return Vec::new();
    }
    timestamps
        .windows(2)
        .map(|w| (w[1] - w[0]) as f64 * 1000.0 / qpc_freq as f64)
        .filter(|ms| *ms > 0.0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(ts: i64, pid: u32, source: Source, id: u16) -> PresentEvent {
        PresentEvent { ts, pid, source, event_id: id }
    }

    #[test]
    fn prefers_dxgi_and_filters_pid() {
        let mut v = Vec::new();
        for i in 0..10 {
            v.push(ev(i * 100, 7, Source::Dxgi, 42));
            v.push(ev(i * 100 + 1, 7, Source::DxgKrnl, 184));
            v.push(ev(i * 50, 99, Source::Dxgi, 42)); // other process
        }
        let s = select(&v, &[7]).unwrap();
        assert_eq!(s.source, Source::Dxgi);
        assert_eq!(s.pid, 7);
        assert_eq!(s.timestamps.len(), 10);
        assert_eq!(s.seen.len(), 2);
    }

    #[test]
    fn falls_back_to_dxgkrnl_without_double_counting() {
        let mut v = Vec::new();
        for i in 0..20 {
            v.push(ev(i * 100, 5, Source::DxgKrnl, 184));
            v.push(ev(i * 100 + 2, 5, Source::DxgKrnl, 215));
        }
        v.push(ev(5000, 5, Source::DxgKrnl, 999)); // unrelated id ignored
        let s = select(&v, &[5]).unwrap();
        assert_eq!(s.source, Source::DxgKrnl);
        assert_eq!(s.timestamps.len(), 20);
        assert_eq!(s.event_id, 184); // tie broken by preference order
    }

    #[test]
    fn picks_busiest_process_when_game_has_several() {
        let mut v = Vec::new();
        for i in 0..5 {
            v.push(ev(i, 1, Source::Dxgi, 42)); // launcher window
        }
        for i in 0..50 {
            v.push(ev(i, 2, Source::Dxgi, 42)); // the game
        }
        let s = select(&v, &[1, 2]).unwrap();
        assert_eq!(s.pid, 2);
    }

    #[test]
    fn nothing_seen_is_none() {
        assert!(select(&[], &[1]).is_none());
        assert!(select(&[ev(1, 1, Source::Dxgi, 42)], &[1]).is_none());
    }

    #[test]
    fn timestamps_to_frame_times() {
        // 10 MHz QPC, frames 1/100 s apart -> 10 ms each; sorted input required.
        let ts = vec![0, 100_000, 200_000, 300_000];
        let ft = frame_times_ms(&ts, 10_000_000);
        assert_eq!(ft, vec![10.0, 10.0, 10.0]);
        assert!(frame_times_ms(&ts, 0).is_empty());
    }

    #[test]
    fn select_sorts_out_of_order_events() {
        let v = vec![
            ev(300, 1, Source::Dxgi, 42),
            ev(100, 1, Source::Dxgi, 42),
            ev(200, 1, Source::Dxgi, 42),
        ];
        let s = select(&v, &[1]).unwrap();
        assert_eq!(s.timestamps, vec![100, 200, 300]);
    }
}
