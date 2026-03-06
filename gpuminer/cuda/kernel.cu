#include <cuda_runtime.h>
#include <cstdint>

namespace {

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

__constant__ uint32_t K[64] = {
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

__device__ void sha256_one_block(const uint8_t* data, uint32_t len, uint8_t out[32]) {
    uint8_t block[64];
    #pragma unroll
    for (int i = 0; i < 64; i++) {
        block[i] = 0;
    }

    for (uint32_t i = 0; i < len; i++) {
        block[i] = data[i];
    }
    block[len] = 0x80;
    const uint64_t bit_len = static_cast<uint64_t>(len) * 8ULL;
    block[63] = static_cast<uint8_t>(bit_len & 0xff);
    block[62] = static_cast<uint8_t>((bit_len >> 8) & 0xff);
    block[61] = static_cast<uint8_t>((bit_len >> 16) & 0xff);
    block[60] = static_cast<uint8_t>((bit_len >> 24) & 0xff);
    block[59] = static_cast<uint8_t>((bit_len >> 32) & 0xff);
    block[58] = static_cast<uint8_t>((bit_len >> 40) & 0xff);
    block[57] = static_cast<uint8_t>((bit_len >> 48) & 0xff);
    block[56] = static_cast<uint8_t>((bit_len >> 56) & 0xff);

    uint32_t w[64];
    #pragma unroll
    for (int i = 0; i < 16; i++) {
        const int j = i * 4;
        w[i] = (static_cast<uint32_t>(block[j]) << 24) |
               (static_cast<uint32_t>(block[j + 1]) << 16) |
               (static_cast<uint32_t>(block[j + 2]) << 8) |
               (static_cast<uint32_t>(block[j + 3]));
    }
    #pragma unroll
    for (int i = 16; i < 64; i++) {
        w[i] = sig1(w[i - 2]) + w[i - 7] + sig0(w[i - 15]) + w[i - 16];
    }

    uint32_t a = 0x6a09e667u;
    uint32_t b = 0xbb67ae85u;
    uint32_t c = 0x3c6ef372u;
    uint32_t d = 0xa54ff53au;
    uint32_t e = 0x510e527fu;
    uint32_t f = 0x9b05688cu;
    uint32_t g = 0x1f83d9abu;
    uint32_t h = 0x5be0cd19u;

    #pragma unroll
    for (int i = 0; i < 64; i++) {
        const uint32_t t1 = h + ep1(e) + ch(e, f, g) + K[i] + w[i];
        const uint32_t t2 = ep0(a) + maj(a, b, c);
        h = g;
        g = f;
        f = e;
        e = d + t1;
        d = c;
        c = b;
        b = a;
        a = t1 + t2;
    }

    a += 0x6a09e667u;
    b += 0xbb67ae85u;
    c += 0x3c6ef372u;
    d += 0xa54ff53au;
    e += 0x510e527fu;
    f += 0x9b05688cu;
    g += 0x1f83d9abu;
    h += 0x5be0cd19u;

    const uint32_t H[8] = {a, b, c, d, e, f, g, h};
    #pragma unroll
    for (int i = 0; i < 8; i++) {
        out[i * 4 + 0] = static_cast<uint8_t>((H[i] >> 24) & 0xff);
        out[i * 4 + 1] = static_cast<uint8_t>((H[i] >> 16) & 0xff);
        out[i * 4 + 2] = static_cast<uint8_t>((H[i] >> 8) & 0xff);
        out[i * 4 + 3] = static_cast<uint8_t>(H[i] & 0xff);
    }
}

__global__ void gpuminer_prepare_batch_kernel(const uint8_t* header_hash,
                                              const uint8_t* extra_nonce,
                                              uint64_t start_nonce,
                                              uint64_t count,
                                              uint8_t* out_key_hashes,
                                              uint8_t* out_sign_data_hashes) {
    const uint64_t idx = static_cast<uint64_t>(blockIdx.x) * blockDim.x + threadIdx.x;
    if (idx >= count) {
        return;
    }

    const uint64_t nonce = start_nonce + idx;
    uint8_t msg[49];
    #pragma unroll
    for (int i = 0; i < 32; i++) {
        msg[i] = header_hash[i];
    }
    msg[32] = 16;  // len(nonce)+len(extra_nonce)
    msg[33] = static_cast<uint8_t>((nonce >> 56) & 0xff);
    msg[34] = static_cast<uint8_t>((nonce >> 48) & 0xff);
    msg[35] = static_cast<uint8_t>((nonce >> 40) & 0xff);
    msg[36] = static_cast<uint8_t>((nonce >> 32) & 0xff);
    msg[37] = static_cast<uint8_t>((nonce >> 24) & 0xff);
    msg[38] = static_cast<uint8_t>((nonce >> 16) & 0xff);
    msg[39] = static_cast<uint8_t>((nonce >> 8) & 0xff);
    msg[40] = static_cast<uint8_t>(nonce & 0xff);
    #pragma unroll
    for (int i = 0; i < 8; i++) {
        msg[41 + i] = extra_nonce[i];
    }

    uint8_t first_hash[32];
    uint8_t key_hash[32];
    uint8_t sign_data_hash[32];
    sha256_one_block(msg, 49, first_hash);
    sha256_one_block(first_hash, 32, key_hash);
    sha256_one_block(key_hash, 32, sign_data_hash);

    const uint64_t offset = idx * 32ULL;
    for (int i = 0; i < 32; i++) {
        out_key_hashes[offset + i] = key_hash[i];
        out_sign_data_hashes[offset + i] = sign_data_hash[i];
    }
}

}  // namespace

extern "C" int gpuminer_prepare_batch_cuda(const uint8_t* header_hash,
                                           const uint8_t* extra_nonce,
                                           uint64_t start_nonce,
                                           uint64_t count,
                                           uint8_t* out_key_hashes,
                                           uint8_t* out_sign_data_hashes) {
    if (!header_hash || !extra_nonce || !out_key_hashes || !out_sign_data_hashes) {
        return -1;
    }
    if (count == 0) {
        return 0;
    }

    uint8_t* d_header = nullptr;
    uint8_t* d_extra = nullptr;
    uint8_t* d_key = nullptr;
    uint8_t* d_sign = nullptr;

    const size_t batch_bytes = static_cast<size_t>(count) * 32ULL;

    if (cudaMalloc(&d_header, 32) != cudaSuccess) return -10;
    if (cudaMalloc(&d_extra, 8) != cudaSuccess) {
        cudaFree(d_header);
        return -11;
    }
    if (cudaMalloc(&d_key, batch_bytes) != cudaSuccess) {
        cudaFree(d_header);
        cudaFree(d_extra);
        return -12;
    }
    if (cudaMalloc(&d_sign, batch_bytes) != cudaSuccess) {
        cudaFree(d_header);
        cudaFree(d_extra);
        cudaFree(d_key);
        return -13;
    }

    cudaError_t st = cudaMemcpy(d_header, header_hash, 32, cudaMemcpyHostToDevice);
    if (st != cudaSuccess) goto fail;
    st = cudaMemcpy(d_extra, extra_nonce, 8, cudaMemcpyHostToDevice);
    if (st != cudaSuccess) goto fail;

    constexpr int threads = 256;
    const int blocks = static_cast<int>((count + threads - 1) / threads);
    gpuminer_prepare_batch_kernel<<<blocks, threads>>>(
        d_header, d_extra, start_nonce, count, d_key, d_sign);

    st = cudaGetLastError();
    if (st != cudaSuccess) goto fail;
    st = cudaDeviceSynchronize();
    if (st != cudaSuccess) goto fail;

    st = cudaMemcpy(out_key_hashes, d_key, batch_bytes, cudaMemcpyDeviceToHost);
    if (st != cudaSuccess) goto fail;
    st = cudaMemcpy(out_sign_data_hashes, d_sign, batch_bytes, cudaMemcpyDeviceToHost);
    if (st != cudaSuccess) goto fail;

    cudaFree(d_header);
    cudaFree(d_extra);
    cudaFree(d_key);
    cudaFree(d_sign);
    return 0;

fail:
    cudaFree(d_header);
    cudaFree(d_extra);
    cudaFree(d_key);
    cudaFree(d_sign);
    return -20;
}
