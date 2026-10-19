//! Whisper encoder-decoder (the published architecture, Radford et al. 2022, as laid
//! out in the Hugging Face `WhisperForConditionalGeneration` safetensors):
//! encoder = conv1d(k3) + GELU, conv1d(k3, stride 2) + GELU, + positional table,
//! pre-LayerNorm transformer blocks, final LayerNorm; decoder = token + learned position
//! embeddings, blocks of causal self-attention, cross-attention and MLP, final
//! LayerNorm, logits against the token embedding (tied).
use std::path::Path;
use std::time::Instant;

use rayon::prelude::*;
use serde_json::Value;

use crate::gemm::{matmul, PackedB};
use crate::omega::Resident;
use crate::weights::SafeTensors;

#[derive(Clone, Debug)]
pub struct Config {
    pub d: usize,
    pub n_mels: usize,
    pub enc_layers: usize,
    pub enc_heads: usize,
    pub dec_layers: usize,
    pub dec_heads: usize,
    pub vocab: usize,
    pub n_audio_ctx: usize,
    pub n_text_ctx: usize,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?)
            .map_err(|e| format!("config.json: {e}"))?;
        let g = |k: &str| v[k].as_u64().map(|x| x as usize).ok_or(format!("config.json: {k} missing"));
        Ok(Self {
            d: g("d_model")?,
            n_mels: g("num_mel_bins")?,
            enc_layers: g("encoder_layers")?,
            enc_heads: g("encoder_attention_heads")?,
            dec_layers: g("decoder_layers")?,
            dec_heads: g("decoder_attention_heads")?,
            vocab: g("vocab_size")?,
            n_audio_ctx: g("max_source_positions")?,
            n_text_ctx: g("max_target_positions")?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Cpu,
    Omega,
}

/// Where the time went (seconds), for receipts.
#[derive(Default, Clone, Debug)]
pub struct Timing {
    pub encoder_s: f64,
    pub encoder_matmul_s: f64,
    pub encoder_attention_s: f64,
    pub decoder_s: f64,
    pub decoder_tokens: usize,
    pub fallbacks: usize,
    pub windows: usize,
}

pub struct Linear {
    pub n: usize,
    pub k: usize,
    cpu: Option<PackedB>,
    gpu: Option<Resident>,
    bias: Option<Vec<f32>>,
}

impl Linear {
    fn load(st: &SafeTensors, name: &str, bias: bool, gpu: bool) -> Result<Self, String> {
        let (w, shape) = st.f32(&format!("{name}.weight"))?;
        let (n, k) = match shape.len() {
            2 => (shape[0], shape[1]),
            3 => (shape[0], shape[1] * shape[2]), // conv1d [out, in, kernel]
            _ => return Err(format!("{name}: unexpected shape {shape:?}")),
        };
        let bias = if bias { Some(st.f32(&format!("{name}.bias"))?.0) } else { None };
        let (cpu, gpu) = if gpu {
            (None, Some(Resident::upload_rows(&w, n, k)?))
        } else {
            (Some(PackedB::from_rows(&w, n, k)), None)
        };
        Ok(Self { n, k, cpu, gpu, bias })
    }

    /// q_proj, k_proj (no bias) and v_proj stacked: one matrix multiply instead of three.
    fn load_qkv(st: &SafeTensors, prefix: &str, gpu: bool) -> Result<Self, String> {
        let (q, qs) = st.f32(&format!("{prefix}.q_proj.weight"))?;
        let (k, _) = st.f32(&format!("{prefix}.k_proj.weight"))?;
        let (v, _) = st.f32(&format!("{prefix}.v_proj.weight"))?;
        let (d, kk) = (qs[0], qs[1]);
        let mut w = q;
        w.extend_from_slice(&k);
        w.extend_from_slice(&v);
        let mut b = st.f32(&format!("{prefix}.q_proj.bias"))?.0;
        b.extend(std::iter::repeat_n(0.0, d));
        b.extend_from_slice(&st.f32(&format!("{prefix}.v_proj.bias"))?.0);
        let n = 3 * d;
        let (cpu, gpu) = if gpu {
            (None, Some(Resident::upload_rows(&w, n, kk)?))
        } else {
            (Some(PackedB::from_rows(&w, n, kk)), None)
        };
        Ok(Self { n, k: kk, cpu, gpu, bias: Some(b) })
    }

    pub fn forward(&self, x: &[f32], m: usize) -> Vec<f32> {
        let mut y = vec![0f32; m * self.n];
        if let Some(g) = &self.gpu {
            g.matmul(x, m, &mut y).unwrap_or_else(|e| panic!("{e}"));
        } else {
            matmul(x, m, self.cpu.as_ref().unwrap(), &mut y);
        }
        if let Some(b) = &self.bias {
            y.par_chunks_mut(self.n).for_each(|row| {
                for (v, bb) in row.iter_mut().zip(b) {
                    *v += bb;
                }
            });
        }
        y
    }
}

struct LayerNorm {
    w: Vec<f32>,
    b: Vec<f32>,
}

impl LayerNorm {
    fn load(st: &SafeTensors, name: &str) -> Result<Self, String> {
        Ok(Self { w: st.f32(&format!("{name}.weight"))?.0, b: st.f32(&format!("{name}.bias"))?.0 })
    }
    fn apply(&self, x: &[f32]) -> Vec<f32> {
        let d = self.w.len();
        let mut y = vec![0f32; x.len()];
        y.par_chunks_mut(d).zip(x.par_chunks(d)).for_each(|(o, r)| {
            // two passes in f32 with 8 partial sums (vectorizes; ample precision for d <= 1280)
            let mut s = [0f32; 8];
            for c in r.chunks(8) {
                for (i, v) in c.iter().enumerate() {
                    s[i] += v;
                }
            }
            let mean = s.iter().sum::<f32>() / d as f32;
            let mut s2 = [0f32; 8];
            for c in r.chunks(8) {
                for (i, v) in c.iter().enumerate() {
                    s2[i] += (v - mean) * (v - mean);
                }
            }
            let inv = 1.0 / (s2.iter().sum::<f32>() / d as f32 + 1e-5).sqrt();
            for i in 0..d {
                o[i] = (r[i] - mean) * inv * self.w[i] + self.b[i];
            }
        });
        y
    }
}

/// e^x for the GELU, branch-free so it vectorizes: 2^(x log2 e) split into an integer
/// power (exponent bits) and 2^f on [0, 1) from its Taylor series to degree 7
/// (relative error below 2e-6).
#[inline(always)]
fn exp_fast(x: f32) -> f32 {
    let x = x.clamp(-87.0, 88.0);
    let t = x * std::f32::consts::LOG2_E;
    let fi = t.floor();
    let f = t - fi;
    let p = 1.0
        + f * (0.693_147_2
            + f * (0.240_226_5
                + f * (0.055_504_11 + f * (0.009_618_129 + f * (0.001_333_355 + f * (0.000_154_035_3 + f * 0.000_015_252_73))))));
    p * f32::from_bits(((fi as i32 + 127) as u32) << 23)
}

/// erf by Abramowitz and Stegun 7.1.26 with exp_fast (absolute error below 2e-6 in f32;
/// checked against libm erff in the tests).
#[inline(always)]
pub fn erf_fast(x: f32) -> f32 {
    let a = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * a);
    let y = 1.0
        - t * (0.254_829_6 + t * (-0.284_496_74 + t * (1.421_413_7 + t * (-1.453_152_1 + t * 1.061_405_4))))
            * exp_fast(-a * a);
    y.copysign(x)
}

