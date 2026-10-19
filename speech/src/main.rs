//! rig-listen: transcribe a video or audio file to the creator transcript JSON.
use std::path::{Path, PathBuf};
use std::time::Instant;

use aien_rig_speech::{audio, mel, model, omega, transcribe, weights, wer};
use serde_json::{json, Value};

const USAGE: &str = "\
rig-listen: speech to text for AIEN Rig (Whisper-family model, our own runtime)

USAGE:
  rig-listen [OPTIONS] <video-or-audio-file>
  rig-listen wer <reference.txt> <transcript.json or .txt>
  rig-listen gpu-selftest            (Omega build: GPU vs CPU parity and speed)

OPTIONS:
  --model DIR        model folder (config.json, model.safetensors, tokenizer.json)
                     default: $RIG_SPEECH_MODEL, else ~/models/whisper-large-v3-turbo-41f01f3f
  --out FILE         transcript JSON (default: <input>.transcript.json)
  --language CODE    en, de, ... or auto (default: auto, detected from the first 30 s)
  --backend B        cpu or omega (omega = matrix multiplies on the GPU through Omega;
                     needs a build with --features omega)
  --threads N        CPU threads (default: all)
  --receipt FILE     write a speed receipt (JSON) here
  --machine LABEL    machine name for the receipt (for example GB10)
  -q, --quiet        no progress lines
";

fn die(msg: &str) -> ! {
    eprintln!("rig-listen: {msg}");
    std::process::exit(2)
}

fn sha256_of_model(dir: &Path) -> Option<String> {
    // ~/models/<name>.sha256 is written by speech/scripts/fetch-model.sh
    let side = PathBuf::from(format!("{}.sha256", dir.display()));
    let text = std::fs::read_to_string(side).ok()?;
    text.lines().find(|l| l.ends_with(" model.safetensors")).map(|l| l.split_whitespace().next().unwrap().to_string())
}

