//! CPU matrix multiply in plain Rust (no BLAS): C[m, n] = A[m, k] * B, where B is
//! packed once into 16-column panels. Register-blocked 6 x 16 micro-kernel, K blocked
//! by 256, threads over (row block, panel group) tiles.
use rayon::prelude::*;

pub const NR: usize = 16;
const MR: usize = 6;
const KC: usize = 256;
const MB: usize = MR * 16;

/// B packed for the kernel: panel p holds columns p*16..p*16+16 for every k,
/// laid out k-major (16 contiguous values per k). Columns past n are zero.
pub struct PackedB {
    pub n: usize,
    pub k: usize,
    panels: usize,
    data: Vec<f32>,
}

impl PackedB {
    /// From a PyTorch Linear weight: `w` is [n, k] row-major (one row per output).
    pub fn from_rows(w: &[f32], n: usize, k: usize) -> Self {
        assert_eq!(w.len(), n * k);
        let panels = n.div_ceil(NR);
        let mut data = vec![0f32; panels * k * NR];
        data.par_chunks_mut(k * NR).enumerate().for_each(|(p, panel)| {
            for j in 0..NR {
                let col = p * NR + j;
                if col >= n {
                    break;
                }
                let row = &w[col * k..(col + 1) * k];
                for (kk, &v) in row.iter().enumerate() {
                    panel[kk * NR + j] = v;
                }
            }
        });
        Self { n, k, panels, data }
    }

    /// From a plain [k, n] row-major matrix.
    pub fn from_kn(b: &[f32], k: usize, n: usize) -> Self {
        assert_eq!(b.len(), n * k);
        let panels = n.div_ceil(NR);
        let mut data = vec![0f32; panels * k * NR];
        for p in 0..panels {
            let c0 = p * NR;
            let w = NR.min(n - c0);
            let panel = &mut data[p * k * NR..(p + 1) * k * NR];
            for kk in 0..k {
                panel[kk * NR..kk * NR + w].copy_from_slice(&b[kk * n + c0..kk * n + c0 + w]);
            }
        }
        Self { n, k, panels, data }
    }
}

#[inline(always)]
fn fmadd(a: f32, b: f32, c: f32) -> f32 {
    #[cfg(any(target_arch = "aarch64", target_feature = "fma"))]
    {
        a.mul_add(b, c)
    }
    #[cfg(not(any(target_arch = "aarch64", target_feature = "fma")))]
    {
        a * b + c
    }
}

/// R rows of A (stride lda) times one packed panel chunk (kc values of k).
#[inline(always)]
unsafe fn kernel<const R: usize>(
    a: *const f32,
    lda: usize,
    b: *const f32,
    kc: usize,
    c: *mut f32,
    ldc: usize,
    ncols: usize,
    accumulate: bool,
) {
    let mut acc = [[0f32; NR]; R];
    if accumulate {
        for i in 0..R {
            for j in 0..ncols {
                acc[i][j] = *c.add(i * ldc + j);
            }
        }
    }
    for kk in 0..kc {
        let bv: [f32; NR] = *(b.add(kk * NR) as *const [f32; NR]);
        for i in 0..R {
            let av = *a.add(i * lda + kk);
            for j in 0..NR {
                acc[i][j] = fmadd(av, bv[j], acc[i][j]);
            }
        }
    }
    for i in 0..R {
        for j in 0..ncols {
            *c.add(i * ldc + j) = acc[i][j];
        }
    }
}

#[derive(Clone, Copy)]
struct SendPtr(*mut f32);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}
impl SendPtr {
    fn get(self) -> *mut f32 {
        self.0
    }
}

