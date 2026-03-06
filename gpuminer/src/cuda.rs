use crate::versahash;

#[cfg(feature = "cuda")]
unsafe extern "C" {
    fn gpuminer_scan_nonces_cuda(
        header_hash: *const u8,
        extra_nonce: *const u8,
        start_nonce: u64,
        count: u64,
        target: *const u8,
        found_nonce: *mut u64,
        found: *mut i32,
    ) -> i32;
}

fn scan_nonces_cpu(
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
fn scan_nonces_cuda(
    header_hash: &[u8; 32],
    extra_nonce: &[u8; 8],
    start_nonce: u64,
    count: u64,
    target: &[u8; 32],
) -> Result<Option<u64>, i32> {
    let mut found_nonce = 0_u64;
    let mut found_flag = 0_i32;

    let rc = unsafe {
        gpuminer_scan_nonces_cuda(
            header_hash.as_ptr(),
            extra_nonce.as_ptr(),
            start_nonce,
            count,
            target.as_ptr(),
            &mut found_nonce as *mut u64,
            &mut found_flag as *mut i32,
        )
    };
    if rc != 0 {
        return Err(rc);
    }
    if found_flag == 1 {
        Ok(Some(found_nonce))
    } else {
        Ok(None)
    }
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
        match scan_nonces_cuda(header_hash, extra_nonce, start_nonce, count, target) {
            Ok(result) => return result,
            Err(rc) => {
                eprintln!(
                    "[gpuminer] backend CUDA non utilizzabile (rc={}), fallback CPU.",
                    rc
                );
            }
        }
    }

    scan_nonces_cpu(header_hash, extra_nonce, start_nonce, count, target)
}
