# InitVerse Miner (Rust)

Miner esterno ad alte prestazioni per InitVerse (inihash / VersaHash) scritto in Rust.

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
./build-linux.sh
```

Il binario sarà disponibile in `target/release/miner`.

### Build manuale

```bash
cargo build --release
```

## Utilizzo

### Mining diretto (RPC)

Mining diretto tramite nodo InitVerse:

```bash
miner [RPC_URL] [THREADS] [EXTRA_NONCE_HEX]
```

**Esempi:**

```bash
# Usa il numero di thread disponibili
miner http://localhost:8545

# Specifica il numero di thread
miner http://localhost:8545 8

# Con extra nonce personalizzato
miner http://localhost:8545 8 0x1234567890abcdef
```

### Mining pool (Stratum)

Mining tramite pool Stratum:

```bash
miner stratum+tcp://WALLET.WORKER@host:port [THREADS]
```

**Esempi:**

```bash
# Mining pool con thread automatici
miner stratum+tcp://0x...Wallet.Worker001@pool-a.yatespool.com:31588

# Con numero di thread specificato
miner stratum+tcp://0x...Wallet.Worker001@pool-a.yatespool.com:31588 8
```

## Architettura

Il progetto è organizzato in moduli:

- **`main.rs`**: Entry point e logica principale del miner
- **`rpc.rs`**: Client JSON-RPC per comunicazione con il nodo InitVerse
- **`stratum.rs`**: Client Stratum per mining pool
- **`versahash.rs`**: Implementazione dell'algoritmo VersaHash

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
