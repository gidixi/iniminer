# InitVerse Miners (Rust)

Repository con due progetti separati per InitVerse (inihash / VersaHash):

- **cpuminer**: miner CPU chain-compatible.
- **gpuminer**: progetto GPU con backend CUDA (con fallback CPU per correttezza).

## Caratteristiche

- ⚡ **Alte prestazioni**: Implementazione ottimizzata in Rust con supporto multi-threading
- 🔗 **Mining diretto**: Supporto per mining diretto tramite RPC JSON
- 🏊 **Pool mining**: Supporto completo per protocollo Stratum
- 🔐 **VersaHash**: Implementazione fedele dell'algoritmo VersaHash del nodo InitVerse
- 📊 **Statistiche**: Monitoraggio in tempo reale dell'hashrate

## Requisiti

- Rust 1.70+ (edition 2021)
- Linux (x86_64) per la build predefinita

## Build

### Build per Linux

```bash
# Build workspace completo (cpuminer + gpuminer)
./build-linux.sh

# Debug
./build-linux.sh debug

# Release solo cpuminer
./build-linux.sh release x86_64-unknown-linux-gnu cpuminer

# Release solo gpuminer
./build-linux.sh release x86_64-unknown-linux-gnu gpuminer

# Release gpuminer con backend CUDA (richiede nvcc)
./build-linux.sh release x86_64-unknown-linux-gnu gpuminer cuda

# Se nvcc rifiuta GCC troppo nuovo:
NVCC_CCBIN=/usr/bin/gcc-13 ./build-linux.sh release x86_64-unknown-linux-gnu gpuminer cuda
```

Note CUDA toolchain:
- di default il build script abilita `-allow-unsupported-compiler` per `nvcc`.
- puoi disattivarlo con `GPUMINER_NVCC_ALLOW_UNSUPPORTED=0`.
- puoi forzare il compilatore host con `NVCC_CCBIN=/path/to/gcc-13`.
- workaround automatico per errori `sinpi/cospi noexcept` (glibc/CUDA): ON di default.
  Disattivalo con `GPUMINER_NVCC_PATCH_MATH_FUNCTIONS=0`.
- workaround aggiuntivo: `-U_GNU_SOURCE` (ON di default) per evitare prototype GNU in conflitto.
  Disattivalo con `GPUMINER_NVCC_UNDEF_GNU_SOURCE=0`.

Il binario sarà disponibile in:
- `target/<target-triple>/release/cpuminer` (release CPU)
- `target/<target-triple>/debug/cpuminer` (debug CPU)
- `target/<target-triple>/release/gpuminer` (release GPU)
- `target/<target-triple>/debug/gpuminer` (debug GPU)

### Build manuale

```bash
cargo build --workspace --release
```

## Utilizzo

### cpuminer — Mining diretto (RPC)

Mining diretto tramite nodo InitVerse:

```bash
cpuminer [RPC_URL] [THREADS] [EXTRA_NONCE_HEX]
```

**Esempi:**

```bash
# Usa il numero di thread disponibili
cpuminer http://localhost:8545

# Specifica il numero di thread
cpuminer http://localhost:8545 8

# Con extra nonce personalizzato
cpuminer http://localhost:8545 8 0x1234567890abcdef
```

### cpuminer — Mining pool (Stratum)

Mining tramite pool Stratum:

```bash
cpuminer stratum+tcp://WALLET.WORKER@host:port [THREADS]
```

**Esempi:**

```bash
# Mining pool con thread automatici
cpuminer stratum+tcp://0x...Wallet.Worker001@pool-a.yatespool.com:31588

# Con numero di thread specificato
cpuminer stratum+tcp://0x...Wallet.Worker001@pool-a.yatespool.com:31588 8
```

### gpuminer — scansione range nonce

```bash
gpuminer scan <seal_hash_hex> <extra_nonce_hex> <target_hex> [start_nonce] [count]
```

### gpuminer — mining diretto (RPC) e pool

```bash
# mining diretto
gpuminer [RPC_URL] [BATCH_SIZE] [EXTRA_NONCE_HEX]

# pool
gpuminer stratum+tcp://WALLET.WORKER@host:port [BATCH_SIZE]
```

Con feature CUDA (`--features cuda`) il gpuminer supporta due modalità:
- `GPUMINER_CUDA_MODE=full` (default): tenta il percorso full-offload su GPU.
- `GPUMINER_CUDA_MODE=hybrid`: usa pipeline ibrida.

Pipeline ibrida:
- **GPU**: calcolo batch di `first_hash`, `key_hash`, `sign_data = SHA256(key_hash)`.
- **CPU**: firma Schnorr chain-compatible + hash finale + confronto target.

Questo mantiene la compatibilità con il chain, evitando divergenze crittografiche.

## Architettura

Il repository è organizzato in due crate:

- **`cpuminer/`**: entrypoint CPU + RPC + Stratum.
- **`gpuminer/`**: entrypoint GPU + interfaccia CUDA (`gpuminer/cuda/kernel.cu`).

### VersaHash

L'algoritmo VersaHash è implementato fedelmente rispetto alla versione Go del nodo InitVerse:

1. **SealHash**: Calcolo del hash dell'header (senza Nonce, ExtraNonce, MixDigest)
2. **Mining**: Iterazione su nonce e extra_nonce
3. **VersaHash**: Algoritmo di hashing che combina SHA256 e firma Schnorr
4. **Verifica**: Confronto del risultato con il target (2^256 / difficulty)

## Protocollo RPC

Il miner utilizza i seguenti metodi RPC:

- `eth_getWork`: Ottiene i parametri di lavoro (seal_hash, target, block_number, block_time)
- `eth_submitWork`: Invia una soluzione trovata (nonce, extra_nonce, seal_hash)

## Protocollo Stratum

Il miner supporta il protocollo Stratum con i seguenti messaggi:

- `mining.subscribe`: Iscrizione al pool
- `mining.notify`: Ricezione di nuovi job
- `mining.submit`: Invio di soluzioni trovate

## Performance

Il miner è ottimizzato per massime prestazioni:

- **LTO (Link Time Optimization)**: Abilitato in release
- **Codegen units**: Impostato a 1 per ottimizzazioni migliori
- **Opt level**: 3 per massime ottimizzazioni
- **Multi-threading**: Supporto nativo per mining parallelo

## CI: binari Linux automatici

La pipeline GitHub Actions (`Linux Build & Test`) ora:
- compila in release `cpuminer` e `gpuminer`,
- esegue i test workspace,
- pubblica gli eseguibili Linux come artifact (`cpuminer-linux-x86_64`, `gpuminer-linux-x86_64`),
- su tag (`v*`) allega i binari direttamente alla release GitHub.

## Dipendenze

- `sha2`, `sha3`: Algoritmi di hashing
- `k256`: Operazioni crittografiche (curva secp256k1)
- `hmac`: Autenticazione HMAC
- `serde`, `serde_json`: Serializzazione JSON
- `reqwest`: Client HTTP per RPC
- `rand`: Generazione numeri casuali
- `anyhow`: Gestione errori
- `subtle`: Operazioni crittografiche costanti nel tempo

## Licenza

[Specificare la licenza del progetto]

## Contributi

I contributi sono benvenuti! Per favore apri una issue o una pull request.

## Riferimenti

- [InitVerse Chain](https://github.com/inichain/chain)
- [VersaHash Algorithm](https://github.com/inichain/chain/tree/main/crypto/versaHash)
