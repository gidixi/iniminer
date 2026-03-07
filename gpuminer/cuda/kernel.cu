#include <cuda_runtime.h>
#include <cstdint>
#include <cstdio>

namespace {

inline int fail_cuda(const char *fn, int stage, cudaError_t st) {
    std::fprintf(stderr,
                 "[gpuminer][cuda] %s stage=%d code=%d msg=%s\n",
                 fn,
                 stage,
                 static_cast<int>(st),
                 cudaGetErrorString(st));
    return -(stage * 1000 + static_cast<int>(st));
}

struct U256 {
    uint64_t v[4]; // little-endian limbs
};

struct PointA {
    U256 x;
    U256 y;
    bool inf;
};

struct PointJ {
    U256 x;
    U256 y;
    U256 z;
    bool inf;
};

__device__ __constant__ uint64_t P_LIMBS[4] = {
    0xFFFFFFFEFFFFFC2FULL, 0xFFFFFFFFFFFFFFFFULL, 0xFFFFFFFFFFFFFFFFULL, 0xFFFFFFFFFFFFFFFFULL};
__device__ __constant__ uint64_t N_LIMBS[4] = {
    0xBFD25E8CD0364141ULL, 0xBAAEDCE6AF48A03BULL, 0xFFFFFFFFFFFFFFFEULL, 0xFFFFFFFFFFFFFFFFULL};
__device__ __constant__ uint64_t GX_LIMBS[4] = {
    0x59F2815B16F81798ULL, 0x029BFCDB2DCE28D9ULL, 0x55A06295CE870B07ULL, 0x79BE667EF9DCBBACULL};
__device__ __constant__ uint64_t GY_LIMBS[4] = {
    0x9C47D08FFB10D4B8ULL, 0xFD17B448A6855419ULL, 0x5DA4FBFC0E1108A8ULL, 0x483ADA7726A3C465ULL};
__device__ __constant__ uint64_t P_MINUS_2_LIMBS[4] = {
    0xFFFFFFFEFFFFFC2DULL, 0xFFFFFFFFFFFFFFFFULL, 0xFFFFFFFFFFFFFFFFULL, 0xFFFFFFFFFFFFFFFFULL};
__device__ __constant__ uint64_t P_MINUS_1_DIV_2_LIMBS[4] = {
    0xFFFFFFFF7FFFFE17ULL, 0xFFFFFFFFFFFFFFFFULL, 0xFFFFFFFFFFFFFFFFULL, 0x7FFFFFFFFFFFFFFFULL};

__constant__ uint32_t K256[64] = {
    0x428a2f98u, 0x71374491u, 0xb5c0fbcfu, 0xe9b5dba5u, 0x3956c25bu, 0x59f111f1u,
    0x923f82a4u, 0xab1c5ed5u, 0xd807aa98u, 0x12835b01u, 0x243185beu, 0x550c7dc3u,
    0x72be5d74u, 0x80deb1feu, 0x9bdc06a7u, 0xc19bf174u, 0xe49b69c1u, 0xefbe4786u,
    0x0fc19dc6u, 0x240ca1ccu, 0x2de92c6fu, 0x4a7484aau, 0x5cb0a9dcu, 0x76f988dau,
    0x983e5152u, 0xa831c66du, 0xb00327c8u, 0xbf597fc7u, 0xc6e00bf3u, 0xd5a79147u,
    0x06ca6351u, 0x14292967u, 0x27b70a85u, 0x2e1b2138u, 0x4d2c6dfcu, 0x53380d13u,
    0x650a7354u, 0x766a0abbu, 0x81c2c92eu, 0x92722c85u, 0xa2bfe8a1u, 0xa81a664bu,
    0xc24b8b70u, 0xc76c51a3u, 0xd192e819u, 0xd6990624u, 0xf40e3585u, 0x106aa070u,
    0x19a4c116u, 0x1e376c08u, 0x2748774cu, 0x34b0bcb5u, 0x391c0cb3u, 0x4ed8aa4au,
    0x5b9cca4fu, 0x682e6ff3u, 0x748f82eeu, 0x78a5636fu, 0x84c87814u, 0x8cc70208u,
    0x90befffau, 0xa4506cebu, 0xbef9a3f7u, 0xc67178f2u};

// ---------- U256 helpers ----------
__device__ __forceinline__ U256 u256_zero() { return {{0, 0, 0, 0}}; }
__device__ __forceinline__ U256 u256_one() { return {{1, 0, 0, 0}}; }
__device__ __forceinline__ U256 u256_from_u64(uint64_t x) { return {{x, 0, 0, 0}}; }
__device__ __forceinline__ U256 u256_from_const(const uint64_t c[4]) {
    return {{c[0], c[1], c[2], c[3]}};
}
__device__ __forceinline__ bool u256_is_zero(const U256 &a) {
    return (a.v[0] | a.v[1] | a.v[2] | a.v[3]) == 0;
}
__device__ __forceinline__ int u256_cmp(const U256 &a, const U256 &b) {
    for (int i = 3; i >= 0; --i) {
        if (a.v[i] < b.v[i]) return -1;
        if (a.v[i] > b.v[i]) return 1;
    }
    return 0;
}
__device__ void u256_from_be_bytes(const uint8_t in[32], U256 *out) {
    for (int li = 0; li < 4; ++li) {
        int off = (3 - li) * 8;
        uint64_t x = 0;
        for (int i = 0; i < 8; ++i) x = (x << 8) | in[off + i];
        out->v[li] = x;
    }
}
__device__ void u256_to_be_bytes(const U256 &in, uint8_t out[32]) {
    for (int li = 0; li < 4; ++li) {
        uint64_t x = in.v[li];
        int off = (3 - li) * 8;
        for (int i = 7; i >= 0; --i) {
            out[off + i] = static_cast<uint8_t>(x & 0xff);
            x >>= 8;
        }
    }
}
__device__ __forceinline__ int u256_get_bit(const U256 &a, int bit) {
    return static_cast<int>((a.v[bit >> 6] >> (bit & 63)) & 1ULL);
}
__device__ __forceinline__ int u512_get_bit(const uint64_t x[8], int bit) {
    return static_cast<int>((x[bit >> 6] >> (bit & 63)) & 1ULL);
}
__device__ void u256_shl1_addbit(U256 *a, uint32_t bit) {
    uint64_t carry = bit & 1U;
    for (int i = 0; i < 4; ++i) {
        uint64_t nc = a->v[i] >> 63;
        a->v[i] = (a->v[i] << 1) | carry;
        carry = nc;
    }
}
__device__ uint64_t u256_add_raw(U256 *out, const U256 &a, const U256 &b) {
    unsigned __int128 carry = 0;
    for (int i = 0; i < 4; ++i) {
        unsigned __int128 t = static_cast<unsigned __int128>(a.v[i]) + b.v[i] + carry;
        out->v[i] = static_cast<uint64_t>(t);
        carry = t >> 64;
    }
    return static_cast<uint64_t>(carry);
}
__device__ uint64_t u256_sub_raw(U256 *out, const U256 &a, const U256 &b) {
    uint64_t borrow = 0;
    for (int i = 0; i < 4; ++i) {
        unsigned __int128 ai = a.v[i];
        unsigned __int128 bi = static_cast<unsigned __int128>(b.v[i]) + borrow;
        if (ai >= bi) {
            out->v[i] = static_cast<uint64_t>(ai - bi);
            borrow = 0;
        } else {
            out->v[i] = static_cast<uint64_t>((static_cast<unsigned __int128>(1) << 64) + ai - bi);
            borrow = 1;
        }
    }
    return borrow;
}
__device__ void u256_mul_512(const U256 &a, const U256 &b, uint64_t out[8]) {
    for (int i = 0; i < 8; ++i) out[i] = 0;
    for (int i = 0; i < 4; ++i) {
        unsigned __int128 carry = 0;
        for (int j = 0; j < 4; ++j) {
            unsigned __int128 t = static_cast<unsigned __int128>(a.v[i]) * b.v[j] + out[i + j] + carry;
            out[i + j] = static_cast<uint64_t>(t);
            carry = t >> 64;
        }
        int k = i + 4;
        while (carry != 0 && k < 8) {
            unsigned __int128 t = static_cast<unsigned __int128>(out[k]) + carry;
            out[k] = static_cast<uint64_t>(t);
            carry = t >> 64;
            ++k;
        }
    }
}
__device__ U256 mod_reduce_512(const uint64_t in[8], const U256 &mod) {
    U256 r = u256_zero();
    for (int bit = 511; bit >= 0; --bit) {
        u256_shl1_addbit(&r, static_cast<uint32_t>(u512_get_bit(in, bit)));
        if (u256_cmp(r, mod) >= 0) {
            U256 t;
            u256_sub_raw(&t, r, mod);
            r = t;
        }
    }
    return r;
}
__device__ U256 mod_add(const U256 &a, const U256 &b, const U256 &mod) {
    U256 s;
    uint64_t carry = u256_add_raw(&s, a, b);
    if (carry || u256_cmp(s, mod) >= 0) {
        U256 t;
        u256_sub_raw(&t, s, mod);
        return t;
    }
    return s;
}
__device__ U256 mod_sub(const U256 &a, const U256 &b, const U256 &mod) {
    if (u256_cmp(a, b) >= 0) {
        U256 t;
        u256_sub_raw(&t, a, b);
        return t;
    }
    U256 d, t;
    u256_sub_raw(&d, b, a);
    u256_sub_raw(&t, mod, d);
    return t;
}
__device__ U256 mod_mul(const U256 &a, const U256 &b, const U256 &mod) {
    uint64_t p[8];
    u256_mul_512(a, b, p);
    return mod_reduce_512(p, mod);
}
__device__ U256 mod_sqr(const U256 &a, const U256 &mod) { return mod_mul(a, a, mod); }
__device__ U256 mod_pow(U256 base, const U256 &exp, const U256 &mod) {
    U256 r = u256_one();
    for (int bit = 255; bit >= 0; --bit) {
        r = mod_sqr(r, mod);
        if (u256_get_bit(exp, bit)) r = mod_mul(r, base, mod);
    }
    return r;
}
__device__ bool bytes_leq_be(const uint8_t a[32], const uint8_t b[32]) {
    for (int i = 0; i < 32; ++i) {
        if (a[i] < b[i]) return true;
        if (a[i] > b[i]) return false;
    }
    return true;
}

// ---------- SHA-256 / HMAC ----------
__device__ __forceinline__ uint32_t rotr(uint32_t x, uint32_t n) {
    return (x >> n) | (x << (32 - n));
}
__device__ __forceinline__ uint32_t ch(uint32_t x, uint32_t y, uint32_t z) {
    return (x & y) ^ (~x & z);
}
__device__ __forceinline__ uint32_t maj(uint32_t x, uint32_t y, uint32_t z) {
    return (x & y) ^ (x & z) ^ (y & z);
}
__device__ __forceinline__ uint32_t ep0(uint32_t x) {
    return rotr(x, 2) ^ rotr(x, 13) ^ rotr(x, 22);
}
__device__ __forceinline__ uint32_t ep1(uint32_t x) {
    return rotr(x, 6) ^ rotr(x, 11) ^ rotr(x, 25);
}
__device__ __forceinline__ uint32_t sig0(uint32_t x) {
    return rotr(x, 7) ^ rotr(x, 18) ^ (x >> 3);
}
__device__ __forceinline__ uint32_t sig1(uint32_t x) {
    return rotr(x, 17) ^ rotr(x, 19) ^ (x >> 10);
}
__device__ void sha256_compress(uint32_t state[8], const uint8_t block[64]) {
    uint32_t w[64];
    #pragma unroll
    for (int i = 0; i < 16; ++i) {
        int j = i * 4;
        w[i] = (static_cast<uint32_t>(block[j]) << 24) |
               (static_cast<uint32_t>(block[j + 1]) << 16) |
               (static_cast<uint32_t>(block[j + 2]) << 8) |
               (static_cast<uint32_t>(block[j + 3]));
    }
    #pragma unroll
    for (int i = 16; i < 64; ++i) w[i] = sig1(w[i - 2]) + w[i - 7] + sig0(w[i - 15]) + w[i - 16];

    uint32_t a = state[0], b = state[1], c = state[2], d = state[3];
    uint32_t e = state[4], f = state[5], g = state[6], h = state[7];
    #pragma unroll
    for (int i = 0; i < 64; ++i) {
        uint32_t t1 = h + ep1(e) + ch(e, f, g) + K256[i] + w[i];
        uint32_t t2 = ep0(a) + maj(a, b, c);
        h = g; g = f; f = e; e = d + t1;
        d = c; c = b; b = a; a = t1 + t2;
    }
    state[0] += a; state[1] += b; state[2] += c; state[3] += d;
    state[4] += e; state[5] += f; state[6] += g; state[7] += h;
}
__device__ void sha256_bytes(const uint8_t *data, uint32_t len, uint8_t out[32]) {
    uint32_t state[8] = {
        0x6a09e667u, 0xbb67ae85u, 0x3c6ef372u, 0xa54ff53au,
        0x510e527fu, 0x9b05688cu, 0x1f83d9abu, 0x5be0cd19u};

    uint32_t offset = 0;
    while (offset + 64 <= len) {
        sha256_compress(state, data + offset);
        offset += 64;
    }

    uint8_t tail[128];
    uint32_t rem = len - offset;
    for (uint32_t i = 0; i < rem; ++i) tail[i] = data[offset + i];
    tail[rem] = 0x80;

    uint64_t bit_len = static_cast<uint64_t>(len) * 8ULL;
    if (rem + 1 + 8 <= 64) {
        for (uint32_t i = rem + 1; i < 56; ++i) tail[i] = 0;
        for (int i = 0; i < 8; ++i) tail[56 + i] = static_cast<uint8_t>((bit_len >> ((7 - i) * 8)) & 0xff);
        sha256_compress(state, tail);
    } else {
        for (uint32_t i = rem + 1; i < 64; ++i) tail[i] = 0;
        for (uint32_t i = 64; i < 120; ++i) tail[i] = 0;
        for (int i = 0; i < 8; ++i) tail[120 + i] = static_cast<uint8_t>((bit_len >> ((7 - i) * 8)) & 0xff);
        sha256_compress(state, tail);
        sha256_compress(state, tail + 64);
    }

    for (int i = 0; i < 8; ++i) {
        out[i * 4 + 0] = static_cast<uint8_t>((state[i] >> 24) & 0xff);
        out[i * 4 + 1] = static_cast<uint8_t>((state[i] >> 16) & 0xff);
        out[i * 4 + 2] = static_cast<uint8_t>((state[i] >> 8) & 0xff);
        out[i * 4 + 3] = static_cast<uint8_t>(state[i] & 0xff);
    }
}
__device__ bool hmac_sha256(const uint8_t *key, uint32_t key_len,
                            const uint8_t *msg, uint32_t msg_len,
                            uint8_t out[32]) {
    if (msg_len > 192) return false;

    uint8_t k0[64];
    for (int i = 0; i < 64; ++i) k0[i] = 0;
    if (key_len > 64) {
        uint8_t kh[32];
        sha256_bytes(key, key_len, kh);
        for (int i = 0; i < 32; ++i) k0[i] = kh[i];
    } else {
        for (uint32_t i = 0; i < key_len; ++i) k0[i] = key[i];
    }

    uint8_t inner[256];
    for (int i = 0; i < 64; ++i) inner[i] = k0[i] ^ 0x36;
    for (uint32_t i = 0; i < msg_len; ++i) inner[64 + i] = msg[i];
    uint8_t inner_hash[32];
    sha256_bytes(inner, 64 + msg_len, inner_hash);

    uint8_t outer[96];
    for (int i = 0; i < 64; ++i) outer[i] = k0[i] ^ 0x5c;
    for (int i = 0; i < 32; ++i) outer[64 + i] = inner_hash[i];
    sha256_bytes(outer, 96, out);
    return true;
}

// ---------- secp256k1 point ops ----------
__device__ PointJ pointj_inf() { return {u256_zero(), u256_one(), u256_zero(), true}; }
__device__ PointJ pointj_from_affine(const PointA &p) {
    if (p.inf) return pointj_inf();
    return {p.x, p.y, u256_one(), false};
}
__device__ PointA pointa_G() {
    return {u256_from_const(GX_LIMBS), u256_from_const(GY_LIMBS), false};
}
__device__ PointJ point_double(const PointJ &p, const U256 &modp) {
    if (p.inf || u256_is_zero(p.y)) return pointj_inf();
    U256 y2 = mod_sqr(p.y, modp);
    U256 x_y2 = mod_mul(p.x, y2, modp);
    U256 s = mod_add(x_y2, x_y2, modp);
    s = mod_add(s, s, modp); // *4

    U256 x2 = mod_sqr(p.x, modp);
    U256 m = mod_add(x2, x2, modp);
    m = mod_add(m, x2, modp); // *3

    U256 x3 = mod_sub(mod_sqr(m, modp), mod_add(s, s, modp), modp);
    U256 y4 = mod_sqr(y2, modp);
    U256 eight_y4 = mod_add(y4, y4, modp);
    eight_y4 = mod_add(eight_y4, eight_y4, modp);
    eight_y4 = mod_add(eight_y4, eight_y4, modp); // *8
    U256 y3 = mod_sub(mod_mul(m, mod_sub(s, x3, modp), modp), eight_y4, modp);
    U256 z3 = mod_add(mod_mul(p.y, p.z, modp), mod_mul(p.y, p.z, modp), modp); // *2
    return {x3, y3, z3, false};
}
__device__ PointJ point_add_mixed(const PointJ &p, const PointA &q, const U256 &modp) {
    if (q.inf) return p;
    if (p.inf) return pointj_from_affine(q);

    U256 z1z1 = mod_sqr(p.z, modp);
    U256 u2 = mod_mul(q.x, z1z1, modp);
    U256 s2 = mod_mul(q.y, mod_mul(z1z1, p.z, modp), modp);

    U256 h = mod_sub(u2, p.x, modp);
    U256 r = mod_sub(s2, p.y, modp);
    if (u256_is_zero(h)) {
        if (u256_is_zero(r)) return point_double(p, modp);
        return pointj_inf();
    }

    U256 hh = mod_sqr(h, modp);
    U256 hhh = mod_mul(hh, h, modp);
    U256 v = mod_mul(p.x, hh, modp);
    U256 x3 = mod_sub(mod_sub(mod_sqr(r, modp), hhh, modp), mod_add(v, v, modp), modp);
    U256 y3 = mod_sub(mod_mul(r, mod_sub(v, x3, modp), modp), mod_mul(p.y, hhh, modp), modp);
    U256 z3 = mod_mul(p.z, h, modp);
    return {x3, y3, z3, false};
}
__device__ PointA point_to_affine(const PointJ &p, const U256 &modp) {
    if (p.inf) return {u256_zero(), u256_zero(), true};
    U256 zinv = mod_pow(p.z, u256_from_const(P_MINUS_2_LIMBS), modp);
    U256 zinv2 = mod_sqr(zinv, modp);
    U256 x = mod_mul(p.x, zinv2, modp);
    U256 y = mod_mul(p.y, mod_mul(zinv2, zinv, modp), modp);
    return {x, y, false};
}
__device__ PointA scalar_mul_G(const U256 &k, const U256 &modp) {
    PointA g = pointa_G();
    PointJ r = pointj_inf();
    for (int bit = 255; bit >= 0; --bit) {
        if (!r.inf) r = point_double(r, modp);
        if (u256_get_bit(k, bit)) r = point_add_mixed(r, g, modp);
    }
    return point_to_affine(r, modp);
}

// ---------- draft Schnorr helpers ----------
__device__ U256 bits2octets_u256(const uint8_t in[32], const U256 &modn) {
    U256 x;
    u256_from_be_bytes(in, &x);
    if (u256_cmp(x, modn) >= 0) {
        U256 t;
        u256_sub_raw(&t, x, modn);
        return t;
    }
    return x;
}
__device__ bool scalar_from_bytes_strict(const uint8_t in[32], const U256 &modn, U256 *out) {
    u256_from_be_bytes(in, out);
    if (u256_is_zero(*out)) return false;
    return u256_cmp(*out, modn) < 0;
}
__device__ bool rfc6979_nonce_draft(const uint8_t privkey[32], const uint8_t msg[32], const U256 &modn, U256 *out_k) {
    uint8_t b2o_bytes[32];
    U256 b2o = bits2octets_u256(msg, modn);
    u256_to_be_bytes(b2o, b2o_bytes);

    const uint8_t tag[16] = {'S','c','h','n','o','r','r','+','S','H','A','2','5','6',' ',' '};
    uint8_t bx[80];
    for (int i = 0; i < 32; ++i) bx[i] = privkey[i];
    for (int i = 0; i < 32; ++i) bx[32 + i] = b2o_bytes[i];
    for (int i = 0; i < 16; ++i) bx[64 + i] = tag[i];

    uint8_t v[32];
    uint8_t k[32];
    for (int i = 0; i < 32; ++i) { v[i] = 0x01; k[i] = 0x00; }

    uint8_t m1[113];
    for (int i = 0; i < 32; ++i) m1[i] = v[i];
    m1[32] = 0x00;
    for (int i = 0; i < 80; ++i) m1[33 + i] = bx[i];
    if (!hmac_sha256(k, 32, m1, 113, k)) return false;
    if (!hmac_sha256(k, 32, v, 32, v)) return false;

    for (int i = 0; i < 32; ++i) m1[i] = v[i];
    m1[32] = 0x01;
    for (int i = 0; i < 80; ++i) m1[33 + i] = bx[i];
    if (!hmac_sha256(k, 32, m1, 113, k)) return false;
    if (!hmac_sha256(k, 32, v, 32, v)) return false;

    uint8_t tmp[33];
    for (int iter = 0; iter < 16; ++iter) {
        if (!hmac_sha256(k, 32, v, 32, v)) return false;
        U256 t;
        if (scalar_from_bytes_strict(v, modn, &t)) {
            *out_k = t;
            return true;
        }
        for (int i = 0; i < 32; ++i) tmp[i] = v[i];
        tmp[32] = 0x00;
        if (!hmac_sha256(k, 32, tmp, 33, k)) return false;
        if (!hmac_sha256(k, 32, v, 32, v)) return false;
    }
    return false;
}
__device__ bool is_quadratic_residue(const U256 &y, const U256 &modp) {
    U256 r = mod_pow(y, u256_from_const(P_MINUS_1_DIV_2_LIMBS), modp);
    return u256_cmp(r, u256_one()) == 0;
}
__device__ bool schnorr_sign_draft(const uint8_t privkey[32], const uint8_t msg[32], uint8_t sig_out[64]) {
    U256 modp = u256_from_const(P_LIMBS);
    U256 modn = u256_from_const(N_LIMBS);

    U256 d;
    if (!scalar_from_bytes_strict(privkey, modn, &d)) return false;

    U256 k0;
    if (!rfc6979_nonce_draft(privkey, msg, modn, &k0)) return false;

    PointA R = scalar_mul_G(k0, modp);
    if (R.inf) return false;

    U256 k = k0;
    if (!is_quadratic_residue(R.y, modp)) {
        U256 t;
        u256_sub_raw(&t, modn, k0);
        k = t;
    }

    PointA P = scalar_mul_G(d, modp);
    if (P.inf) return false;

    uint8_t rx[32];
    u256_to_be_bytes(R.x, rx);
    uint8_t px[32];
    u256_to_be_bytes(P.x, px);
    uint8_t hinput[97];
    for (int i = 0; i < 32; ++i) hinput[i] = rx[i];
    hinput[32] = static_cast<uint8_t>(0x02 + (P.y.v[0] & 1ULL));
    for (int i = 0; i < 32; ++i) hinput[33 + i] = px[i];
    for (int i = 0; i < 32; ++i) hinput[65 + i] = msg[i];

    uint8_t eh[32];
    sha256_bytes(hinput, 97, eh);
    U256 e = bits2octets_u256(eh, modn);
    U256 ed = mod_mul(e, d, modn);
    U256 s = mod_add(k, ed, modn);
    uint8_t sb[32];
    u256_to_be_bytes(s, sb);

    for (int i = 0; i < 32; ++i) sig_out[i] = rx[i];
    for (int i = 0; i < 32; ++i) sig_out[32 + i] = sb[i];
    return true;
}

// ---------- VersaHash ----------
__device__ bool versahash_full_check(const uint8_t header_hash[32],
                                     const uint8_t extra_nonce[8],
                                     uint64_t nonce,
                                     const uint8_t target[32]) {
    uint8_t msg49[49];
    for (int i = 0; i < 32; ++i) msg49[i] = header_hash[i];
    msg49[32] = 16;
    msg49[33] = static_cast<uint8_t>((nonce >> 56) & 0xff);
    msg49[34] = static_cast<uint8_t>((nonce >> 48) & 0xff);
    msg49[35] = static_cast<uint8_t>((nonce >> 40) & 0xff);
    msg49[36] = static_cast<uint8_t>((nonce >> 32) & 0xff);
    msg49[37] = static_cast<uint8_t>((nonce >> 24) & 0xff);
    msg49[38] = static_cast<uint8_t>((nonce >> 16) & 0xff);
    msg49[39] = static_cast<uint8_t>((nonce >> 8) & 0xff);
    msg49[40] = static_cast<uint8_t>(nonce & 0xff);
    for (int i = 0; i < 8; ++i) msg49[41 + i] = extra_nonce[i];

    uint8_t first_hash[32];
    uint8_t key_hash[32];
    uint8_t sign_data[32];
    sha256_bytes(msg49, 49, first_hash);
    sha256_bytes(first_hash, 32, key_hash);
    sha256_bytes(key_hash, 32, sign_data);

    uint8_t sig[64];
    if (!schnorr_sign_draft(key_hash, sign_data, sig)) return false;

    uint8_t end_hash[32];
    uint8_t result[32];
    sha256_bytes(sig, 64, end_hash);
    for (int i = 0; i < 32; ++i) result[i] = end_hash[31 - i];
    return bytes_leq_be(result, target);
}

// ---------- Kernels ----------
__global__ void gpuminer_prepare_batch_kernel(const uint8_t* header_hash,
                                              const uint8_t* extra_nonce,
                                              uint64_t start_nonce,
                                              uint64_t count,
                                              uint8_t* out_key_hashes,
                                              uint8_t* out_sign_data_hashes) {
    const uint64_t idx = static_cast<uint64_t>(blockIdx.x) * blockDim.x + threadIdx.x;
    if (idx >= count) return;

    const uint64_t nonce = start_nonce + idx;
    uint8_t msg[49];
    for (int i = 0; i < 32; ++i) msg[i] = header_hash[i];
    msg[32] = 16;
    msg[33] = static_cast<uint8_t>((nonce >> 56) & 0xff);
    msg[34] = static_cast<uint8_t>((nonce >> 48) & 0xff);
    msg[35] = static_cast<uint8_t>((nonce >> 40) & 0xff);
    msg[36] = static_cast<uint8_t>((nonce >> 32) & 0xff);
    msg[37] = static_cast<uint8_t>((nonce >> 24) & 0xff);
    msg[38] = static_cast<uint8_t>((nonce >> 16) & 0xff);
    msg[39] = static_cast<uint8_t>((nonce >> 8) & 0xff);
    msg[40] = static_cast<uint8_t>(nonce & 0xff);
    for (int i = 0; i < 8; ++i) msg[41 + i] = extra_nonce[i];

    uint8_t first_hash[32];
    uint8_t key_hash[32];
    uint8_t sign_data_hash[32];
    sha256_bytes(msg, 49, first_hash);
    sha256_bytes(first_hash, 32, key_hash);
    sha256_bytes(key_hash, 32, sign_data_hash);

    const uint64_t offset = idx * 32ULL;
    for (int i = 0; i < 32; ++i) {
        out_key_hashes[offset + i] = key_hash[i];
        out_sign_data_hashes[offset + i] = sign_data_hash[i];
    }
}

__global__ void gpuminer_full_scan_kernel(const uint8_t *header_hash,
                                          const uint8_t *extra_nonce,
                                          uint64_t start_nonce,
                                          uint64_t count,
                                          const uint8_t *target,
                                          uint64_t *found_nonce,
                                          int *found) {
    const uint64_t idx = static_cast<uint64_t>(blockIdx.x) * blockDim.x + threadIdx.x;
    if (idx >= count) return;
    if (atomicAdd(found, 0) != 0) return;

    const uint64_t nonce = start_nonce + idx;
    if (versahash_full_check(header_hash, extra_nonce, nonce, target)) {
        if (atomicCAS(found, 0, 1) == 0) {
            *found_nonce = nonce;
        }
    }
}

}  // namespace

