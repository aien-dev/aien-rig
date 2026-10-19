//! Frame-time statistics: average FPS, 1% low, 0.1% low, percentiles and a
//! small graph series. Pure logic, no OS calls, tested on every platform.
//!
//! Definitions (written into every receipt so a reader knows what we mean):
//! - avg_fps: frames divided by the total time those frames took
//!   (1000 * n / sum(frame_ms)). This is NOT the mean of per-frame FPS,
//!   which overweights fast frames.
//! - low_1pct_fps: take the slowest 1% of frames (at least one frame), average
//!   their frame times, convert to FPS. Same idea for low_01pct_fps with 0.1%.
//! - p99_fps / p999_fps: the 99th / 99.9th percentile frame time (nearest
//!   rank), converted to FPS. Some tools call this the "1% low" instead; we
//!   report both so numbers can be compared with any tool.

use serde::{Deserialize, Serialize};

pub const STATS_DEFINITION: &str = "avg_fps = 1000*frames/sum(frame_ms). low_1pct_fps = 1000/mean(slowest 1% of frame times, at least 1 frame); low_01pct_fps same with 0.1%. p99_fps/p999_fps = 1000/(99th or 99.9th percentile frame time, nearest rank).";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameStats {
    pub frames: usize,
    pub seconds: f64,
    pub avg_fps: f64,
    pub low_1pct_fps: f64,
    pub low_01pct_fps: f64,
    pub p99_fps: f64,
    pub p999_fps: f64,
    pub frame_ms_avg: f64,
    pub frame_ms_median: f64,
    pub frame_ms_p99: f64,
    pub frame_ms_p999: f64,
    pub frame_ms_max: f64,
    /// Frames slower than 2.5x the median frame time. A plain stutter count.
    pub stutters: usize,
}

fn to_fps(ms: f64) -> f64 {
    if ms > 0.0 {
        1000.0 / ms
    } else {
        0.0
    }
}

/// Nearest-rank percentile on an ascending-sorted slice. `p` in 0..=100.
pub fn percentile_sorted(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let p = p.clamp(0.0, 100.0);
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    let idx = rank.max(1) - 1;
    sorted[idx.min(sorted.len() - 1)]
}

/// Mean of the slowest `fraction` of frames (fraction 0.01 = 1%), at least one frame.
pub fn mean_of_slowest(sorted_asc: &[f64], fraction: f64) -> f64 {
    if sorted_asc.is_empty() {
        return 0.0;
    }
    let k = ((sorted_asc.len() as f64) * fraction).ceil() as usize;
    let k = k.clamp(1, sorted_asc.len());
    let tail = &sorted_asc[sorted_asc.len() - k..];
    tail.iter().sum::<f64>() / k as f64
}

/// Compute statistics from frame times in milliseconds. Non-finite and
/// non-positive values are dropped (they cannot be real frames).
pub fn compute(frame_ms: &[f64]) -> Option<FrameStats> {
    let clean: Vec<f64> = frame_ms
        .iter()
        .copied()
        .filter(|v| v.is_finite() && *v > 0.0)
        .collect();
    if clean.is_empty() {
        return None;
    }
    let mut sorted = clean.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let total_ms: f64 = clean.iter().sum();
    let n = clean.len();
    let avg_ms = total_ms / n as f64;
    let median = percentile_sorted(&sorted, 50.0);
    let p99 = percentile_sorted(&sorted, 99.0);
    let p999 = percentile_sorted(&sorted, 99.9);
    let low1 = mean_of_slowest(&sorted, 0.01);
    let low01 = mean_of_slowest(&sorted, 0.001);
    let stutters = clean.iter().filter(|v| **v > median * 2.5).count();
    Some(FrameStats {
        frames: n,
        seconds: total_ms / 1000.0,
        avg_fps: to_fps(avg_ms),
        low_1pct_fps: to_fps(low1),
        low_01pct_fps: to_fps(low01),
        p99_fps: to_fps(p99),
        p999_fps: to_fps(p999),
        frame_ms_avg: avg_ms,
        frame_ms_median: median,
        frame_ms_p99: p99,
        frame_ms_p999: p999,
        frame_ms_max: *sorted.last().unwrap(),
        stutters,
    })
}

