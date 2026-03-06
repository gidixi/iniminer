use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=cuda/kernel.cu");
    println!("cargo:rerun-if-env-changed=NVCC_CCBIN");
    println!("cargo:rerun-if-env-changed=GPUMINER_NVCC_ALLOW_UNSUPPORTED");

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

    let mut nvcc_args = vec![
        "-c".to_string(),
        "cuda/kernel.cu".to_string(),
        "-o".to_string(),
        obj.to_str().expect("path utf8").to_string(),
        "-O3".to_string(),
        "-Xcompiler".to_string(),
        "-fPIC".to_string(),
    ];

    // CUDA toolkit spesso è in ritardo rispetto alle versioni GCC più recenti.
    // Default: abilita override, disattivabile con GPUMINER_NVCC_ALLOW_UNSUPPORTED=0.
    let allow_unsupported = env::var("GPUMINER_NVCC_ALLOW_UNSUPPORTED")
        .map(|v| v != "0")
        .unwrap_or(true);
    if allow_unsupported {
        nvcc_args.push("-allow-unsupported-compiler".to_string());
    }

    // Permette di forzare il compilatore host (es. gcc-13) quando serve.
    if let Ok(ccbin) = env::var("NVCC_CCBIN") {
        if !ccbin.is_empty() {
            nvcc_args.push("-ccbin".to_string());
            nvcc_args.push(ccbin);
        }
    }

    let status = Command::new("nvcc")
        .args(nvcc_args)
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
