use crate::versahash;

#[cfg(feature = "cuda")]
unsafe extern "C" {
    fn gpuminer_prepare_batch_cuda(
        header_hash: *const u8,
        extra_nonce: *const u8,
        start_nonce: u64,
        count: u64,
        out_key_hashes: *mut u8,
        out_sign_data_hashes: *mut u8,
    ) -> i32;
}

fn scan_nonces_cpu_full(
    header_hash: &[u8; 32],
    extra_nonce: &[u8; 8],
    start_nonce: u64,
    count: u64,
    target: &[u8; 32],
) -> Option<u64> {
    let mut nonce = start_nonce;
    for _ in 0..count {
        let nonce_bytes = nonce.to_be_bytes();
        let result = versahash::versa_hash(header_hash, &nonce_bytes, extra_nonce);
        if &result <= target {
            return Some(nonce);
        }
        nonce = nonce.wrapping_add(1);
    }
    None
}

#[cfg(feature = "cuda")]
fn scan_nonces_cuda_hybrid(
    header_hash: &[u8; 32],
    extra_nonce: &[u8; 8],
    start_nonce: u64,
    count: u64,
    target: &[u8; 32],
) -> Result<Option<u64>, i32> {
    const BATCH: u64 = 65_536;

    let mut scanned = 0_u64;
    while scanned < count {
        let this_count = (count - scanned).min(BATCH);
        let base_nonce = start_nonce.wrapping_add(scanned);
        let mut key_hashes = vec![0_u8; (this_count as usize) * 32];
        let mut sign_data_hashes = vec![0_u8; (this_count as usize) * 32];

        let rc = unsafe {
            gpuminer_prepare_batch_cuda(
                header_hash.as_ptr(),
                extra_nonce.as_ptr(),
                base_nonce,
                this_count,
                key_hashes.as_mut_ptr(),
                sign_data_hashes.as_mut_ptr(),
            )
        };
        if rc != 0 {
            return Err(rc);
        }

        for i in 0..(this_count as usize) {
            let mut key_hash = [0_u8; 32];
            let mut sign_data = [0_u8; 32];
            key_hash.copy_from_slice(&key_hashes[i * 32..(i + 1) * 32]);
            sign_data.copy_from_slice(&sign_data_hashes[i * 32..(i + 1) * 32]);

            let result = versahash::versa_hash_from_key_material(&key_hash, &sign_data);
            if &result <= target {
                return Ok(Some(base_nonce.wrapping_add(i as u64)));
            }
        }

        scanned = scanned.wrapping_add(this_count);
    }

    Ok(None)
}

pub fn scan_nonces(
    header_hash: &[u8; 32],
    extra_nonce: &[u8; 8],
    start_nonce: u64,
    count: u64,
    target: &[u8; 32],
) -> Option<u64> {
    #[cfg(feature = "cuda")]
    {
        match scan_nonces_cuda_hybrid(header_hash, extra_nonce, start_nonce, count, target) {
            Ok(result) => return result,
            Err(rc) => {
                eprintln!(
                    "[gpuminer] backend CUDA non utilizzabile (rc={}), fallback CPU.",
                    rc
                );
            }
        }
    }

    scan_nonces_cpu_full(header_hash, extra_nonce, start_nonce, count, target)
}