/// Exact (erf) GELU, as Whisper uses (torch.nn.GELU default).
fn gelu(x: &mut [f32]) {
    x.par_chunks_mut(4096).for_each(|c| {
        for v in c {
            *v = 0.5 * *v * (1.0 + erf_fast(*v * std::f32::consts::FRAC_1_SQRT_2));
        }
    });
}

fn add_into(x: &mut [f32], y: &[f32]) {
    x.par_chunks_mut(4096).zip(y.par_chunks(4096)).for_each(|(a, b)| {
        for (u, v) in a.iter_mut().zip(b) {
            *u += v;
        }
    });
}

fn softmax_row(r: &mut [f32]) {
    let m = r.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let mut s = 0f32;
    for v in r.iter_mut() {
        *v = (*v - m).exp();
        s += *v;
    }
    let inv = 1.0 / s;
    for v in r.iter_mut() {
        *v *= inv;
    }
}

struct EncLayer {
    ln1: LayerNorm,
    qkv: Linear, // q, k, v projections stacked into one [3d, d] matrix
    o: Linear,
    ln2: LayerNorm,
    fc1: Linear,
    fc2: Linear,
}

struct DecLayer {
    ln1: LayerNorm,
    q: Linear,
    k: Linear,
    v: Linear,
    o: Linear,
    ln2: LayerNorm,
    cq: Linear,
    ck: Linear,
    cv: Linear,
    co: Linear,
    ln3: LayerNorm,
    fc1: Linear,
    fc2: Linear,
}

