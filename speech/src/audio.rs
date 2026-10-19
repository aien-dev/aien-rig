//! Audio input: any audio or video file symphonia can open (wav, flac, mp3, ogg/vorbis,
//! mp4/m4a/mov with AAC or ALAC, mkv/webm with AAC/FLAC/Vorbis/PCM), first audio track,
//! downmixed to mono and resampled to 16 kHz with a windowed-sinc filter.
use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const SAMPLE_RATE: u32 = 16_000;

pub struct Decoded {
    /// Mono samples at the source rate.
    pub samples: Vec<f32>,
    pub source_rate: u32,
    pub source_channels: usize,
    pub codec: String,
}

/// Decode the first playable audio track to mono f32 at its own sample rate.
pub fn decode_mono(path: &Path) -> Result<Decoded, String> {
    let file = File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("{}: not a media file symphonia can read: {e}", path.display()))?;
    let mut format = probed.format;
    let codecs = symphonia::default::get_codecs();
    // First track that is audio (has a sample rate) and that we can decode. Video
    // tracks in mp4/mkv/mov are skipped.
    let mut chosen = None;
    for t in format.tracks() {
        if t.codec_params.codec == CODEC_TYPE_NULL || t.codec_params.sample_rate.is_none() {
            continue;
        }
        if let Ok(dec) = codecs.make(&t.codec_params, &DecoderOptions::default()) {
            let name = codecs
                .get_codec(t.codec_params.codec)
                .map(|d| d.short_name.to_string())
                .unwrap_or_else(|| "?".into());
            chosen = Some((t.id, t.codec_params.sample_rate.unwrap(), dec, name));
            break;
        }
    }
    let (track_id, mut rate, mut decoder, codec) = chosen.ok_or_else(|| {
        format!(
            "{}: no audio track we can decode (supported: AAC, ALAC, FLAC, MP3, Vorbis, PCM, ADPCM; Opus is not supported yet)",
            path.display()
        )
    })?;
    let mut mono = Vec::new();
    let mut channels = 0usize;
    let mut buf: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => return Err(format!("{}: read error: {e}", path.display())),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymError::DecodeError(_)) => continue, // skip a corrupt packet, as players do
            Err(SymError::IoError(_)) => continue,
            Err(e) => return Err(format!("{}: decode error: {e}", path.display())),
        };
        let spec = *decoded.spec();
        rate = spec.rate;
        let ch = spec.channels.count().max(1);
        channels = ch;
        let need = decoded.capacity() as u64;
        if buf.as_ref().map(|b| (b.capacity() as u64) < need * ch as u64).unwrap_or(true) {
            buf = Some(SampleBuffer::<f32>::new(need, spec));
        }
        let b = buf.as_mut().unwrap();
        b.copy_interleaved_ref(decoded);
        let s = b.samples();
        let inv = 1.0 / ch as f32;
        for frame in s.chunks_exact(ch) {
            mono.push(frame.iter().sum::<f32>() * inv);
        }
    }
    if mono.is_empty() {
        return Err(format!("{}: the audio track decoded to zero samples", path.display()));
    }
    Ok(Decoded { samples: mono, source_rate: rate, source_channels: channels, codec })
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// One filter tap: windowed sinc at distance `d` source samples.
fn tap(d: f64, cutoff: f64, half_width: f64) -> f64 {
    if d.abs() >= half_width {
        return 0.0;
    }
    let arg = std::f64::consts::PI * d * cutoff;
    let sinc = if arg.abs() < 1e-12 { 1.0 } else { arg.sin() / arg };
    // Blackman-Harris over [-half_width, half_width]
    let u = (d / half_width + 1.0) * 0.5; // 0..1
    let tau = 2.0 * std::f64::consts::PI;
    let w = 0.35875 - 0.48829 * (tau * u).cos() + 0.14128 * (2.0 * tau * u).cos()
        - 0.01168 * (3.0 * tau * u).cos();
    sinc * w * cutoff
}