extern "C" int gpuminer_prepare_batch_cuda(const uint8_t* header_hash,
                                           const uint8_t* extra_nonce,
                                           uint64_t start_nonce,
                                           uint64_t count,
                                           uint8_t* out_key_hashes,
                                           uint8_t* out_sign_data_hashes) {
    if (!header_hash || !extra_nonce || !out_key_hashes || !out_sign_data_hashes) return -1;
    if (count == 0) return 0;

    constexpr int threads = 128;
    const int blocks = static_cast<int>((count + threads - 1) / threads);

    uint8_t* d_header = nullptr;
    uint8_t* d_extra = nullptr;
    uint8_t* d_key = nullptr;
    uint8_t* d_sign = nullptr;
    const size_t batch_bytes = static_cast<size_t>(count) * 32ULL;

    cudaError_t st_alloc = cudaMalloc(&d_header, 32);
    if (st_alloc != cudaSuccess) return fail_cuda("prepare_batch", 1, st_alloc);
    st_alloc = cudaMalloc(&d_extra, 8);
    if (st_alloc != cudaSuccess) { cudaFree(d_header); return fail_cuda("prepare_batch", 2, st_alloc); }
    st_alloc = cudaMalloc(&d_key, batch_bytes);
    if (st_alloc != cudaSuccess) { cudaFree(d_header); cudaFree(d_extra); return fail_cuda("prepare_batch", 3, st_alloc); }
    st_alloc = cudaMalloc(&d_sign, batch_bytes);
    if (st_alloc != cudaSuccess) { cudaFree(d_header); cudaFree(d_extra); cudaFree(d_key); return fail_cuda("prepare_batch", 4, st_alloc); }

    cudaError_t st = cudaMemcpy(d_header, header_hash, 32, cudaMemcpyHostToDevice);
    if (st != cudaSuccess) goto fail_10;
    st = cudaMemcpy(d_extra, extra_nonce, 8, cudaMemcpyHostToDevice);
    if (st != cudaSuccess) goto fail_11;

    gpuminer_prepare_batch_kernel<<<blocks, threads>>>(d_header, d_extra, start_nonce, count, d_key, d_sign);
    st = cudaGetLastError(); if (st != cudaSuccess) goto fail_12;
    st = cudaDeviceSynchronize(); if (st != cudaSuccess) goto fail_13;

    st = cudaMemcpy(out_key_hashes, d_key, batch_bytes, cudaMemcpyDeviceToHost);
    if (st != cudaSuccess) goto fail_14;
    st = cudaMemcpy(out_sign_data_hashes, d_sign, batch_bytes, cudaMemcpyDeviceToHost);
    if (st != cudaSuccess) goto fail_15;

    cudaFree(d_header); cudaFree(d_extra); cudaFree(d_key); cudaFree(d_sign);
    return 0;

fail_10:
    cudaFree(d_header); cudaFree(d_extra); cudaFree(d_key); cudaFree(d_sign);
    return fail_cuda("prepare_batch", 10, st);
fail_11:
    cudaFree(d_header); cudaFree(d_extra); cudaFree(d_key); cudaFree(d_sign);
    return fail_cuda("prepare_batch", 11, st);
fail_12:
    cudaFree(d_header); cudaFree(d_extra); cudaFree(d_key); cudaFree(d_sign);
    return fail_cuda("prepare_batch", 12, st);
fail_13:
    cudaFree(d_header); cudaFree(d_extra); cudaFree(d_key); cudaFree(d_sign);
    return fail_cuda("prepare_batch", 13, st);
fail_14:
    cudaFree(d_header); cudaFree(d_extra); cudaFree(d_key); cudaFree(d_sign);
    return fail_cuda("prepare_batch", 14, st);
fail_15:
    cudaFree(d_header); cudaFree(d_extra); cudaFree(d_key); cudaFree(d_sign);
    return fail_cuda("prepare_batch", 15, st);
}

