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
/// - **Schnorr** (crypto/versaHash/schnorr.go): getE(Px,Py,rX,m) = SHA256(rX || Marshal_compressed(P) || m) mod N;
///   getK(Ry,k0) = k0 se Jacobi(Ry,P)==1 altrimenti N-k0. Tag RFC6979: "Schnorr+SHA256  " (16 byte).
///
/// Nessuna dipendenza da num-bigint: tutto il lavoro aritmetico usa
/// k256::Scalar (mod N) e k256::FieldElement (mod P) direttamente.

use hmac::{Hmac, Mac};
use k256::{
    elliptic_curve::{sec1::ToEncodedPoint, PrimeField},
    AffinePoint, FieldBytes, FieldElement, ProjectivePoint, Scalar,
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

// ─── Jacobi: residuo quadratico mod P via FieldElement::sqrt ─────────────────

/// Restituisce true se y è un residuo quadratico mod P  (Jacobi(y,P) == 1).
/// Usa l'operazione sqrt() ottimizzata di k256 (P ≡ 3 mod 4 ⟹ sqrt = y^((P+1)/4)).
/// Molto più veloce di BigUint::modpow con esponente a 256 bit.
fn is_quadratic_residue(y_bytes: &[u8; 32]) -> bool {
    let fb = FieldBytes::from(*y_bytes);
    // and_then propaga None se from_bytes fallisce O se sqrt non esiste (non-QR)
    bool::from(FieldElement::from_bytes(&fb).and_then(|fe| fe.sqrt()).is_some())
}

// ─── HMAC-SHA256 helper ───────────────────────────────────────────────────────

fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    let mut h = HmacSha256::new_from_slice(key).unwrap();
    for p in parts {
        h.update(p);
    }
    h.finalize().into_bytes().to_vec()
}

// ─── RFC 6979 — nonce deterministico ─────────────────────────────────────────
//
// Implementa generateSecret da rfc6979.go con:
//   - tag "Schnorr+SHA256  " (16 byte)
//   - qlen = holen = rolen = 32 (SHA-256, secp256k1)
// Ritorna direttamente un Scalar (< N, != 0): nessuna conversione BigUint.
fn rfc6979_nonce(private_key: &[u8; 32], message: &[u8; 32]) -> Scalar {
    // bx = private_key(32) || bits2octets(message)(32) || "Schnorr+SHA256  "(16)
    let b2o = bits2octets(message);
    let schnorr_tag = b"Schnorr+SHA256  "; // 16 byte esatti
    let mut bx = [0u8; 80];
    bx[..32].copy_from_slice(private_key);
    bx[32..64].copy_from_slice(&b2o);
    bx[64..].copy_from_slice(schnorr_tag);

    let mut v = vec![0x01u8; 32];
    let mut k = vec![0x00u8; 32];

    // Steps D–G dell'RFC 6979 §3.2
    k = hmac_sha256(&k, &[&v, &[0x00], &bx]);
    v = hmac_sha256(&k, &[&v]);
    k = hmac_sha256(&k, &[&v, &[0x01], &bx]);
    v = hmac_sha256(&k, &[&v]);

    // Step H: per qlen=256 e HMAC-SHA256, t è sempre esattamente un output (32 byte).
    // Usiamo Scalar::from_repr per il range-check 0 < secret < N, senza BigUint.
    loop {
        v = hmac_sha256(&k, &[&v]);
        let t: [u8; 32] = v.as_slice().try_into().unwrap();

        // from_repr ritorna None se t >= N; se t == 0 from_repr ritorna Some(ZERO)
        if t != [0u8; 32] {
            if let Some(s) = bytes_to_scalar(t) {
                return s;
            }
        }

        k = hmac_sha256(&k, &[&v, &[0x00]]);
        v = hmac_sha256(&k, &[&v]);
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

    let k0_scalar = rfc6979_nonce(private_key, message);

    // R = k0 * G
    let r_affine = (ProjectivePoint::GENERATOR * k0_scalar).to_affine();
    let r_enc = r_affine.to_encoded_point(false); // uncompressed per estrarre y
    // coercion esplicita &FieldBytes → &[u8] tramite Deref, evita ambiguità as_ref()
    let x_slice: &[u8] = r_enc.x().unwrap();
    let rx_bytes: [u8; 32] = x_slice.try_into().unwrap();
    let y_slice: &[u8] = r_enc.y().unwrap();
    let ry_bytes: [u8; 32] = y_slice.try_into().unwrap();

    // k = k0 se Jacobi(Ry, P) == 1, altrimenti k = N - k0
    let k_scalar = if is_quadratic_residue(&ry_bytes) {
        k0_scalar
    } else {
        -k0_scalar // Neg::neg ≡ N - k0_scalar in Scalar field
    };

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
    versa_hash_from_key_material(&key_hash, &sign_data)
}

/// Calcola la parte finale di VersaHash a partire dal materiale già preparato:
/// - key_hash = SHA256(first_hash)
/// - sign_data = SHA256(key_hash)
///
/// Utile per pipeline ibride (es. precomputo su GPU, firma/finalizzazione su CPU)
/// mantenendo output identico all'implementazione chain.
pub fn versa_hash_from_key_material(key_hash: &[u8; 32], sign_data: &[u8; 32]) -> [u8; 32] {
    let sig = schnorr_sign(key_hash, sign_data);
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
        // Vettore reale dal blocco 1000 testnet INIChain.
        // Il risultato atteso è verificato contro l'implementazione Go ufficiale:
        // github.com/Project-InitVerse/chain/crypto/versaHash.VersaHash
        let seal_hash: [u8; 32] = hex::decode(
            "794c71e01331b1c9fd07b1f41749ebe8d8cc731783dad9fc5396f17a86f4eebb"
        ).unwrap().try_into().unwrap();

        let nonce: [u8; 8] = hex::decode("cc735a07241b71a7")
            .unwrap().try_into().unwrap();

        let extra: [u8; 8] = hex::decode("0056000100000000")
            .unwrap().try_into().unwrap();

        let result = versa_hash(&seal_hash, &nonce, &extra);
        let expected: [u8; 32] = hex::decode(
            "7a4168565d4b7dbccfba33064f3d067993f90f6af623011dcd67293fcd67d7eb"
        ).unwrap().try_into().unwrap();
        assert_eq!(result, expected, "VersaHash non allineato al chain Go");
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
