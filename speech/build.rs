//! With `--features omega`, link Omega's `libomega_gpu.a` (built by `make libomega_gpu`
//! in an omega checkout; our own Blackwell code generator and command submission,
//! no CUDA, no nvcc). Point `OMEGA_GPU_LIB` at the archive.
fn main() {
    println!("cargo:rerun-if-env-changed=OMEGA_GPU_LIB");
    if let Ok(o) = std::process::Command::new("git").args(["rev-parse", "--short=12", "HEAD"]).output() {
        if o.status.success() {
            println!("cargo:rustc-env=RIG_GIT_SHA={}", String::from_utf8_lossy(&o.stdout).trim());
        }
    }
    if std::env::var("CARGO_FEATURE_OMEGA").is_err() {
        return;
    }
    let lib = std::env::var("OMEGA_GPU_LIB")
        .expect("feature `omega` needs OMEGA_GPU_LIB=/path/to/libomega_gpu.a (make libomega_gpu in an omega checkout)");
    let path = std::path::Path::new(&lib);
    assert!(path.is_file(), "OMEGA_GPU_LIB={lib} is not a file");
    println!("cargo:rerun-if-changed={lib}");
    let dir = path.parent().expect("OMEGA_GPU_LIB has no parent directory");
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!("cargo:rustc-link-lib=static=omega_gpu");
    println!("cargo:rustc-link-lib=dylib=pthread");
    println!("cargo:rustc-link-lib=dylib=m");
    println!("cargo:rustc-env=RIG_OMEGA_GPU_LIB={lib}");
}