/// Polyphase windowed-sinc resampler (Blackman-Harris window, 32 zero crossings each
/// side). Cutoff at 0.95 of the lower Nyquist so downsampling does not alias. The
/// filter for every output phase is computed once (48 kHz to 16 kHz has 1 phase,
/// 44.1 kHz to 16 kHz has 160).
pub fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    use rayon::prelude::*;
    if from == to {
        return x.to_vec();
    }
    let (from64, to64) = (from as u64, to as u64);
    let g = gcd(from64, to64);
    let (up, down) = (to64 / g, from64 / g); // output n sits at source n*down/up
    let ratio = to as f64 / from as f64;
    let cutoff = 0.95 * ratio.min(1.0); // in units of the source Nyquist
    let half_width = 32.0 / cutoff; // in source samples
    let w = half_width.ceil() as i64;
    let ntaps = (2 * w + 1) as usize;
    assert!(up <= 1 << 16, "sample rate ratio {from}:{to} has too many phases");
    // phase p: fractional offset p/up; taps for source index ip - w + i
    let table: Vec<f32> = (0..up)
        .flat_map(|p| {
            let frac = p as f64 / up as f64;
            (0..ntaps).map(move |i| {
                let j = i as i64 - w;
                tap(frac - j as f64, cutoff, half_width) as f32
            })
        })
        .collect();
    let out_len = (x.len() as u64 * up / down) as usize;
    let mut out = vec![0f32; out_len];
    out.par_chunks_mut(4096).enumerate().for_each(|(ci, chunk)| {
        for (i, o) in chunk.iter_mut().enumerate() {
            let n = (ci * 4096 + i) as u64;
            let ip = (n * down / up) as i64;
            let p = (n * down % up) as usize;
            let taps = &table[p * ntaps..(p + 1) * ntaps];
            let mut acc = 0f32;
            let base = ip - w;
            if base >= 0 && base + ntaps as i64 <= x.len() as i64 {
                let xs = &x[base as usize..base as usize + ntaps];
                for (a, b) in xs.iter().zip(taps) {
                    acc += a * b;
                }
            } else {
                for (k, t) in taps.iter().enumerate() {
                    let j = base + k as i64;
                    if j >= 0 && (j as usize) < x.len() {
                        acc += x[j as usize] * t;
                    }
                }
            }
            *o = acc;
        }
    });
    out
}

/// Decode a file and return 16 kHz mono samples plus what we found.
pub fn load_16k(path: &Path) -> Result<(Vec<f32>, Decoded), String> {
    let mut d = decode_mono(path)?;
    let s = resample(&d.samples, d.source_rate, SAMPLE_RATE);
    d.samples = Vec::new();
    Ok((s, d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_keeps_a_low_tone_and_its_length() {
        let from = 48_000u32;
        let f = 440.0f64;
        let x: Vec<f32> = (0..from as usize)
            .map(|n| (2.0 * std::f64::consts::PI * f * n as f64 / from as f64).sin() as f32)
            .collect();
        let y = resample(&x, from, 16_000);
        assert_eq!(y.len(), 16_000);
        // Compare the middle against the ideal 16 kHz tone.
        let mut err = 0f64;
        for n in 1000..15000 {
            let ideal = (2.0 * std::f64::consts::PI * f * n as f64 / 16_000.0).sin();
            err = err.max((y[n] as f64 - ideal).abs());
        }
        assert!(err < 2e-3, "max error {err}");
    }

    #[test]
    fn resample_removes_tone_above_new_nyquist() {
        let from = 48_000u32;
        let f = 12_000.0f64; // above 8 kHz: must vanish
        let x: Vec<f32> = (0..from as usize)
            .map(|n| (2.0 * std::f64::consts::PI * f * n as f64 / from as f64).sin() as f32)
            .collect();
        let y = resample(&x, from, 16_000);
        let peak = y[1000..15000].iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak < 1e-3, "alias peak {peak}");
    }
}
