//! Long-form transcription: 30 s windows, greedy decoding with Whisper's timestamp
//! rules, segments cut at timestamp pairs, the window advanced to the last complete
//! segment (the procedure in OpenAI whisper `transcribe.py` / `decoding.py`, temperature
//! 0 only).
use std::path::Path;
use std::time::Instant;

use serde_json::Value;

use crate::mel::{self, N_FRAMES};
use crate::model::{Timing, Whisper};
use crate::weights::Tokenizer;

const TIME_PRECISION: f64 = 0.02; // seconds per timestamp step
const INPUT_STRIDE: usize = 2; // mel frames per timestamp step
const FRAME_S: f64 = 0.01; // seconds per mel frame

pub struct Specials {
    pub eot: u32,
    pub sot: u32,
    pub transcribe: u32,
    pub no_timestamps: u32,
    pub ts_begin: u32,
    pub no_speech: Option<u32>,
    pub languages: Vec<(u32, String)>,
    pub suppress: Vec<u32>,
    pub begin_suppress: Vec<u32>,
    pub max_initial_ts_index: u32,
}

impl Specials {
    pub fn load(tok: &Tokenizer, dir: &Path) -> Result<Self, String> {
        let need = |s: &str| tok.id(s).ok_or(format!("tokenizer has no {s}"));
        let eot = need("<|endoftext|>")?;
        let sot = need("<|startoftranscript|>")?;
        let translate = need("<|translate|>")?;
        let transcribe = need("<|transcribe|>")?;
        let no_timestamps = need("<|notimestamps|>")?;
        let ts_begin = need("<|0.00|>")?;
        if ts_begin != no_timestamps + 1 {
            return Err("unexpected token layout: <|0.00|> does not follow <|notimestamps|>".into());
        }
        let no_speech = tok.id("<|nospeech|>").or_else(|| tok.id("<|nocaptions|>"));
        let languages = (sot + 1..translate)
            .filter_map(|id| tok.special_by_id.get(&id).map(|s| (id, s.trim_start_matches("<|").trim_end_matches("|>").to_string())))
            .collect();
        let gen: Value = std::fs::read_to_string(dir.join("generation_config.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        let list = |k: &str| -> Vec<u32> {
            gen[k].as_array().map(|a| a.iter().filter_map(|x| x.as_u64().map(|v| v as u32)).collect()).unwrap_or_default()
        };
        let mut suppress = list("suppress_tokens");
        // Always suppress the task and control tokens (whisper's SuppressTokens does).
        for s in ["<|translate|>", "<|transcribe|>", "<|startoftranscript|>", "<|startofprev|>", "<|startoflm|>", "<|nospeech|>", "<|nocaptions|>"] {
            if let Some(id) = tok.id(s) {
                suppress.push(id);
            }
        }
        let mut begin_suppress = list("begin_suppress_tokens");
        if begin_suppress.is_empty() {
            begin_suppress = vec![220, eot];
        }
        let max_initial_ts_index = gen["max_initial_timestamp_index"].as_u64().unwrap_or(50) as u32;
        Ok(Self { eot, sot, transcribe, no_timestamps, ts_begin, no_speech, languages, suppress, begin_suppress, max_initial_ts_index })
    }

    pub fn language_token(&self, code: &str) -> Option<u32> {
        self.languages.iter().find(|(_, c)| c == code).map(|(id, _)| *id)
    }
}

#[derive(Clone, Debug)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

pub struct Options {
    pub language: Option<String>,
    pub no_speech_threshold: f32,
    pub logprob_threshold: f32,
    pub verbose: bool,
}

pub struct Transcript {
    pub language: String,
    pub segments: Vec<Segment>,
    pub timing: Timing,
}

fn log_softmax(x: &[f32]) -> Vec<f32> {
    let m = x.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let s: f64 = x.iter().map(|&v| ((v - m) as f64).exp()).sum();
    let ls = m as f64 + s.ln();
    x.iter().map(|&v| (v as f64 - ls) as f32).collect()
}

/// Whisper's logit filters for one step: SuppressBlank, SuppressTokens,
/// ApplyTimestampRules. `sampled` = tokens generated so far in this window.
pub fn apply_filters(logits: &mut [f32], sampled: &[u32], sp: &Specials) {
    let ninf = f32::NEG_INFINITY;
    if sampled.is_empty() {
        for &t in &sp.begin_suppress {
            logits[t as usize] = ninf;
        }
    }
    for &t in &sp.suppress {
        if (t as usize) < logits.len() {
            logits[t as usize] = ninf;
        }
    }
    let tb = sp.ts_begin as usize;
    logits[sp.no_timestamps as usize] = ninf;
    let last_ts = sampled.last().map(|&t| t >= sp.ts_begin).unwrap_or(false);
    let penult_ts = sampled.len() < 2 || sampled[sampled.len() - 2] >= sp.ts_begin;
    if last_ts {
        if penult_ts {
            for v in &mut logits[tb..] {
                *v = ninf;
            }
        } else {
            for v in &mut logits[..sp.eot as usize] {
                *v = ninf;
            }
        }
    }
    if let Some(&last) = sampled.iter().rev().find(|&&t| t >= sp.ts_begin) {
        let upto = if last_ts && !penult_ts { last } else { last + 1 } as usize;
        let upto = upto.min(logits.len());
        for v in &mut logits[tb..upto] {
            *v = ninf;
        }
    }
    if sampled.is_empty() {
        for v in &mut logits[..tb] {
            *v = ninf;
        }
        let last_allowed = tb + sp.max_initial_ts_index as usize;
        let from = (last_allowed + 1).min(logits.len());
        for v in &mut logits[from..] {
            *v = ninf;
        }
    }
    let lp = log_softmax(logits);
    let ts_lse = {
        let m = lp[tb..].iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        if m == f32::NEG_INFINITY {
            m
        } else {
            m + lp[tb..].iter().map(|&v| (v - m).exp()).sum::<f32>().ln()
        }
    };
    let max_text = lp[..tb].iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    if ts_lse > max_text {
        for v in &mut logits[..tb] {
            *v = ninf;
        }
    }
}

fn argmax(x: &[f32]) -> usize {
    let mut best = 0;
    for i in 1..x.len() {
        if x[i] > x[best] {
            best = i;
        }
    }
    best
}

struct WindowResult {
    tokens: Vec<u32>, // without EOT
    avg_logprob: f32,
    no_speech_prob: f32,
}

fn detect_language(model: &Whisper, enc: &[f32], sp: &Specials) -> String {
    let mut st = model.new_state(enc);
    let h = model.decode(&mut st, &[sp.sot]);
    let logits = model.logits(&h);
    let best = sp.languages.iter().max_by(|a, b| logits[a.0 as usize].total_cmp(&logits[b.0 as usize])).unwrap();
    best.1.clone()
}

fn decode_window(model: &Whisper, enc: &[f32], sp: &Specials, lang_tok: u32, timing: &mut Timing) -> WindowResult {
    let t0 = Instant::now();
    let d = model.cfg.d;
    let mut st = model.new_state(enc);
    let prompt = [sp.sot, lang_tok, sp.transcribe];
    let h = model.decode(&mut st, &prompt);
    let no_speech_prob = match sp.no_speech {
        Some(ns) => {
            let l = model.logits(&h[..d]);
            log_softmax(&l)[ns as usize].exp()
        }
        None => 0.0,
    };
    let mut logits = model.logits(&h[(prompt.len() - 1) * d..]);
    let sample_len = model.cfg.n_text_ctx / 2;
    let mut tokens: Vec<u32> = Vec::new();
    let mut sum_lp = 0f64;
    let mut ended = false;
    for _ in 0..sample_len {
        apply_filters(&mut logits, &tokens, sp);
        let next = argmax(&logits) as u32;
        let lp = log_softmax(&logits)[next as usize];
        sum_lp += lp as f64;
        timing.decoder_tokens += 1;
        if next == sp.eot {
            ended = true;
            break;
        }
        tokens.push(next);
        if st.len >= model.cfg.n_text_ctx {
            break;
        }
        let h = model.decode(&mut st, &[next]);
        logits = model.logits(&h);
    }
    let _ = ended;
    timing.decoder_s += t0.elapsed().as_secs_f64();
    WindowResult { avg_logprob: (sum_lp / (tokens.len() + 1) as f64) as f32, tokens, no_speech_prob }
}

pub fn transcribe(model: &Whisper, tok: &Tokenizer, sp: &Specials, mel_all: &[f32], frames: usize, opts: &Options) -> Transcript {
    let nm = model.cfg.n_mels;
    let content_frames = frames.saturating_sub(N_FRAMES);
    let mut timing = Timing::default();
    let mut segments = Vec::new();
    let mut language = opts.language.clone();
    let mut seek = 0usize;
    let ts = |t: u32| (t - sp.ts_begin) as f64;
    while seek < content_frames {
        let segment_size = N_FRAMES.min(content_frames - seek);
        let time_offset = seek as f64 * FRAME_S;
        let segment_duration = segment_size as f64 * FRAME_S;
        let win = mel::window(mel_all, nm, frames, seek, segment_size);
        let enc = model.encode(&win, &mut timing);
        timing.windows += 1;
        let lang = match &language {
            Some(l) => l.clone(),
            None => {
                let l = detect_language(model, &enc, sp);
                if opts.verbose {
                    eprintln!("detected language: {l}");
                }
                language = Some(l.clone());
                l
            }
        };
        let lang_tok = sp.language_token(&lang).unwrap_or_else(|| sp.language_token("en").unwrap());
        let r = decode_window(model, &enc, sp, lang_tok, &mut timing);
        if r.no_speech_prob > opts.no_speech_threshold && r.avg_logprob < opts.logprob_threshold {
            if opts.verbose {
                eprintln!("[{time_offset:.2}] no speech (p={:.2}), skipping {segment_duration:.1} s", r.no_speech_prob);
            }
            seek += segment_size;
            continue;
        }
        let toks = &r.tokens;
        let is_ts: Vec<bool> = toks.iter().map(|&t| t >= sp.ts_begin).collect();
        let single_ending = toks.len() >= 2 && !is_ts[toks.len() - 2] && is_ts[toks.len() - 1]
            || toks.len() == 1 && is_ts[0];
        let consecutive: Vec<usize> = (1..toks.len()).filter(|&i| is_ts[i - 1] && is_ts[i]).collect();
        let mut add = |start: f64, end: f64, text_toks: &[u32]| {
            let text_ids: Vec<u32> = text_toks.iter().cloned().filter(|&t| t < sp.eot).collect();
            let text = tok.decode(&text_ids).trim().to_string();
            if !text.is_empty() && end > start {
                if opts.verbose {
                    eprintln!("[{} --> {}] {}", hms(start), hms(end), text);
                }
                segments.push(Segment { start, end, text });
            }
        };
        if !consecutive.is_empty() {
            let mut slices = consecutive.clone();
            if single_ending {
                slices.push(toks.len());
            }
            let mut last = 0;
            for &cur in &slices {
                let sl = &toks[last..cur];
                if sl.len() >= 2 && sl[0] >= sp.ts_begin && sl[sl.len() - 1] >= sp.ts_begin {
                    add(time_offset + ts(sl[0]) * TIME_PRECISION, time_offset + ts(sl[sl.len() - 1]) * TIME_PRECISION, &sl[1..sl.len() - 1]);
                } else if !sl.is_empty() {
                    add(time_offset, time_offset + segment_duration, sl);
                }
                last = cur;
            }
            if single_ending {
                seek += segment_size;
            } else {
                let last_pos = (toks[last - 1] - sp.ts_begin) as usize;
                seek += (last_pos * INPUT_STRIDE).max(1);
            }
        } else {
            let mut duration = segment_duration;
            if let Some(&t) = toks.iter().rev().find(|&&t| t >= sp.ts_begin) {
                if t != sp.ts_begin {
                    duration = ts(t) * TIME_PRECISION;
                }
            }
            add(time_offset, time_offset + duration, toks);
            seek += segment_size;
        }
    }
    Transcript { language: language.unwrap_or_else(|| "en".into()), segments, timing }
}

pub fn hms(t: f64) -> String {
    let ms = (t * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}.{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// Transcript JSON for the creator tool.
pub fn to_json(t: &Transcript) -> Value {
    let segs: Vec<Value> = t
        .segments
        .iter()
        .map(|s| serde_json::json!({"start": round3(s.start), "end": round3(s.end), "text": s.text}))
        .collect();
    serde_json::json!({"segments": segs, "language": t.language})
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp() -> Specials {
        Specials {
            eot: 50257,
            sot: 50258,
            transcribe: 50360,
            no_timestamps: 50364,
            ts_begin: 50365,
            no_speech: Some(50363),
            languages: vec![(50259, "en".into())],
            suppress: vec![],
            begin_suppress: vec![220, 50257],
            max_initial_ts_index: 50,
        }
    }

    #[test]
    fn first_token_must_be_an_early_timestamp() {
        let s = sp();
        let mut l = vec![0f32; 51866];
        l[100] = 10.0; // a strong text token
        apply_filters(&mut l, &[], &s);
        let best = argmax(&l) as u32;
        assert!(best >= s.ts_begin && best <= s.ts_begin + 50, "got {best}");
    }

    #[test]
    fn timestamps_come_in_pairs_and_never_go_back() {
        let s = sp();
        // after "<|1.00|> text" the next timestamp must be >= 1.00
        let mut l = vec![0f32; 51866];
        l[(s.ts_begin + 10) as usize] = 50.0; // earlier timestamp: forbidden
        apply_filters(&mut l, &[s.ts_begin + 50, 300], &s);
        assert_eq!(l[(s.ts_begin + 10) as usize], f32::NEG_INFINITY);
        // after a single closing timestamp, text is forbidden (must be ts or eot)
        let mut l = vec![0f32; 51866];
        l[300] = 50.0;
        apply_filters(&mut l, &[s.ts_begin, 300, s.ts_begin + 20], &s);
        assert_eq!(l[300], f32::NEG_INFINITY);
        // after a pair, a timestamp is forbidden (must be text)
        let mut l = vec![0f32; 51866];
        apply_filters(&mut l, &[s.ts_begin, 300, s.ts_begin + 20, s.ts_begin + 20], &s);
        assert!(l[s.ts_begin as usize..].iter().all(|&v| v == f32::NEG_INFINITY));
    }
}
