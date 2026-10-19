//! Whisper log-mel features, written from the published recipe (OpenAI whisper
//! `audio.py` and librosa's Slaney mel filter bank):
//! 400-point STFT (periodic Hann window, hop 160, reflect padding of 200 at both ends,
//! last frame dropped), power spectrum, Slaney-normalised mel filters, log10 clamped at
//! 1e-10, floored at (global max - 8), then (x + 4) / 4.
use rayon::prelude::*;
use rustfft::{num_complex::Complex32, FftPlanner};

pub const N_FFT: usize = 400;
pub const HOP: usize = 160;
pub const N_SAMPLES: usize = 480_000; // 30 s at 16 kHz
pub const N_FRAMES: usize = 3000; // mel frames in one 30 s window

fn hz_to_mel(f: f64) -> f64 {
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1000.0;
    let min_log_mel = min_log_hz / f_sp;
    let logstep = (6.4f64).ln() / 27.0;
    if f >= min_log_hz {
        min_log_mel + (f / min_log_hz).ln() / logstep
    } else {
        f / f_sp
    }
}

fn mel_to_hz(m: f64) -> f64 {
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1000.0;
    let min_log_mel = min_log_hz / f_sp;
    let logstep = (6.4f64).ln() / 27.0;
    if m >= min_log_mel {
        min_log_hz * (logstep * (m - min_log_mel)).exp()
    } else {
        f_sp * m
    }
}

/// librosa.filters.mel(sr=16000, n_fft=400, n_mels, fmin=0, fmax=8000, htk=False,
/// norm="slaney"). Row-major [n_mels, 201].
pub fn mel_filters(n_mels: usize) -> Vec<f32> {
    let sr = 16000.0f64;
    let n_bins = N_FFT / 2 + 1;
    let fft_freqs: Vec<f64> = (0..n_bins).map(|k| k as f64 * sr / N_FFT as f64).collect();
    let (mmin, mmax) = (hz_to_mel(0.0), hz_to_mel(sr / 2.0));
    let hz: Vec<f64> = (0..n_mels + 2)
        .map(|i| mel_to_hz(mmin + (mmax - mmin) * i as f64 / (n_mels + 1) as f64))
        .collect();
    let mut w = vec![0f32; n_mels * n_bins];
    for m in 0..n_mels {
        let enorm = 2.0 / (hz[m + 2] - hz[m]);
        for (k, &f) in fft_freqs.iter().enumerate() {
            let lower = (f - hz[m]) / (hz[m + 1] - hz[m]);
            let upper = (hz[m + 2] - f) / (hz[m + 2] - hz[m + 1]);
            let v = lower.min(upper).max(0.0);
            w[m * n_bins + k] = (v * enorm) as f32;
        }
    }
    w
}

/// Log-mel spectrogram of `samples` (16 kHz mono) with 30 s of zeros appended, as
/// whisper's transcribe() does. Returns ([n_mels, frames] row-major, frames).
pub fn log_mel(samples: &[f32], n_mels: usize) -> (Vec<f32>, usize) {
    let mut audio = Vec::with_capacity(samples.len() + N_SAMPLES);
    audio.extend_from_slice(samples);
    audio.resize(samples.len() + N_SAMPLES, 0.0);
    log_mel_raw(&audio, n_mels)
}

