//! Omega GPU matmul (our own Blackwell code generator and command submission; no CUDA).
//! Weights are uploaded once as bf16 and stay resident on the GPU; each call moves the
//! activation rows (rounded to bf16 by Omega) and returns f32. API: omega
//! `src/omega_gpu_matmul_api.h` (libomega_gpu.a).
#[cfg(feature = "omega")]
mod imp {
    use std::ffi::CStr;
    use std::os::raw::{c_char, c_int, c_void};

    extern "C" {
        fn omega_gpu_tensor_upload_bf16(k: u32, n: u32, b: *const u16, out: *mut *mut c_void) -> c_int;
        fn omega_gpu_tensor_free(t: *mut c_void);
        fn omega_gpu_matmul_resident_f32(m: u32, a: *const f32, b: *const c_void, c: *mut f32, info: *mut c_void) -> c_int;
        fn omega_gpu_matmul_last_error() -> *const c_char;
        fn omega_gpu_matmul_rc_name(rc: c_int) -> *const c_char;
    }

    /// Omega's OmegaGpuMatmulInfo is about 200 bytes; we hand it a larger zeroed,
    /// aligned buffer so a future field cannot write past our memory.
    #[repr(C, align(16))]
    struct InfoBuf([u8; 1024]);

    fn err(rc: c_int) -> String {
        // SAFETY: both return static NUL-terminated strings (or NULL).
        unsafe {
            let name = omega_gpu_matmul_rc_name(rc);
            let last = omega_gpu_matmul_last_error();
            let n = if name.is_null() { "?".into() } else { CStr::from_ptr(name).to_string_lossy().into_owned() };
            let l = if last.is_null() { String::new() } else { CStr::from_ptr(last).to_string_lossy().into_owned() };
            format!("omega gpu matmul rc {rc} ({n}): {l}")
        }
    }

    pub struct Resident {
        ptr: *mut c_void,
        pub k: usize,
        pub n: usize,
    }
    unsafe impl Send for Resident {}
    unsafe impl Sync for Resident {}

    impl Resident {
        /// `w` is a PyTorch Linear weight [n, k]; the GPU wants B = w^T as [k, n] bf16.
        pub fn upload_rows(w: &[f32], n: usize, k: usize) -> Result<Self, String> {
            let mut bt = vec![0u16; k * n];
            for j in 0..n {
                for kk in 0..k {
                    bt[kk * n + j] = half::bf16::from_f32(w[j * k + kk]).to_bits();
                }
            }
            let mut ptr = std::ptr::null_mut();
            // SAFETY: bt holds k*n values; ptr is a valid out-pointer.
            let rc = unsafe { omega_gpu_tensor_upload_bf16(k as u32, n as u32, bt.as_ptr(), &mut ptr) };
            if rc != 0 || ptr.is_null() {
                return Err(err(rc));
            }
            Ok(Self { ptr, k, n })
        }

        /// c[m, n] = a[m, k] * B
        pub fn matmul(&self, a: &[f32], m: usize, c: &mut [f32]) -> Result<(), String> {
            assert!(a.len() >= m * self.k && c.len() >= m * self.n);
            let mut info = InfoBuf([0; 1024]);
            // SAFETY: lengths checked; ptr is live; info is a large writable buffer.
            let rc = unsafe {
                omega_gpu_matmul_resident_f32(m as u32, a.as_ptr(), self.ptr, c.as_mut_ptr(), info.0.as_mut_ptr() as *mut c_void)
            };
            if rc != 0 {
                return Err(err(rc));
            }
            Ok(())
        }
    }

    impl Drop for Resident {
        fn drop(&mut self) {
            // SAFETY: freed once; came from a successful upload.
            unsafe { omega_gpu_tensor_free(self.ptr) }
        }
    }

    pub const AVAILABLE: bool = true;
}

#[cfg(not(feature = "omega"))]
mod imp {
    pub struct Resident {
        pub k: usize,
        pub n: usize,
    }
    impl Resident {
        pub fn upload_rows(_w: &[f32], _n: usize, _k: usize) -> Result<Self, String> {
            Err("built without the `omega` feature".into())
        }
        pub fn matmul(&self, _a: &[f32], _m: usize, _c: &mut [f32]) -> Result<(), String> {
            Err("built without the `omega` feature".into())
        }
    }
    pub const AVAILABLE: bool = false;
}

pub use imp::*;
