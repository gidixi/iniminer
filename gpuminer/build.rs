use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=cuda/kernel.cu");
    println!("cargo:rerun-if-env-changed=NVCC_CCBIN");
    println!("cargo:rerun-if-env-changed=GPUMINER_NVCC_ALLOW_UNSUPPORTED");
    println!("cargo:rerun-if-env-changed=GPUMINER_NVCC_PATCH_MATH_FUNCTIONS");
    println!("cargo:rerun-if-env-changed=GPUMINER_NVCC_UNDEF_GNU_SOURCE");
    println!("cargo:rerun-if-env-changed=GPUMINER_CUDART_DIR");
    println!("cargo:rerun-if-env-changed=CUDA_HOME");
    println!("cargo:rerun-if-env-changed=CUDA_PATH");

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

    // Evita che glibc esponga le prototype GNU cospi/sinpi che confliggono
    // con quelle CUDA su alcune combinazioni toolkit/glibc.
    let undef_gnu_source = env::var("GPUMINER_NVCC_UNDEF_GNU_SOURCE")
        .map(|v| v != "0")
        .unwrap_or(true);
    if undef_gnu_source {
        nvcc_args.push("-U_GNU_SOURCE".to_string());
        nvcc_args.push("-Xcompiler".to_string());
        nvcc_args.push("-U_GNU_SOURCE".to_string());
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
    let link_mode = add_cuda_link_search_paths();
    match link_mode {
        CudartLinkMode::Dynamic => {
            println!("cargo:rustc-link-lib=dylib=cudart");
        }
        CudartLinkMode::Static => {
            println!("cargo:rustc-link-lib=static=cudart_static");
            println!("cargo:rustc-link-lib=dylib=dl");
            println!("cargo:rustc-link-lib=dylib=pthread");
            println!("cargo:rustc-link-lib=dylib=rt");
        }
        CudartLinkMode::Unknown => {
            // Mantiene comportamento storico, ma con warning esplicito.
            println!("cargo:warning=libcudart non trovata nei path CUDA noti; provo link dinamico standard");
            println!("cargo:rustc-link-lib=dylib=cudart");
        }
    }
    println!("cargo:rustc-link-lib=dylib=stdc++");
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum CudartLinkMode {
    Dynamic,
    Static,
    Unknown,
}

fn add_cuda_link_search_paths() -> CudartLinkMode {
    let mut dirs: Vec<PathBuf> = Vec::new();

    // Override esplicito: directory che contiene libcudart.so / libcudart_static.a
    if let Ok(d) = env::var("GPUMINER_CUDART_DIR") {
        if !d.is_empty() {
            dirs.push(PathBuf::from(d));
        }
    }

    // CUDA roots comuni
    for var in ["CUDA_HOME", "CUDA_PATH"] {
        if let Ok(root) = env::var(var) {
            if !root.is_empty() {
                let r = PathBuf::from(root);
                dirs.push(r.join("lib64"));
                dirs.push(r.join("targets/x86_64-linux/lib"));
                dirs.push(r.join("lib"));
            }
        }
    }

    // Default toolkit path
    let default_root = PathBuf::from("/usr/local/cuda");
    dirs.push(default_root.join("lib64"));
    dirs.push(default_root.join("targets/x86_64-linux/lib"));
    dirs.push(default_root.join("lib"));

    // Versioned toolkits: /usr/local/cuda-*
    if let Ok(entries) = fs::read_dir("/usr/local") {
        for e in entries.flatten() {
            let p = e.path();
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("cuda-") {
                    dirs.push(p.join("lib64"));
                    dirs.push(p.join("targets/x86_64-linux/lib"));
                    dirs.push(p.join("lib"));
                }
            }
        }
    }

    // Dedup + output link-search path esistenti
    let mut uniq: Vec<PathBuf> = Vec::new();
    for d in dirs {
        if d.exists() && !uniq.iter().any(|u| u == &d) {
            println!("cargo:rustc-link-search=native={}", d.display());
            uniq.push(d);
        }
    }

    let mut has_dynamic = false;
    let mut has_static = false;
    for d in &uniq {
        if directory_has_prefix(d, "libcudart.so") {
            has_dynamic = true;
        }
        if d.join("libcudart_static.a").exists() {
            has_static = true;
        }
    }

    if has_dynamic {
        CudartLinkMode::Dynamic
    } else if has_static {
        CudartLinkMode::Static
    } else {
        CudartLinkMode::Unknown
    }
}

fn directory_has_prefix(dir: &Path, prefix: &str) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for e in entries.flatten() {
        if let Some(name) = e.file_name().to_str() {
            if name.starts_with(prefix) {
                return true;
            }
        }
    }
    false
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
