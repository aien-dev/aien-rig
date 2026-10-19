//! With `--features omega`, link Omega's `libomega_gpu.a` (built by `make libomega_gpu`
//! in an omega checkout; our own Blackwell code generator and command submission,
//! no CUDA, no nvcc). Point `OMEGA_GPU_LIB` at the archive.
//!
//! Also records the source commit for speed receipts as RIG_GIT_SHA
//! (`git describe --always --dirty`, so a build from uncommitted files says `-dirty`).
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let o = Command::new("git").args(args).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn main() {
    println!("cargo:rerun-if-env-changed=OMEGA_GPU_LIB");
    // Re-run when the sources or the checked-out commit change, so the recorded commit is current.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    if let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={head}");
    }
    if let Some(index) = git(&["rev-parse", "--git-path", "index"]) {
        println!("cargo:rerun-if-changed={index}");
    }
    if let Some(sha) = git(&["describe", "--always", "--dirty", "--abbrev=12"]) {
        println!("cargo:rustc-env=RIG_GIT_SHA={sha}");
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
    if let Some(osha) = Command::new("git")
        .args(["-C", &dir.join("..").to_string_lossy(), "describe", "--always", "--dirty", "--abbrev=12"])
        .output()
        .ok()
        .filter(|o| o.status.success())
    {
        println!("cargo:rustc-env=RIG_OMEGA_GIT_SHA={}", String::from_utf8_lossy(&osha.stdout).trim());
    }
}
