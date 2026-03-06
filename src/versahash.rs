/// VersaHash — porta Rust fedele di crypto/versaHash del nodo InitVerse.
///
/// Riferimento: repo `chain` (consensus/inihash, crypto/versaHash).
///
/// ## Flusso mining (da chain)
/// - **SealHash** (consensus/inihash/consensus.go): `Keccak256(RLP(header senza Nonce, ExtraNonce, MixDigest))`.
///   Campi RLP: ParentHash, UncleHash, Coinbase, Root, TxHash, ReceiptHash, Bloom,
///   Difficulty, Number, GasLimit, GasUsed, Time, Extra, Provider, TeamAddress, ValidatorRate, TeamRate.
/// - **Mining** (consensus/inihash/sealer.go): `hash = SealHash(header).Bytes()`; nonce big-endian
///   (`binary.BigEndian.PutUint64(n[:], nonce)`); `result = versaHash.VersaHash(hash, n[:], extraNonce[:])`;
///   `result <= target` (target = 2^256 / difficulty).
/// - **VersaHash** (crypto/versaHash/versaHash.go): newData = data || byte(len(nonce)+len(extraNonce)) || nonce || extraNonce;
///   firstHash=SHA256(newData); keyHash=SHA256(firstHash); signData=SHA256(keyHash); sig=Schnorr(keyHash, signData);
///   endHash=SHA256(sig); return reverse(endHash).
/// - **Schnorr** (crypto/versaHash/schnorr.go): e = SHA256(rX || Marshal_compressed(P) || m) mod N,
///   con nonce RFC6979 deterministico basato su (private_key, message).
///
/// Nessuna dipendenza da num-bigint: tutto il lavoro aritmetico usa
/// k256::Scalar (mod N) e k256::FieldElement (mod P) direttamente.

use hmac::{Hmac, Mac};
use k256::{
    elliptic_curve::{sec1::ToEncodedPoint, PrimeField},
    AffinePoint, FieldBytes, ProjectivePoint, Scalar,
};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

// ─── costanti secp256k1 (big-endian) ─────────────────────────────────────────

/// Ordine N della curva: FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
const CURVE_N: [u8; 32] = [
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
    0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B,
    0xBF, 0xD2, 0x5E, 0x8C, 0xD0, 0x36, 0x41, 0x41,
];

// ─── aritmetica su byte (big-endian, 32 byte) ─────────────────────────────────

/// a - b con prestito, entrambi big-endian 32 byte. Richiede a >= b.
fn sub32(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut borrow: u8 = 0;
    for i in (0..32).rev() {
        let (d1, o1) = a[i].overflowing_sub(b[i]);
        let (d2, o2) = d1.overflowing_sub(borrow);
        out[i] = d2;
        borrow = if o1 || o2 { 1 } else { 0 };
    }
    out
}

/// bits2octets(b, N): se b >= N restituisce b - N, altrimenti b.
/// Equivalente a b mod N per input a 256 bit (una sola sottrazione basta
/// perché b < 2^256 < 2N per secp256k1).
fn bits2octets(b: &[u8; 32]) -> [u8; 32] {
    if b >= &CURVE_N { sub32(b, &CURVE_N) } else { *b }
}

/// Interpreta bytes come Scalar mod N via Scalar::from_repr.
/// Restituisce None se bytes >= N (da cui from_repr ritorna None).
#[inline]
fn bytes_to_scalar(bytes: [u8; 32]) -> Option<Scalar> {
    Option::from(Scalar::from_repr(FieldBytes::from(bytes)))
}

// ─── HMAC-SHA256 helper ───────────────────────────────────────────────────────