/// Log-mel of exactly `audio` (no padding added).
pub fn log_mel_raw(audio: &[f32], n_mels: usize) -> (Vec<f32>, usize) {
    let n = audio.len();
    let pad = N_FFT / 2;
    // reflect padding (torch.stft center=True, pad_mode="reflect")
    let mut padded = Vec::with_capacity(n + 2 * pad);
    for i in (1..=pad).rev() {
        padded.push(audio[i.min(n - 1)]);
    }
    padded.extend_from_slice(audio);
    for i in 0..pad {
        padded.push(audio[n.saturating_sub(2 + i)]);
    }
    let frames = 1 + n / HOP - 1; // torch.stft gives 1 + n/HOP frames; whisper drops the last
    let window: Vec<f32> = (0..N_FFT)
        .map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / N_FFT as f64).cos()) as f32)
        .collect();
    let filters = mel_filters(n_mels);
    let n_bins = N_FFT / 2 + 1;
    let fft = FftPlanner::<f32>::new().plan_fft_forward(N_FFT);
    // frame-major first ([frames, n_mels]), transposed at the end
    let mut fm = vec![0f32; frames * n_mels];
    fm.par_chunks_mut(n_mels * 64).enumerate().for_each(|(ci, chunk)| {
        let mut buf = vec![Complex32::new(0.0, 0.0); N_FFT];
        let mut scratch = vec![Complex32::new(0.0, 0.0); fft.get_inplace_scratch_len()];
        let mut power = vec![0f32; n_bins];
        for (fi, out) in chunk.chunks_mut(n_mels).enumerate() {
            let t = ci * 64 + fi;
            let start = t * HOP;
            for i in 0..N_FFT {
                buf[i] = Complex32::new(padded[start + i] * window[i], 0.0);
            }
            fft.process_with_scratch(&mut buf, &mut scratch);
            for k in 0..n_bins {
                power[k] = buf[k].norm_sqr();
            }
            for m in 0..n_mels {
                let row = &filters[m * n_bins..(m + 1) * n_bins];
                let mut acc = 0f32;
                for k in 0..n_bins {
                    acc += row[k] * power[k];
                }
                out[m] = acc.max(1e-10).log10();
            }
        }
    });
    let gmax = fm.par_iter().cloned().reduce(|| f32::NEG_INFINITY, f32::max);
    let floor = gmax - 8.0;
    let mut mel = vec![0f32; n_mels * frames];
    for t in 0..frames {
        for m in 0..n_mels {
            mel[m * frames + t] = (fm[t * n_mels + m].max(floor) + 4.0) / 4.0;
        }
    }
    (mel, frames)
}

/// mel[:, seek:seek+len] zero-padded (or cut) to N_FRAMES, as whisper's pad_or_trim.
pub fn window(mel: &[f32], n_mels: usize, frames: usize, seek: usize, len: usize) -> Vec<f32> {
    let mut out = vec![0f32; n_mels * N_FRAMES];
    let len = len.min(N_FRAMES).min(frames.saturating_sub(seek));
    for m in 0..n_mels {
        out[m * N_FRAMES..m * N_FRAMES + len]
            .copy_from_slice(&mel[m * frames + seek..m * frames + seek + len]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slaney_mel_scale_round_trips_and_has_known_points() {
        for f in [0.0, 100.0, 999.0, 1000.0, 4000.0, 8000.0] {
            assert!((mel_to_hz(hz_to_mel(f)) - f).abs() < 1e-6);
        }
        assert!((hz_to_mel(1000.0) - 15.0).abs() < 1e-12);
    }

    #[test]
    fn filters_are_nonnegative_and_cover_the_band() {
        let w = mel_filters(80);
        assert!(w.iter().all(|&v| v >= 0.0));
        // every filter has some weight
        for m in 0..80 {
            assert!(w[m * 201..(m + 1) * 201].iter().sum::<f32>() > 0.0, "filter {m} empty");
        }
    }

    #[test]
    fn frame_count_matches_whisper() {
        let x = vec![0.0f32; 16000];
        let (_, frames) = log_mel(&x, 80);
        assert_eq!(frames, (16000 + N_SAMPLES) / HOP);
    }

    #[test]
    fn a_tone_lights_up_the_right_mel_band() {
        let f = 1000.0f64;
        let x: Vec<f32> = (0..N_SAMPLES)
            .map(|n| (0.5 * (2.0 * std::f64::consts::PI * f * n as f64 / 16000.0).sin()) as f32)
            .collect();
        let (mel, frames) = log_mel_raw(&x, 80);
        let t = frames / 2;
        let best = (0..80).max_by(|&a, &b| mel[a * frames + t].total_cmp(&mel[b * frames + t])).unwrap();
        // centre frequencies: the filter whose peak is nearest 1 kHz
        let hz: Vec<f64> = (0..82).map(|i| mel_to_hz(hz_to_mel(8000.0) * i as f64 / 81.0)).collect();
        let nearest = (0..80).min_by(|&a, &b| (hz[a + 1] - f).abs().total_cmp(&(hz[b + 1] - f).abs())).unwrap();
        assert!((best as i64 - nearest as i64).abs() <= 1, "best {best} nearest {nearest}");
    }
}