fn wer_cmd(args: &[String]) {
    if args.len() != 2 {
        die("usage: rig-listen wer <reference.txt> <transcript.json or .txt>");
    }
    let reference = std::fs::read_to_string(&args[0]).unwrap_or_else(|e| die(&format!("{}: {e}", args[0])));
    let hyp_raw = std::fs::read_to_string(&args[1]).unwrap_or_else(|e| die(&format!("{}: {e}", args[1])));
    let hyp = match serde_json::from_str::<Value>(&hyp_raw) {
        Ok(v) if v["segments"].is_array() => v["segments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["text"].as_str().unwrap_or("").to_string())
            .collect::<Vec<_>>()
            .join(" "),
        _ => hyp_raw,
    };
    let w = wer::wer(&reference, &hyp);
    println!(
        "{}",
        json!({
            "wer": (w.rate() * 10000.0).round() / 10000.0,
            "substitutions": w.substitutions,
            "deletions": w.deletions,
            "insertions": w.insertions,
            "reference_words": w.ref_words,
            "normaliser": "aien-rig-speech wer::normalize (lower case, punctuation removed)"
        })
    );
}

/// Parity and speed of the Omega GPU paths against our CPU paths, on the shapes the
/// whisper-large-v3-turbo encoder uses (1500 rows, d=1280, 20 heads of 64, MLP 5120).
fn gpu_selftest() {
    use aien_rig_speech::gemm::{matmul, PackedB};
    use std::time::Instant;
    fn rnd(n: usize, seed: u64, scale: f32) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (((s >> 33) as f32 / (1u64 << 31) as f32) - 0.5) * scale
            })
            .collect()
    }
    fn err(a: &[f32], b: &[f32]) -> (f32, f32) {
        let mut max_abs = 0f32;
        let mut max_ref = 0f32;
        for (x, y) in a.iter().zip(b) {
            max_abs = max_abs.max((x - y).abs());
            max_ref = max_ref.max(y.abs());
        }
        (max_abs, max_abs / max_ref.max(1e-30))
    }
    let mut results = Vec::new();
    for &(m, k, n) in &[(1500usize, 1280usize, 1280usize), (1500, 1280, 5120), (1500, 5120, 1280), (3000, 384, 1280)] {
        let a = rnd(m * k, 1 + m as u64 + k as u64, 2.0);
        let w = rnd(n * k, 9 + n as u64, 0.1);
        // the GPU sees bf16 inputs; compare against the CPU result on the same bf16-rounded inputs
        let a16: Vec<f32> = a.iter().map(|&x| half::bf16::from_f32(x).to_f32()).collect();
        let w16: Vec<f32> = w.iter().map(|&x| half::bf16::from_f32(x).to_f32()).collect();
        let pb = PackedB::from_rows(&w16, n, k);
        let mut c_cpu = vec![0f32; m * n];
        let t = Instant::now();
        matmul(&a16, m, &pb, &mut c_cpu);
        let cpu_s = t.elapsed().as_secs_f64();
        let r = omega::Resident::upload_rows(&w, n, k).unwrap_or_else(|e| die(&e));
        let mut c_gpu = vec![0f32; m * n];
        r.matmul(&a, m, &mut c_gpu).unwrap_or_else(|e| die(&e)); // warm-up (kernel generation)
        let t = Instant::now();
        let reps = 5;
        for _ in 0..reps {
            r.matmul(&a, m, &mut c_gpu).unwrap_or_else(|e| die(&e));
        }
        let gpu_s = t.elapsed().as_secs_f64() / reps as f64;
        let (ea, er) = err(&c_gpu, &c_cpu);
        let gflop = 2.0 * (m * k * n) as f64 / 1e9;
        results.push(json!({"op": "matmul", "m": m, "k": k, "n": n, "max_abs_err": ea, "max_err_over_max_ref": er,
            "cpu_s": cpu_s, "gpu_s": gpu_s, "cpu_gflops": gflop / cpu_s, "gpu_gflops": gflop / gpu_s}));
    }
    for &(t_len, heads, hd) in &[(1500usize, 20usize, 64usize), (1500, 6, 64)] {
        let d = heads * hd;
        let q = rnd(t_len * d, 3, 2.0);
        let k = rnd(t_len * d, 4, 2.0);
        let v = rnd(t_len * d, 5, 2.0);
        let t = Instant::now();
        let o_cpu = model::attention_full(&q, &k, &v, t_len, heads, hd);
        let cpu_s = t.elapsed().as_secs_f64();
        let _ = omega::attention_full(&q, &k, &v, t_len, t_len, heads, hd).unwrap_or_else(|e| die(&e));
        let t = Instant::now();
        let o_gpu = omega::attention_full(&q, &k, &v, t_len, t_len, heads, hd).unwrap_or_else(|e| die(&e));
        let gpu_s = t.elapsed().as_secs_f64();
        let (ea, er) = err(&o_gpu, &o_cpu);
        results.push(json!({"op": "attention_full", "rows": t_len, "kv_len": t_len, "heads": heads, "head_dim": hd,
            "max_abs_err": ea, "max_err_over_max_ref": er, "cpu_s": cpu_s, "gpu_s": gpu_s}));
    }
    println!("{}", serde_json::to_string_pretty(&json!({
        "tool": "rig-listen gpu-selftest",
        "git_sha": option_env!("RIG_GIT_SHA").unwrap_or("unknown"),
        "omega_gpu_lib": option_env!("RIG_OMEGA_GPU_LIB"),
        "threads": rayon::current_num_threads(),
        "load_average_at_end": std::fs::read_to_string("/proc/loadavg").ok().map(|s| s.trim().to_string()),
        "results": results
    })).unwrap());
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|s| s.as_str()) == Some("wer") {
        return wer_cmd(&args[1..]);
    }
    if args.first().map(|s| s.as_str()) == Some("gpu-selftest") {
        if !omega::AVAILABLE {
            die("gpu-selftest needs a build with --features omega");
        }
        return gpu_selftest();
    }
    let mut input = None;
    let mut model_dir = std::env::var("RIG_SPEECH_MODEL").ok().map(PathBuf::from);
    let mut out = None;
    let mut language: Option<String> = None;
    let mut backend = if omega::AVAILABLE { model::Backend::Omega } else { model::Backend::Cpu };
    let mut threads = None;
    let mut receipt = None;
    let mut machine = None;
    let mut quiet = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().unwrap_or_else(|| die(&format!("{a} needs a value")));
        match a.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return;
            }
            "--model" => model_dir = Some(PathBuf::from(val())),
            "--out" => out = Some(PathBuf::from(val())),
            "--language" => {
                let l = val();
                language = if l == "auto" { None } else { Some(l) }
            }
            "--backend" => {
                backend = match val().as_str() {
                    "cpu" => model::Backend::Cpu,
                    "omega" => model::Backend::Omega,
                    b => die(&format!("unknown backend {b}")),
                }
            }
            "--threads" => threads = Some(val().parse::<usize>().unwrap_or_else(|_| die("--threads needs a number"))),
            "--receipt" => receipt = Some(PathBuf::from(val())),
            "--machine" => machine = Some(val()),
            "-q" | "--quiet" => quiet = true,
            s if s.starts_with('-') => die(&format!("unknown option {s}\n\n{USAGE}")),
            s => input = Some(PathBuf::from(s)),
        }
    }
    let input = input.unwrap_or_else(|| die(&format!("no input file\n\n{USAGE}")));
    if backend == model::Backend::Omega && !omega::AVAILABLE {
        die("--backend omega needs a build with --features omega");
    }
    if let Some(n) = threads {
        rayon::ThreadPoolBuilder::new().num_threads(n).build_global().ok();
    }
    let model_dir = model_dir.unwrap_or_else(|| {
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
        PathBuf::from(home).join("models").join("whisper-large-v3-turbo-41f01f3f")
    });
    let out = out.unwrap_or_else(|| PathBuf::from(format!("{}.transcript.json", input.display())));

    let t_all = Instant::now();
    let t = Instant::now();
    let (samples, dec) = audio::load_16k(&input).unwrap_or_else(|e| die(&e));
    let decode_s = t.elapsed().as_secs_f64();
    let audio_s = samples.len() as f64 / audio::SAMPLE_RATE as f64;
    if !quiet {
        eprintln!(
            "audio: {:.1} s, {} Hz, {} ch, codec {} ({decode_s:.2} s to decode and resample)",
            audio_s, dec.source_rate, dec.source_channels, dec.codec
        );
    }
    let t = Instant::now();
    let m = model::Whisper::load(&model_dir, backend).unwrap_or_else(|e| die(&e));
    let tok = weights::Tokenizer::load(&model_dir.join("tokenizer.json")).unwrap_or_else(|e| die(&e));
    let sp = transcribe::Specials::load(&tok, &model_dir).unwrap_or_else(|e| die(&e));
    let load_s = t.elapsed().as_secs_f64();
    if !quiet {
        eprintln!(
            "model: {} ({} encoder + {} decoder layers, d={}), backend {:?}, loaded in {load_s:.1} s",
            model_dir.display(),
            m.cfg.enc_layers,
            m.cfg.dec_layers,
            m.cfg.d,
            backend
        );
    }
    let t = Instant::now();
    let (mel_all, frames) = mel::log_mel(&samples, m.cfg.n_mels);
    let mel_s = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let opts = transcribe::Options { language, no_speech_threshold: 0.6, logprob_threshold: -1.0, verbose: !quiet };
    let tr = transcribe::transcribe(&m, &tok, &sp, &mel_all, frames, &opts);
    let transcribe_s = t.elapsed().as_secs_f64();
    let doc = transcribe::to_json(&tr);
    std::fs::write(&out, serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap_or_else(|e| die(&format!("{}: {e}", out.display())));
    let total_s = t_all.elapsed().as_secs_f64();
    let process_s = decode_s + mel_s + transcribe_s;
    if !quiet {
        eprintln!(
            "wrote {} ({} segments, language {}). {:.1} s of audio in {:.1} s ({:.1}x real time, model load {:.1} s not counted)",
            out.display(),
            tr.segments.len(),
            tr.language,
            audio_s,
            process_s,
            audio_s / process_s,
            load_s
        );
    }
    if let Some(r) = receipt {
        let tm = &tr.timing;
        let doc = json!({
            "tool": "rig-listen",
            "crate_version": env!("CARGO_PKG_VERSION"),
            "git_sha": option_env!("RIG_GIT_SHA").unwrap_or("unknown"),
            "machine": machine.unwrap_or_else(|| "unlabelled".into()),
            "backend": format!("{backend:?}").to_lowercase(),
            "decoder_on_gpu": m.decoder_on_gpu,
            "omega_gpu_lib": option_env!("RIG_OMEGA_GPU_LIB"),
            "threads": rayon::current_num_threads(),
            "load_average_at_end": std::fs::read_to_string("/proc/loadavg").ok().map(|s| s.trim().to_string()),
            "model_dir": model_dir.display().to_string(),
            "model_safetensors_sha256": sha256_of_model(&model_dir),
            "input": input.display().to_string(),
            "audio_seconds": audio_s,
            "source_rate_hz": dec.source_rate,
            "source_channels": dec.source_channels,
            "codec": dec.codec,
            "language": tr.language,
            "segments": tr.segments.len(),
            "seconds": {
                "model_load": load_s,
                "audio_decode_and_resample": decode_s,
                "log_mel": mel_s,
                "transcribe": transcribe_s,
                "encoder": tm.encoder_s,
                "encoder_matmul": tm.encoder_matmul_s,
                "encoder_attention": tm.encoder_attention_s,
                "encoder_other_cpu_ops": tm.encoder_s - tm.encoder_matmul_s - tm.encoder_attention_s,
                "decoder": tm.decoder_s,
                "total_including_load": total_s
            },
            "windows": tm.windows,
            "decoder_steps": tm.decoder_tokens,
            "temperature_fallbacks": tm.fallbacks,
            "realtime_factor_excluding_load": audio_s / process_s,
            "transcript": out.display().to_string()
        });
        std::fs::write(&r, serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap_or_else(|e| die(&format!("{}: {e}", r.display())));
    }
}