/// Downsample a frame-time series to at most `max_points` points for a graph.
/// Each point is [seconds since start, frame ms]; each bucket keeps its
/// slowest frame so stutters stay visible after downsampling.
pub fn graph(frame_ms: &[f64], max_points: usize) -> Vec<[f64; 2]> {
    let max_points = max_points.max(1);
    let mut out = Vec::new();
    if frame_ms.is_empty() {
        return out;
    }
    let bucket = frame_ms.len().div_ceil(max_points);
    let mut t_ms = 0.0;
    for chunk in frame_ms.chunks(bucket) {
        let mut worst = 0.0f64;
        let mut worst_t = t_ms;
        let mut t = t_ms;
        for v in chunk {
            t += v;
            if *v > worst {
                worst = *v;
                worst_t = t;
            }
        }
        out.push([round3(worst_t / 1000.0), round3(worst)]);
        t_ms = t;
    }
    out
}

pub fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn steady_60fps() {
        let ft = vec![1000.0 / 60.0; 600];
        let s = compute(&ft).unwrap();
        assert_eq!(s.frames, 600);
        assert!((s.avg_fps - 60.0).abs() < 1e-6);
        assert!((s.low_1pct_fps - 60.0).abs() < 1e-6);
        assert!((s.low_01pct_fps - 60.0).abs() < 1e-6);
        assert!((s.seconds - 10.0).abs() < 1e-6);
        assert_eq!(s.stutters, 0);
    }

    #[test]
    fn one_percent_low_is_mean_of_slowest_one_percent() {
        // 990 frames at 10 ms, 10 frames at 50 ms: slowest 1% (10 frames) = 50 ms.
        let mut ft = vec![10.0; 990];
        ft.extend(vec![50.0; 10]);
        let s = compute(&ft).unwrap();
        assert!(close(s.low_1pct_fps, 20.0));
        // 0.1% of 1000 = 1 frame = 50 ms.
        assert!(close(s.low_01pct_fps, 20.0));
        // avg = 1000*1000 / (9900+500) ms
        assert!(close(s.avg_fps, 1000.0 * 1000.0 / 10400.0));
        assert_eq!(s.stutters, 10);
        // 99th percentile nearest rank: rank 990 -> 10 ms (still fast)
        assert!(close(s.frame_ms_p99, 10.0));
        assert!(close(s.frame_ms_p999, 50.0));
    }

    #[test]
    fn avg_fps_weights_by_time_not_by_frame() {
        // One 100 ms frame and one 1 ms frame: 2 frames in 101 ms = 19.8 fps,
        // not the mean of 10 and 1000 fps.
        let s = compute(&[100.0, 1.0]).unwrap();
        assert!(close(s.avg_fps, 2000.0 / 101.0));
    }

    #[test]
    fn tiny_samples_use_at_least_one_frame() {
        let s = compute(&[10.0, 20.0, 30.0]).unwrap();
        assert!(close(s.low_1pct_fps, 1000.0 / 30.0));
        assert!(close(s.frame_ms_max, 30.0));
    }

    #[test]
    fn bad_values_are_dropped_and_empty_is_none() {
        assert!(compute(&[]).is_none());
        assert!(compute(&[0.0, -1.0, f64::NAN]).is_none());
        let s = compute(&[f64::INFINITY, 10.0]).unwrap();
        assert_eq!(s.frames, 1);
    }

    #[test]
    fn percentile_nearest_rank() {
        let v: Vec<f64> = (1..=100).map(|x| x as f64).collect();
        assert!(close(percentile_sorted(&v, 50.0), 50.0));
        assert!(close(percentile_sorted(&v, 99.0), 99.0));
        assert!(close(percentile_sorted(&v, 100.0), 100.0));
        assert!(close(percentile_sorted(&v, 0.0), 1.0));
    }

    #[test]
    fn graph_keeps_spikes() {
        let mut ft = vec![10.0; 1000];
        ft[500] = 80.0;
        let g = graph(&ft, 100);
        assert_eq!(g.len(), 100);
        assert!(g.iter().any(|p| close(p[1], 80.0)));
        let last = g.last().unwrap();
        assert!(last[0] <= 10.07 + 1e-9 && last[0] > 9.9);
        assert!(graph(&[], 10).is_empty());
        assert_eq!(graph(&[5.0; 7], 600).len(), 7);
    }
}