pub struct Whisper {
    pub cfg: Config,
    pub backend: Backend,
    conv1: Linear,
    conv2: Linear,
    enc_pos: Vec<f32>,
    enc: Vec<EncLayer>,
    enc_ln: LayerNorm,
    embed: Vec<f32>,
    dec_pos: Vec<f32>,
    dec: Vec<DecLayer>,
    dec_ln: LayerNorm,
    logits_w: Linear,
    pub decoder_on_gpu: bool,
}

pub struct DecState {
    k: Vec<Vec<f32>>,
    v: Vec<Vec<f32>>,
    ck: Vec<Vec<f32>>,
    cv: Vec<Vec<f32>>,
    pub len: usize,
    t_audio: usize,
}

impl Whisper {
    pub fn load(dir: &Path, backend: Backend) -> Result<Self, String> {
        let cfg = Config::load(&dir.join("config.json"))?;
        let st = SafeTensors::open(&dir.join("model.safetensors"))?;
        let gpu = backend == Backend::Omega;
        let e = "model.encoder";
        let mut enc = Vec::new();
        for i in 0..cfg.enc_layers {
            let p = format!("{e}.layers.{i}");
            enc.push(EncLayer {
                ln1: LayerNorm::load(&st, &format!("{p}.self_attn_layer_norm"))?,
                qkv: Linear::load_qkv(&st, &format!("{p}.self_attn"), gpu)?,
                o: Linear::load(&st, &format!("{p}.self_attn.out_proj"), true, gpu)?,
                ln2: LayerNorm::load(&st, &format!("{p}.final_layer_norm"))?,
                fc1: Linear::load(&st, &format!("{p}.fc1"), true, gpu)?,
                fc2: Linear::load(&st, &format!("{p}.fc2"), true, gpu)?,
            });
        }
        let dname = "model.decoder";
        // The decoder runs one token at a time (matrix-vector work). By default it stays on
        // the CPU path; RIG_DECODER_ON_GPU=1 puts its matrices on the GPU too (Omega backend).
        let dgpu = gpu && std::env::var("RIG_DECODER_ON_GPU").as_deref() == Ok("1");
        let mut dec = Vec::new();
        for i in 0..cfg.dec_layers {
            let p = format!("{dname}.layers.{i}");
            dec.push(DecLayer {
                ln1: LayerNorm::load(&st, &format!("{p}.self_attn_layer_norm"))?,
                q: Linear::load(&st, &format!("{p}.self_attn.q_proj"), true, dgpu)?,
                k: Linear::load(&st, &format!("{p}.self_attn.k_proj"), false, dgpu)?,
                v: Linear::load(&st, &format!("{p}.self_attn.v_proj"), true, dgpu)?,
                o: Linear::load(&st, &format!("{p}.self_attn.out_proj"), true, dgpu)?,
                ln2: LayerNorm::load(&st, &format!("{p}.encoder_attn_layer_norm"))?,
                cq: Linear::load(&st, &format!("{p}.encoder_attn.q_proj"), true, dgpu)?,
                // cross-attention keys/values: 1500 rows once per window, worth the GPU
                ck: Linear::load(&st, &format!("{p}.encoder_attn.k_proj"), false, gpu)?,
                cv: Linear::load(&st, &format!("{p}.encoder_attn.v_proj"), true, gpu)?,
                co: Linear::load(&st, &format!("{p}.encoder_attn.out_proj"), true, dgpu)?,
                ln3: LayerNorm::load(&st, &format!("{p}.final_layer_norm"))?,
                fc1: Linear::load(&st, &format!("{p}.fc1"), true, dgpu)?,
                fc2: Linear::load(&st, &format!("{p}.fc2"), true, dgpu)?,
            });
        }
        let (embed, es) = st.f32(&format!("{dname}.embed_tokens.weight"))?;
        if es != [cfg.vocab, cfg.d] {
            return Err(format!("embed_tokens shape {es:?}"));
        }
        let logits_w = if dgpu {
            Linear { n: cfg.vocab, k: cfg.d, cpu: None, gpu: Some(Resident::upload_rows(&embed, cfg.vocab, cfg.d)?), bias: None }
        } else {
            Linear { n: cfg.vocab, k: cfg.d, cpu: Some(PackedB::from_rows(&embed, cfg.vocab, cfg.d)), gpu: None, bias: None }
        };
        Ok(Self {
            conv1: Linear::load(&st, &format!("{e}.conv1"), true, gpu)?,
            conv2: Linear::load(&st, &format!("{e}.conv2"), true, gpu)?,
            enc_pos: st.f32(&format!("{e}.embed_positions.weight"))?.0,
            enc,
            enc_ln: LayerNorm::load(&st, &format!("{e}.layer_norm"))?,
            embed,
            dec_pos: st.f32(&format!("{dname}.embed_positions.weight"))?.0,
            dec,
            dec_ln: LayerNorm::load(&st, &format!("{dname}.layer_norm"))?,
            logits_w,
            decoder_on_gpu: dgpu,
            cfg,
            backend,
        })
    }