// ─── RFC 6979 — nonce deterministico ─────────────────────────────────────────
//
// Replica del flusso richiesto: seed = private_key || message.
fn rfc6979_nonce(private_key: &[u8; 32], message: &[u8; 32]) -> Scalar {
    // RFC6979 sezione 3.2
    let mut v = [0x01u8; 32];
    let mut k = [0x00u8; 32];

    // K = HMAC_K(V || 0x00 || privkey || message)
    let mut mac = HmacSha256::new_from_slice(&k).unwrap();
    mac.update(&v);
    mac.update(&[0x00]);
    mac.update(private_key);
    mac.update(message);
    k = mac.finalize().into_bytes().into();

    // V = HMAC_K(V)
    let mut mac = HmacSha256::new_from_slice(&k).unwrap();
    mac.update(&v);
    v = mac.finalize().into_bytes().into();

    // K = HMAC_K(V || 0x01 || privkey || message)
    let mut mac = HmacSha256::new_from_slice(&k).unwrap();
    mac.update(&v);
    mac.update(&[0x01]);
    mac.update(private_key);
    mac.update(message);
    k = mac.finalize().into_bytes().into();

    // V = HMAC_K(V)
    let mut mac = HmacSha256::new_from_slice(&k).unwrap();
    mac.update(&v);
    v = mac.finalize().into_bytes().into();

    // nonce = HMAC_K(V)
    let mut mac = HmacSha256::new_from_slice(&k).unwrap();
    mac.update(&v);
    let result: [u8; 32] = mac.finalize().into_bytes().into();

    if let Some(s) = bytes_to_scalar(result) {
        s
    } else {
        let reduced = bits2octets(&result);
        bytes_to_scalar(reduced).unwrap_or(Scalar::ZERO)
    }
}

// ─── Schnorr sign ─────────────────────────────────────────────────────────────

/// e = SHA256(Rx || compressed_P || message) mod N
fn get_e(p_affine: &AffinePoint, rx: &[u8; 32], message: &[u8; 32]) -> Scalar {
    let enc = p_affine.to_encoded_point(true); // compressed: 0x02/03 + x = 33 byte
    let mut input = Vec::with_capacity(97);
    input.extend_from_slice(rx);
    input.extend_from_slice(enc.as_bytes());
    input.extend_from_slice(message);

    let hash: [u8; 32] = Sha256::digest(&input).into();
    // Riduce hash mod N con al più una sottrazione (hash < 2^256 < 2N)
    let reduced = bits2octets(&hash);
    bytes_to_scalar(reduced).unwrap_or(Scalar::ZERO)
}

/// Firma Schnorr deterministica (schnorr.go → Sign).
/// Restituisce [0u8;64] se d è fuori range [1, N-1] (probabilità ~2^-128).
fn schnorr_sign(private_key: &[u8; 32], message: &[u8; 32]) -> [u8; 64] {
    // Valida d: deve essere in (0, N)
    let d_scalar = if *private_key == [0u8; 32] {
        return [0u8; 64];
    } else {
        match bytes_to_scalar(*private_key) {
            Some(s) => s,
            None => return [0u8; 64], // private_key >= N
        }
    };

    let k_scalar = rfc6979_nonce(private_key, message);

    // R = k * G
    let r_affine = (ProjectivePoint::GENERATOR * k_scalar).to_affine();
    let r_enc = r_affine.to_encoded_point(false);
    let x_slice: &[u8] = r_enc.x().unwrap();
    let rx_bytes: [u8; 32] = x_slice.try_into().unwrap();

    // P = d * G  (chiave pubblica)
    let p_affine = (ProjectivePoint::GENERATOR * d_scalar).to_affine();

    // e = SHA256(Rx || compressed_P || message) mod N
    let e_scalar = get_e(&p_affine, &rx_bytes, message);

    // s = (k + e * d) mod N  — tutte operazioni Scalar, nessun BigUint
    let s_scalar = k_scalar + e_scalar * d_scalar;

    let mut sig = [0u8; 64];
    sig[..32].copy_from_slice(&rx_bytes);
    sig[32..].copy_from_slice(s_scalar.to_repr().as_ref());
    sig
}

// ─── VersaHash pubblico ───────────────────────────────────────────────────────

