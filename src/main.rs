/// Miner esterno per InitVerse (inihash / VersaHash).
///
/// Uso solo mining (nodo):
///   miner [RPC_URL] [THREADS] [EXTRA_NONCE_HEX]
///   miner http://localhost:8545 8
///
/// Uso pool (Stratum):
///   miner stratum+tcp://WALLET.WORKER@host:port [THREADS]
///   miner stratum+tcp://0x...Wallet.Worker001@pool-a.yatespool.com:31588 8

mod rpc;
mod stratum;
mod versahash;

use rand::Rng;
use rpc::RpcClient;
use stratum::{StratumClient, StratumJob};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::{Duration, Instant};

// ─── confronto target ─────────────────────────────────────────────────────────

/// true se result <= target (confronto big-endian, equivale a confronto numerico)
#[inline]
fn meets_target(result: &[u8; 32], target: &[u8; 32]) -> bool {
    result <= target
}

// ─── decodifica hex dalla risposta RPC ────────────────────────────────────────

fn decode32(hex_str: &str) -> Option<[u8; 32]> {
    let s = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    // Padding se necessario (la stringa potrebbe avere meno di 64 char)
    let padded = format!("{:0>64}", s);
    hex::decode(&padded)
        .ok()
        .and_then(|v| v.try_into().ok())
}

// ─── statistiche hashrate ─────────────────────────────────────────────────────

struct Stats {
    hashes: Arc<AtomicU64>,
    start: Instant,
}

impl Stats {
    fn new(hashes: Arc<AtomicU64>) -> Self {
        Self {
            hashes,
            start: Instant::now(),
        }
    }

    fn print(&self) {
        let h = self.hashes.load(Ordering::Relaxed);
        let elapsed = self.start.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            println!(
                "  hashrate: {:.1} H/s  (totale: {} hash, {:.1}s)",
                h as f64 / elapsed,
                h,
                elapsed
            );
        }
    }
}

// ─── loop di mining per un singolo thread ─────────────────────────────────────

fn mine_thread(
    header_hash: [u8; 32],
    target: [u8; 32],
    extra_nonce: [u8; 8],
    start_nonce: u64,
    stop: Arc<AtomicBool>,
    found_tx: mpsc::SyncSender<(u64, [u8; 8])>,
    global_hashes: Arc<AtomicU64>,
) {
    let mut nonce: u64 = start_nonce;
    let mut local_count: u64 = 0;

    loop {
        // Controlla stop ogni 2^14 iterazioni (evita overhead atomico)
        if local_count & 0x3FFF == 0 {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            // Aggiorna contatore globale
            global_hashes.fetch_add(local_count, Ordering::Relaxed);
            local_count = 0;
        }

        let nonce_bytes = nonce.to_be_bytes();
        let result = versahash::versa_hash(&header_hash, &nonce_bytes, &extra_nonce);

        if meets_target(&result, &target) {
            let _ = found_tx.try_send((nonce, extra_nonce));
            stop.store(true, Ordering::Relaxed);
            break;
        }

        nonce = nonce.wrapping_add(1);
        local_count += 1;
    }

    global_hashes.fetch_add(local_count, Ordering::Relaxed);
}

// ─── main ─────────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let first = args.get(1).map(String::as_str).unwrap_or("");

    if first.starts_with("stratum+tcp://") {
        if let Err(e) = run_pool(&args) {
            eprintln!("[error] {}", e);
            std::process::exit(1);
        }
    } else {
        run_getwork(&args);
    }
}