    /// mel: [n_mels, 3000] -> encoder output [1500, d]
    pub fn encode(&self, mel: &[f32], timing: &mut Timing) -> Vec<f32> {
        let t0 = Instant::now();
        let c = &self.cfg;
        let (d, nm) = (c.d, c.n_mels);
        let t_in = mel.len() / nm;
        let mut mm = 0f64;
        // conv1 (k3, pad 1) as im2col + matmul: row t, column ch*3+j = mel[ch, t+j-1]
        let mut col = vec![0f32; t_in * nm * 3];
        col.par_chunks_mut(nm * 3).enumerate().for_each(|(t, row)| {
            for ch in 0..nm {
                for j in 0..3 {
                    let s = t as isize + j as isize - 1;
                    row[ch * 3 + j] = if s >= 0 && (s as usize) < t_in { mel[ch * t_in + s as usize] } else { 0.0 };
                }
            }
        });
        let tm = Instant::now();
        let mut h1 = self.conv1.forward(&col, t_in);
        mm += tm.elapsed().as_secs_f64();
        gelu(&mut h1);
        // conv2 (k3, stride 2, pad 1): row t, column ch*3+j = h1[2t+j-1, ch]
        let t_out = t_in / 2;
        let mut col2 = vec![0f32; t_out * d * 3];
        col2.par_chunks_mut(d * 3).enumerate().for_each(|(t, row)| {
            for j in 0..3 {
                let s = 2 * t as isize + j as isize - 1;
                if s >= 0 && (s as usize) < t_in {
                    let src = &h1[s as usize * d..(s as usize + 1) * d];
                    for ch in 0..d {
                        row[ch * 3 + j] = src[ch];
                    }
                }
            }
        });
        let tm = Instant::now();
        let mut x = self.conv2.forward(&col2, t_out);
        mm += tm.elapsed().as_secs_f64();
        gelu(&mut x);
        add_into(&mut x, &self.enc_pos[..t_out * d]);
        let mut attn_s = 0f64;
        for l in &self.enc {
            let h = l.ln1.apply(&x);
            let tm = Instant::now();
            let qkv = l.qkv.forward(&h, t_out);
            mm += tm.elapsed().as_secs_f64();
            let (mut q, mut k, mut v) = (vec![0f32; t_out * d], vec![0f32; t_out * d], vec![0f32; t_out * d]);
            q.par_chunks_mut(d)
                .zip(k.par_chunks_mut(d))
                .zip(v.par_chunks_mut(d))
                .zip(qkv.par_chunks(3 * d))
                .for_each(|(((qr, kr), vr), src)| {
                    qr.copy_from_slice(&src[..d]);
                    kr.copy_from_slice(&src[d..2 * d]);
                    vr.copy_from_slice(&src[2 * d..]);
                });
            let ta = Instant::now();
            let a = if self.backend == Backend::Omega {
                crate::omega::attention_full(&q, &k, &v, t_out, t_out, c.enc_heads, d / c.enc_heads).unwrap_or_else(|e| panic!("{e}"))
            } else {
                attention_full(&q, &k, &v, t_out, c.enc_heads, d / c.enc_heads)
            };
            attn_s += ta.elapsed().as_secs_f64();
            let tm = Instant::now();
            let o = l.o.forward(&a, t_out);
            mm += tm.elapsed().as_secs_f64();
            add_into(&mut x, &o);
            let h = l.ln2.apply(&x);
            let tm = Instant::now();
            let mut f = l.fc1.forward(&h, t_out);
            mm += tm.elapsed().as_secs_f64();
            gelu(&mut f);
            let tm = Instant::now();
            let f2 = l.fc2.forward(&f, t_out);
            mm += tm.elapsed().as_secs_f64();
            add_into(&mut x, &f2);
        }
        let out = self.enc_ln.apply(&x);
        timing.encoder_s += t0.elapsed().as_secs_f64();
        timing.encoder_matmul_s += mm;
        timing.encoder_attention_s += attn_s;
        out
    }

