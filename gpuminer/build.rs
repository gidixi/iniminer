use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=cuda/kernel.cu");

    if env::var_os("CARGO_FEATURE_CUDA").is_none() {
        return;
    }

    let nvcc_ok = Command::new("nvcc").arg("--version").output().is_ok();
    if !nvcc_ok {
        panic!("feature 'cuda' abilitata ma nvcc non trovato nel PATH");
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR mancante"));
    let obj = out_dir.join("kernel.o");
    let lib = out_dir.join("libgpuminer_cuda.a");

    let status = Command::new("nvcc")
        .args([
            "-c",
            "cuda/kernel.cu",
            "-o",
            obj.to_str().expect("path utf8"),
            "-O3",
            "-Xcompiler",
            "-fPIC",
        ])
        .status()
        .expect("impossibile lanciare nvcc");
    if !status.success() {
        panic!("compilazione CUDA fallita (nvcc)");
    }

    let status = Command::new("ar")
        .args([
            "crus",
            lib.to_str().expect("path utf8"),
            obj.to_str().expect("path utf8"),
        ])
        .status()
        .expect("impossibile lanciare ar");
    if !status.success() {
        panic!("archiviazione libgpuminer_cuda.a fallita");
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=gpuminer_cuda");
    println!("cargo:rustc-link-lib=dylib=cudart");
    println!("cargo:rustc-link-lib=dylib=stdc++");
}