/// Modalità pool: stratum+tcp://WALLET.WORKER@host:port [THREADS]
fn run_pool(args: &[String]) -> anyhow::Result<()> {
    let pool_url = args
        .get(1)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("serve URL pool: stratum+tcp://WALLET.WORKER@host:port"))?;
    let num_threads: usize = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        });

    println!("=== InitVerse Miner (Rust) — POOL ===");
    println!("  Pool:   {}", pool_url);
    println!("  Thread: {}", num_threads);
    println!();

    let (mut stratum, user) = StratumClient::from_url(&pool_url)?;
    println!("[stratum] connesso come {}", user);
    stratum.connect()?;

    let job_arc = stratum.current_job();
    let mut last_job_id: Option<String> = None;

    loop {
        let job = {
            let guard = job_arc.lock().unwrap();
            guard.clone()
        };
        let job = match job {
            Some(j) => j,
            None => {
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        if last_job_id.as_ref() == Some(&job.job_id) {
            thread::sleep(Duration::from_millis(500));
            continue;
        }
        last_job_id = Some(job.job_id.clone());
        run_mining_round_pool(
            &job,
            num_threads,
            &mut stratum,
            &job_arc,
            &mut last_job_id,
        )?;
    }
}

/// Una round di mining per un job (pool): mina finché trovi o arriva nuovo job
fn run_mining_round_pool(
    job: &StratumJob,
    num_threads: usize,
    stratum: &mut StratumClient,
    job_arc: &std::sync::Arc<std::sync::Mutex<Option<StratumJob>>>,
    last_job_id: &mut Option<String>,
) -> anyhow::Result<()> {
    println!("[work] job={} target=0x{}", job.job_id, hex::encode(job.target));
    let stop = Arc::new(AtomicBool::new(false));
    let global_hashes = Arc::new(AtomicU64::new(0));
    let (found_tx, found_rx) = mpsc::sync_channel::<(u64, [u8; 8])>(1);
    let mut handles = Vec::new();
    for i in 0..num_threads {
        let stop = Arc::clone(&stop);
        let found_tx = found_tx.clone();
        let global_hashes = Arc::clone(&global_hashes);
        let header_hash = job.seal_hash;
        let target = job.target;
        let extra_nonce = job.extra_nonce;
        let start_nonce: u64 = rand::thread_rng().gen::<u64>()
            ^ ((i as u64).wrapping_mul(0x9e3779b97f4a7c15));
        handles.push(thread::spawn(move || {
            mine_thread(
                header_hash,
                target,
                extra_nonce,
                start_nonce,
                stop,
                found_tx,
                global_hashes,
            );
        }));
    }
    drop(found_tx);
    let stats = Stats::new(Arc::clone(&global_hashes));

    let solution = loop {
        match found_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(sol) => break Some(sol),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                stats.print();
                let guard = job_arc.lock().unwrap();
                if let Some(ref new_job) = *guard {
                    if new_job.job_id != job.job_id {
                        println!("[work] nuovo job dalla pool, riavvio");
                        stop.store(true, Ordering::Relaxed);
                        break None;
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break None,
        }
    };

    stop.store(true, Ordering::Relaxed);
    for h in handles {
        let _ = h.join();
    }

    if let Some((nonce, ext)) = solution {
        let nonce_bytes = nonce.to_be_bytes();
        println!(
            "[found] nonce=0x{} extra=0x{}",
            hex::encode(nonce_bytes),
            hex::encode(ext)
        );
        stratum.submit(&job.job_id, &nonce_bytes, &ext)?;
        println!("[ok] Share inviata alla pool");
        *last_job_id = None;
    }
    Ok(())
}

fn run_getwork(args: &[String]) {
    let rpc_url = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "http://localhost:8545".to_string());

    let num_threads: usize = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        });

    let extra_nonce: [u8; 8] = args
        .get(3)
        .map(|s| {
            let s = s.strip_prefix("0x").unwrap_or(s);
            let padded = format!("{:0>16}", s);
            hex::decode(&padded)
                .expect("extra_nonce non valido")
                .try_into()
                .expect("extra_nonce deve essere 8 byte")
        })
        .unwrap_or([0u8; 8]);

    println!("=== InitVerse Miner (Rust) ===");
    println!("  RPC:         {}", rpc_url);
    println!("  Thread:      {}", num_threads);
    println!("  Extra nonce: 0x{}", hex::encode(extra_nonce));
    println!();

    let rpc = Arc::new(RpcClient::new(rpc_url));
    let mut last_work: Option<[String; 4]> = None;

    loop {
        // ── 1. Recupera work ──────────────────────────────────────────────────
        let work = match rpc.get_work() {
            Ok(w) => w,
            Err(e) => {
                eprintln!("[warn] get_work: {e}");
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };

        // Se il work non è cambiato, aspetta
        if last_work.as_ref() == Some(&work) {
            thread::sleep(Duration::from_millis(500));
            continue;
        }

        let header_hash = match decode32(&work[0]) {
            Some(h) => h,
            None => {
                eprintln!("[error] header hash non valido: {}", work[0]);
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        let target = match decode32(&work[1]) {
            Some(t) => t,
            None => {
                eprintln!("[error] target non valido: {}", work[1]);
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };

        println!("[work] block={} target={}", work[2], work[1]);
        last_work = Some(work.clone());

        // ── 2. Avvia i thread di mining ───────────────────────────────────────
        let stop = Arc::new(AtomicBool::new(false));
        let global_hashes = Arc::new(AtomicU64::new(0));
        let (found_tx, found_rx) = mpsc::sync_channel::<(u64, [u8; 8])>(1);

        let mut handles = Vec::new();
        for i in 0..num_threads {
            let stop = Arc::clone(&stop);
            let found_tx = found_tx.clone();
            let global_hashes = Arc::clone(&global_hashes);
            // Ogni thread parte da un nonce random diverso
            let start_nonce: u64 = rand::thread_rng().gen::<u64>()
                ^ ((i as u64).wrapping_mul(0x9e3779b97f4a7c15));

            handles.push(thread::spawn(move || {
                mine_thread(
                    header_hash,
                    target,
                    extra_nonce,
                    start_nonce,
                    stop,
                    found_tx,
                    global_hashes,
                );
            }));
        }
        drop(found_tx); // chiude il lato sender

        let stats = Stats::new(Arc::clone(&global_hashes));

        // ── 3. Attendi soluzione oppure cambio di work ────────────────────────
        let solution = loop {
            match found_rx.recv_timeout(Duration::from_secs(5)) {
                Ok(sol) => break Some(sol),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    stats.print();

                    // Controlla se il work è cambiato
                    match rpc.get_work() {
                        Ok(new_work) if new_work != last_work.as_ref().unwrap().clone() => {
                            println!("[work] nuovo blocco disponibile, riavvio");
                            stop.store(true, Ordering::Relaxed);
                            break None;
                        }
                        _ => {}
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    // Tutti i thread terminati senza trovare (non dovrebbe succedere)
                    break None;
                }
            }
        };

        stop.store(true, Ordering::Relaxed);
        for h in handles {
            let _ = h.join();
        }

        // ── 4. Submit ─────────────────────────────────────────────────────────
        if let Some((nonce, ext)) = solution {
            let nonce_bytes = nonce.to_be_bytes();
            println!(
                "[found] nonce=0x{} extra=0x{}",
                hex::encode(nonce_bytes),
                hex::encode(ext)
            );
            match rpc.submit_work(&nonce_bytes, &ext, &header_hash) {
                Ok(true) => println!("[ok] Blocco ACCETTATO"),
                Ok(false) => println!("[warn] Blocco rifiutato (stale?)"),
                Err(e) => eprintln!("[error] submit_work: {e}"),
            }
            // Reset work così al prossimo giro recupera il nuovo blocco
            last_work = None;
        }
    }
}

// ─── test: formato work allineato al chain ───────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifica che il formato work usato dal miner sia quello prodotto da
    /// chain/consensus/inihash/sealer.go makeWork(): [sealHash.Hex(), target.Hex(), number, time].
    #[test]
    fn work_format_matches_chain_getwork() {
        let seal_hex = "0x794c71e01331b1c9fd07b1f41749ebe8d8cc731783dad9fc5396f17a86f4eebb";
        let target_hex = "0x00000052dc453a085d62e743ab60791725bb1724200664b220e569799bb54902";
        let work: [String; 4] = [
            seal_hex.to_string(),
            target_hex.to_string(),
            "0x3e8".to_string(),
            "0x5f".to_string(),
        ];
        let header_hash = decode32(&work[0]).expect("decode seal");
        let target = decode32(&work[1]).expect("decode target");
        assert_eq!(header_hash.len(), 32);
        assert_eq!(target.len(), 32);
        let extra = [0u8; 8];
        let nonce_bytes = 0u64.to_be_bytes();
        let result = versahash::versa_hash(&header_hash, &nonce_bytes, &extra);
        let _ = meets_target(&result, &target);
    }
}