    /// Fresh decoder state with the cross-attention keys/values for this window.
    pub fn new_state(&self, enc: &[f32]) -> DecState {
        let t = enc.len() / self.cfg.d;
        let (mut ck, mut cv) = (Vec::new(), Vec::new());
        for l in &self.dec {
            ck.push(l.ck.forward(enc, t));
            cv.push(l.cv.forward(enc, t));
        }
        DecState { k: vec![Vec::new(); self.dec.len()], v: vec![Vec::new(); self.dec.len()], ck, cv, len: 0, t_audio: t }
    }

    /// Run new tokens through the decoder; returns hidden states [tokens, d] after the
    /// final LayerNorm.
    pub fn decode(&self, st: &mut DecState, tokens: &[u32]) -> Vec<f32> {
        let d = self.cfg.d;
        let heads = self.cfg.dec_heads;
        let hd = d / heads;
        let n = tokens.len();
        let mut x = vec![0f32; n * d];
        for (i, &tok) in tokens.iter().enumerate() {
            let p = st.len + i;
            for j in 0..d {
                x[i * d + j] = self.embed[tok as usize * d + j] + self.dec_pos[p * d + j];
            }
        }
        for (li, l) in self.dec.iter().enumerate() {
            let h = l.ln1.apply(&x);
            let q = l.q.forward(&h, n);
            st.k[li].extend_from_slice(&l.k.forward(&h, n));
            st.v[li].extend_from_slice(&l.v.forward(&h, n));
            let a = attention_small(&q, n, &st.k[li], &st.v[li], st.len + n, heads, hd, Some(st.len));
            add_into(&mut x, &l.o.forward(&a, n));
            let h = l.ln2.apply(&x);
            let q = l.cq.forward(&h, n);
            let a = attention_small(&q, n, &st.ck[li], &st.cv[li], st.t_audio, heads, hd, None);
            add_into(&mut x, &l.co.forward(&a, n));
            let h = l.ln3.apply(&x);
            let mut f = l.fc1.forward(&h, n);
            gelu(&mut f);
            add_into(&mut x, &l.fc2.forward(&f, n));
        }
        st.len += n;
        self.dec_ln.apply(&x)
    }

