#include <cstdint>

// Stub kernel: punto di ingresso CUDA per futura porting completa di VersaHash.
// La validazione chain-compatibile resta demandata al fallback CPU finché
// l'implementazione completa del pipeline cryptografico non viene portata su GPU.
__global__ void versahash_scan_kernel_stub() {}

extern "C" int gpuminer_scan_nonces_cuda(const uint8_t* header_hash,
                                         const uint8_t* extra_nonce,
                                         uint64_t start_nonce,
                                         uint64_t count,
                                         const uint8_t* target,
                                         uint64_t* found_nonce,
                                         int* found) {
    (void)header_hash;
    (void)extra_nonce;
    (void)start_nonce;
    (void)count;
    (void)target;
    (void)found_nonce;
    (void)found;
    // -2: backend presente ma non ancora implementato per valida share chain-compatible.
    return -2;
}