/// C[m, n] = A[m, k] * B. `c` is overwritten.
pub fn matmul(a: &[f32], m: usize, b: &PackedB, c: &mut [f32]) {
    let (k, n) = (b.k, b.n);
    assert!(a.len() >= m * k, "A too short");
    assert!(c.len() >= m * n, "C too short");
    if m == 0 || n == 0 {
        return;
    }
    if k == 0 {
        c[..m * n].fill(0.0);
        return;
    }
    let pg = if m <= MR { 8 } else { 4 };
    let mchunks = m.div_ceil(MB);
    let pgroups = b.panels.div_ceil(pg);
    let cptr = SendPtr(c.as_mut_ptr());
    let bdata = &b.data;
    (0..mchunks * pgroups).into_par_iter().for_each(|t| {
        let cbase = cptr.get();
        let (mc, gc) = (t / pgroups, t % pgroups);
        let (m0, m1) = (mc * MB, (mc * MB + MB).min(m));
        let (p0, p1) = (gc * pg, (gc * pg + pg).min(b.panels));
        let mut k0 = 0;
        while k0 < k {
            let kc = KC.min(k - k0);
            for p in p0..p1 {
                let ncols = NR.min(n - p * NR);
                // SAFETY: indices stay inside A (m*k), the panel (k*NR) and C (m*n);
                // tiles of C written by different tasks do not overlap.
                unsafe {
                    let bp = bdata.as_ptr().add(p * k * NR + k0 * NR);
                    let mut i = m0;
                    while i < m1 {
                        let r = MR.min(m1 - i);
                        let ap = a.as_ptr().add(i * k + k0);
                        let cp = cbase.add(i * n + p * NR);
                        let accum = k0 > 0;
                        match r {
                            6 => kernel::<6>(ap, k, bp, kc, cp, n, ncols, accum),
                            5 => kernel::<5>(ap, k, bp, kc, cp, n, ncols, accum),
                            4 => kernel::<4>(ap, k, bp, kc, cp, n, ncols, accum),
                            3 => kernel::<3>(ap, k, bp, kc, cp, n, ncols, accum),
                            2 => kernel::<2>(ap, k, bp, kc, cp, n, ncols, accum),
                            _ => kernel::<1>(ap, k, bp, kc, cp, n, ncols, accum),
                        }
                        i += r;
                    }
                }
            }
            k0 += kc;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive(a: &[f32], m: usize, k: usize, w: &[f32], n: usize) -> Vec<f32> {
        let mut c = vec![0f32; m * n];
        for i in 0..m {
            for j in 0..n {
                let mut s = 0f64;
                for kk in 0..k {
                    s += a[i * k + kk] as f64 * w[j * k + kk] as f64;
                }
                c[i * n + j] = s as f32;
            }
        }
        c
    }

    fn rnd(n: usize, seed: u64) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                ((s >> 33) as f32 / (1u64 << 31) as f32) - 0.5
            })
            .collect()
    }

    #[test]
    fn matches_naive_on_odd_shapes() {
        for &(m, k, n) in &[(1, 1, 1), (1, 300, 33), (7, 17, 5), (13, 513, 47), (100, 64, 1500), (97, 600, 16)] {
            let a = rnd(m * k, 1 + m as u64);
            let w = rnd(n * k, 7 + n as u64);
            let pb = PackedB::from_rows(&w, n, k);
            let mut c = vec![f32::NAN; m * n];
            matmul(&a, m, &pb, &mut c);
            let r = naive(&a, m, k, &w, n);
            for (x, y) in c.iter().zip(&r) {
                assert!((x - y).abs() <= 1e-4 * (1.0 + y.abs()), "{m}x{k}x{n}: {x} vs {y}");
            }
            // from_kn gives the same result
            let mut wt = vec![0f32; k * n];
            for j in 0..n {
                for kk in 0..k {
                    wt[kk * n + j] = w[j * k + kk];
                }
            }
            let pb2 = PackedB::from_kn(&wt, k, n);
            let mut c2 = vec![0f32; m * n];
            matmul(&a, m, &pb2, &mut c2);
            assert_eq!(c, c2);
        }
    }
}