    /// Logits [vocab] for one hidden row.
    pub fn logits(&self, h: &[f32]) -> Vec<f32> {
        self.logits_w.forward(h, 1)
    }
}

/// Full (non-causal) multi-head attention for the encoder, with the CPU matmul.
pub fn attention_full(q: &[f32], k: &[f32], v: &[f32], t: usize, heads: usize, hd: usize) -> Vec<f32> {
    let d = heads * hd;
    let scale = 1.0 / (hd as f32).sqrt();
    let mut out = vec![0f32; t * d];
    let mut qh = vec![0f32; t * hd];
    let mut kh = vec![0f32; t * hd];
    let mut vh = vec![0f32; t * hd];
    let mut scores = vec![0f32; t * t];
    let mut oh = vec![0f32; t * hd];
    for h in 0..heads {
        for i in 0..t {
            for j in 0..hd {
                qh[i * hd + j] = q[i * d + h * hd + j] * scale;
                kh[i * hd + j] = k[i * d + h * hd + j];
                vh[i * hd + j] = v[i * d + h * hd + j];
            }
        }
        let kp = PackedB::from_rows(&kh, t, hd);
        matmul(&qh, t, &kp, &mut scores);
        scores.par_chunks_mut(t).for_each(softmax_row);
        let vp = PackedB::from_kn(&vh, t, hd);
        matmul(&scores, t, &vp, &mut oh);
        for i in 0..t {
            out[i * d + h * hd..i * d + h * hd + hd].copy_from_slice(&oh[i * hd..(i + 1) * hd]);
        }
    }
    out
}

/// Attention for a few query rows against a key/value cache ([tk, d] row-major).
/// With `causal_base = Some(b)`, query i sees keys 0..=b+i.
#[allow(clippy::too_many_arguments)]
fn attention_small(q: &[f32], tq: usize, kc: &[f32], vc: &[f32], tk: usize, heads: usize, hd: usize, causal_base: Option<usize>) -> Vec<f32> {
    let d = heads * hd;
    let scale = 1.0 / (hd as f32).sqrt();
    let mut out = vec![0f32; tq * d];
    out.par_chunks_mut(hd).enumerate().for_each(|(idx, o)| {
        let (qi, h) = (idx / heads, idx % heads);
        let limit = match causal_base {
            Some(b) => (b + qi + 1).min(tk),
            None => tk,
        };
        let qv = &q[qi * d + h * hd..qi * d + h * hd + hd];
        let mut s = vec![0f32; limit];
        for (j, sj) in s.iter_mut().enumerate() {
            let kv = &kc[j * d + h * hd..j * d + h * hd + hd];
            let mut acc = 0f32;
            for x in 0..hd {
                acc += qv[x] * kv[x];
            }
            *sj = acc * scale;
        }
        softmax_row(&mut s);
        for (j, &p) in s.iter().enumerate() {
            let vv = &vc[j * d + h * hd..j * d + h * hd + hd];
            for x in 0..hd {
                o[x] += p * vv[x];
            }
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fast_erf_matches_libm() {
        let mut worst = 0f32;
        let mut x = -8.0f32;
        while x <= 8.0 {
            worst = worst.max((erf_fast(x) - libm::erff(x)).abs());
            x += 0.001;
        }
        // A&S 7.1.26 is 1.5e-7 in exact arithmetic; f32 rounding brings it to about 1.3e-6.
        assert!(worst < 2e-6, "worst abs error {worst}");
    }
}
