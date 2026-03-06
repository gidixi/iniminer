use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=cuda/kernel.cu");
    println!("cargo:rerun-if-env-changed=NVCC_CCBIN");
    println!("cargo:rerun-if-env-changed=GPUMINER_NVCC_ALLOW_UNSUPPORTED");
    println!("cargo:rerun-if-env-changed=GPUMINER_NVCC_PATCH_MATH_FUNCTIONS");

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

    // Workaround glibc/cuda header incompatibility (sinpi/cospi noexcept mismatch).
    // Se abilitato, crea un override locale di crt/math_functions.h con noexcept allineato.
    let patch_math = env::var("GPUMINER_NVCC_PATCH_MATH_FUNCTIONS")
        .map(|v| v != "0")
        .unwrap_or(true);
    if patch_math {
        if let Some(patched_header) = create_cuda_math_compat_header(&out_dir) {
            // Forza l'inclusione del file patchato PRIMA delle include CUDA:
            // così l'include guard evita il parsing della versione originale incompatibile.
            nvcc_args.push("--pre-include".to_string());
            nvcc_args.push(patched_header.to_string_lossy().to_string());
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

fn create_cuda_math_compat_header(out_dir: &PathBuf) -> Option<PathBuf> {
    let candidates = [
        "/usr/local/cuda/targets/x86_64-linux/include/crt/math_functions.h",
        "/usr/local/cuda/include/crt/math_functions.h",
    ];
    let src = candidates
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())?;

    let text = fs::read_to_string(&src).ok()?;
    let mut changed = false;
    let mut patched = String::with_capacity(text.len() + 256);

    for line in text.lines() {
        let needs_patch = line.contains("__device_builtin__")
            && (line.contains(" sinpi(")
                || line.contains(" cospi(")
                || line.contains(" sinpif(")
                || line.contains(" cospif("))
            && !line.contains("noexcept");
        if needs_patch {
            if let Some(idx) = line.rfind(';') {
                let mut s = String::with_capacity(line.len() + 18);
                s.push_str(&line[..idx]);
                s.push_str(" noexcept (true)");
                s.push(';');
                patched.push_str(&s);
                patched.push('\n');
                changed = true;
                continue;
            }
        }
        patched.push_str(line);
        patched.push('\n');
    }

    if !changed {
        return None;
    }

    let compat_dir = out_dir.join("cuda_compat");
    if fs::create_dir_all(&compat_dir).is_err() {
        return None;
    }
    let dst = compat_dir.join("math_functions_patched.h");
    if fs::write(&dst, patched).is_err() {
        return None;
    }
    Some(dst)
}