extern "C" int gpuminer_scan_nonces_cuda_full(const uint8_t* header_hash,
                                              const uint8_t* extra_nonce,
                                              uint64_t start_nonce,
                                              uint64_t count,
                                              const uint8_t* target,
                                              uint64_t* found_nonce,
                                              int* found) {
    if (!header_hash || !extra_nonce || !target || !found_nonce || !found) return -1;
    *found = 0;
    *found_nonce = 0;
    if (count == 0) return 0;

    constexpr int threads = 64;
    const int blocks = static_cast<int>((count + threads - 1) / threads);

    uint8_t *d_header = nullptr, *d_extra = nullptr, *d_target = nullptr;
    uint64_t *d_found_nonce = nullptr;
    int *d_found = nullptr;

    cudaError_t st_alloc = cudaMalloc(&d_header, 32);
    if (st_alloc != cudaSuccess) return fail_cuda("scan_full", 1, st_alloc);
    st_alloc = cudaMalloc(&d_extra, 8);
    if (st_alloc != cudaSuccess) { cudaFree(d_header); return fail_cuda("scan_full", 2, st_alloc); }
    st_alloc = cudaMalloc(&d_target, 32);
    if (st_alloc != cudaSuccess) { cudaFree(d_header); cudaFree(d_extra); return fail_cuda("scan_full", 3, st_alloc); }
    st_alloc = cudaMalloc(&d_found_nonce, sizeof(uint64_t));
    if (st_alloc != cudaSuccess) {
        cudaFree(d_header); cudaFree(d_extra); cudaFree(d_target); return fail_cuda("scan_full", 4, st_alloc);
    }
    st_alloc = cudaMalloc(&d_found, sizeof(int));
    if (st_alloc != cudaSuccess) {
        cudaFree(d_header); cudaFree(d_extra); cudaFree(d_target); cudaFree(d_found_nonce); return fail_cuda("scan_full", 5, st_alloc);
    }

    cudaError_t st = cudaMemcpy(d_header, header_hash, 32, cudaMemcpyHostToDevice);
    if (st != cudaSuccess) goto fail_10;
    st = cudaMemcpy(d_extra, extra_nonce, 8, cudaMemcpyHostToDevice);
    if (st != cudaSuccess) goto fail_11;
    st = cudaMemcpy(d_target, target, 32, cudaMemcpyHostToDevice);
    if (st != cudaSuccess) goto fail_12;
    st = cudaMemset(d_found, 0, sizeof(int));
    if (st != cudaSuccess) goto fail_13;
    st = cudaMemset(d_found_nonce, 0, sizeof(uint64_t));
    if (st != cudaSuccess) goto fail_14;

    gpuminer_full_scan_kernel<<<blocks, threads>>>(d_header, d_extra, start_nonce, count, d_target, d_found_nonce, d_found);
    st = cudaGetLastError(); if (st != cudaSuccess) goto fail_15;
    st = cudaDeviceSynchronize(); if (st != cudaSuccess) goto fail_16;

    st = cudaMemcpy(found, d_found, sizeof(int), cudaMemcpyDeviceToHost);
    if (st != cudaSuccess) goto fail_17;
    if (*found == 1) {
        st = cudaMemcpy(found_nonce, d_found_nonce, sizeof(uint64_t), cudaMemcpyDeviceToHost);
        if (st != cudaSuccess) goto fail_18;
    }

    cudaFree(d_header); cudaFree(d_extra); cudaFree(d_target); cudaFree(d_found_nonce); cudaFree(d_found);
    return 0;

fail_10:
    cudaFree(d_header); cudaFree(d_extra); cudaFree(d_target); cudaFree(d_found_nonce); cudaFree(d_found);
    return fail_cuda("scan_full", 10, st);
fail_11:
    cudaFree(d_header); cudaFree(d_extra); cudaFree(d_target); cudaFree(d_found_nonce); cudaFree(d_found);
    return fail_cuda("scan_full", 11, st);
fail_12:
    cudaFree(d_header); cudaFree(d_extra); cudaFree(d_target); cudaFree(d_found_nonce); cudaFree(d_found);
    return fail_cuda("scan_full", 12, st);
fail_13:
    cudaFree(d_header); cudaFree(d_extra); cudaFree(d_target); cudaFree(d_found_nonce); cudaFree(d_found);
    return fail_cuda("scan_full", 13, st);
fail_14:
    cudaFree(d_header); cudaFree(d_extra); cudaFree(d_target); cudaFree(d_found_nonce); cudaFree(d_found);
    return fail_cuda("scan_full", 14, st);
fail_15:
    cudaFree(d_header); cudaFree(d_extra); cudaFree(d_target); cudaFree(d_found_nonce); cudaFree(d_found);
    return fail_cuda("scan_full", 15, st);
fail_16:
    cudaFree(d_header); cudaFree(d_extra); cudaFree(d_target); cudaFree(d_found_nonce); cudaFree(d_found);
    return fail_cuda("scan_full", 16, st);
fail_17:
    cudaFree(d_header); cudaFree(d_extra); cudaFree(d_target); cudaFree(d_found_nonce); cudaFree(d_found);
    return fail_cuda("scan_full", 17, st);
fail_18:
    cudaFree(d_header); cudaFree(d_extra); cudaFree(d_target); cudaFree(d_found_nonce); cudaFree(d_found);
    return fail_cuda("scan_full", 18, st);
}