/// Calcola VersaHash(data, nonce, extraNonce) → 32 byte.
///
/// Corrisponde esattamente a VersaHash() in crypto/versaHash/versaHash.go:
///   newData = data || byte(len(nonce)+len(extraNonce)) || nonce || extraNonce
pub fn versa_hash(data: &[u8], nonce: &[u8; 8], extra_nonce: &[u8; 8]) -> [u8; 32] {
    // Bug 1 fix: lunghezza calcolata dinamicamente come nel Go sorgente
    let len_byte = (nonce.len() + extra_nonce.len()) as u8; // = 16, ma semanticamente corretto

    let mut new_data = Vec::with_capacity(data.len() + 1 + 16);
    new_data.extend_from_slice(data);
    new_data.push(len_byte);
    new_data.extend_from_slice(nonce);
    new_data.extend_from_slice(extra_nonce);

    let first_hash: [u8; 32] = Sha256::digest(&new_data).into();
    let key_hash: [u8; 32] = Sha256::digest(&first_hash).into();
    let sign_data: [u8; 32] = Sha256::digest(&key_hash).into();

    let sig = schnorr_sign(&key_hash, &sign_data);

    let end_hash: [u8; 32] = Sha256::digest(&sig).into();
    let mut result = end_hash;
    result.reverse(); // byte-reverse come nel Go
    result
}

// ─── test ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versa_hash_is_deterministic() {
        let r1 = versa_hash(&[0u8; 32], &[0u8; 8], &[0u8; 8]);
        let r2 = versa_hash(&[0u8; 32], &[0u8; 8], &[0u8; 8]);
        assert_eq!(r1, r2);
    }

    #[test]
    fn different_nonces_differ() {
        let r1 = versa_hash(&[1u8; 32], &[0u8; 8], &[0u8; 8]);
        let r2 = versa_hash(&[1u8; 32], &[0, 0, 0, 0, 0, 0, 0, 1], &[0u8; 8]);
        assert_ne!(r1, r2);
    }

    #[test]
    fn bits2octets_identity_below_n() {
        let b = [0u8; 32]; // 0 < N
        assert_eq!(bits2octets(&b), b);
    }

    #[test]
    fn bits2octets_subtracts_n_above() {
        // N + 1 should give 1
        let mut b = CURVE_N;
        b[31] = b[31].wrapping_add(1);
        let res = bits2octets(&b);
        let mut expected = [0u8; 32];
        expected[31] = 1;
        assert_eq!(res, expected);
    }

    #[test]
    fn versa_hash_block_1000_real() {
        // Vettore reale dal blocco 1000 testnet INIChain
        // debug.getRawHeader("0x3e8") → sealHash calcolato via Python
        let seal_hash: [u8; 32] = hex::decode(
            "794c71e01331b1c9fd07b1f41749ebe8d8cc731783dad9fc5396f17a86f4eebb"
        ).unwrap().try_into().unwrap();

        let nonce: [u8; 8] = hex::decode("cc735a07241b71a7")
            .unwrap().try_into().unwrap();

        let extra: [u8; 8] = hex::decode("0056000100000000")
            .unwrap().try_into().unwrap();

        // target = 2^256 / difficulty(51833755)
        let target: [u8; 32] = hex::decode(
            "00000052dc453a085d62e743ab60791725bb1724200664b220e569799bb54902"
        ).unwrap().try_into().unwrap();

        let result = versa_hash(&seal_hash, &nonce, &extra);

        println!("result: {}", hex::encode(result));
        println!("target: {}", hex::encode(target));

        assert!(
            result <= target,
            "VersaHash scorretto!\nresult: {}\ntarget: {}",
            hex::encode(result),
            hex::encode(target)
        );
    }

    #[test]
    fn versa_hash_debug_steps() {
        let seal_hash: [u8; 32] = hex::decode(
            "794c71e01331b1c9fd07b1f41749ebe8d8cc731783dad9fc5396f17a86f4eebb"
        ).unwrap().try_into().unwrap();
        let nonce: [u8; 8] = hex::decode("cc735a07241b71a7").unwrap().try_into().unwrap();
        let extra: [u8; 8] = hex::decode("0056000100000000").unwrap().try_into().unwrap();

        // Step 1: costruisci newData
        let len_byte = (nonce.len() + extra.len()) as u8;
        let mut new_data = Vec::new();
        new_data.extend_from_slice(&seal_hash);
        new_data.push(len_byte);
        new_data.extend_from_slice(&nonce);
        new_data.extend_from_slice(&extra);
        println!("newData ({} B): {}", new_data.len(), hex::encode(&new_data));

        // Step 2-4: SHA256 in cascata
        use sha2::{Digest, Sha256};
        let first_hash: [u8; 32] = Sha256::digest(&new_data).into();
        let key_hash: [u8; 32] = Sha256::digest(&first_hash).into();
        let sign_data: [u8; 32] = Sha256::digest(&key_hash).into();
        println!("firstHash: {}", hex::encode(first_hash));
        println!("keyHash:   {}", hex::encode(key_hash));
        println!("signData:  {}", hex::encode(sign_data));

        // Step 5: firma
        let sig = schnorr_sign(&key_hash, &sign_data);
        println!("sig (64B): {}", hex::encode(sig));

        // Step 6-7
        let end_hash: [u8; 32] = Sha256::digest(&sig).into();
        let mut result = end_hash;
        result.reverse();
        println!("endHash:   {}", hex::encode(end_hash));
        println!("result:    {}", hex::encode(result));
    }

    /// SealHash nel nodo = Keccak256(RLP(header senza Nonce, ExtraNonce, MixDigest)).
    /// L'RLP completo (gen_header_rlp) termina con ... MixDigest(33B) Nonce(9B) ExtraNonce(9B) = 51B.
    /// Troncando 51B si ottiene un RLP che include ancora MixDigest; il vero SealHash è RLP di
    /// soli 17 campi (senza MixDigest), quindi non coincide con troncamento. Qui proviamo 18B e 51B.
    #[test]
    fn seal_hash_from_raw_header_block_1000() {
        let raw_hex = "f90248a0be7ed4592021bb5838f2abd504eef2e71ec08d60cdc33516065d5fc8651d9a76a01dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d4934794397c20792a1ec6a5fe56f700dce2ec391aa607f0a00b48c3fb3002a12947cd322d5f12da2f5794c0d69e01ac4a70938b692c6503bca056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421b9010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000840316eb9b8203e883e4e1c0808467583f6299d883010102846765746888676f312e32302e34856c696e7578a00000000000000000000000000000000000000000000000000000000000000000940000000000000000000000000000000000000000940000000000000000000000000000000000000000808088cc735a07241b71a7880056000100000000";
        let raw = hex::decode(raw_hex).unwrap();
        let expected = "794c71e01331b1c9fd07b1f41749ebe8d8cc731783dad9fc5396f17a86f4eebb";
        let expected_bytes: [u8; 32] = hex::decode(expected).unwrap().try_into().unwrap();

        for strip in [18_usize, 51_usize] {
            if raw.len() < strip {
                continue;
            }
            let rlp_cut = &raw[..raw.len() - strip];
            use sha2::Sha256;
            use sha3::Keccak256;
            let s256: [u8; 32] = Sha256::digest(rlp_cut).into();
            let k256: [u8; 32] = Keccak256::digest(rlp_cut).into();
            println!("strip {}: SHA256={} Keccak256={}", strip, hex::encode(s256), hex::encode(k256));
            if s256 == expected_bytes {
                println!("-> match SHA256 con strip {}", strip);
            }
            if k256 == expected_bytes {
                println!("-> match Keccak256 con strip {}", strip);
            }
        }
        println!("SealHash atteso: {}", expected);
        println!("(SealHash reale dal nodo = Keccak256(RLP(17 campi senza MixDigest/Nonce/ExtraNonce)))");
    }
}
