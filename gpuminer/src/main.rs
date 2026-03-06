mod cuda;
#[path = "../../src/rpc.rs"]
mod rpc;
#[path = "../../src/stratum.rs"]
mod stratum;
use rand::Rng;
use rpc::RpcClient;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use stratum::{StratumClient, StratumJob};
#[path = "../../src/versahash.rs"]
mod versahash;

use anyhow::{anyhow, Result};

const DEFAULT_BATCH_SIZE: u64 = 262_144;

fn decode32(hex_str: &str) -> Option<[u8; 32]> {
    let s = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let padded = format!("{:0>64}", s);
    hex::decode(&padded)
        .ok()
        .and_then(|v| v.try_into().ok())
}

fn decode8(hex_str: &str) -> Result<[u8; 8]> {
    let s = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let padded = format!("{:0>16}", s);
    let data = hex::decode(&padded).map_err(|e| anyhow!("decode8: {}", e))?;
    data.try_into().map_err(|_| anyhow!("decode8: attesi 8 byte"))
}

fn parse_u64(arg: &str) -> Result<u64> {
    if let Some(hex) = arg.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).map_err(|e| anyhow!("u64 hex non valido: {}", e))
    } else {
        arg.parse::<u64>()
            .map_err(|e| anyhow!("u64 decimale non valido: {}", e))
    }
}

fn print_usage() {
    println!("=== gpuminer (CUDA pipeline) ===");
    println!("Uso:");
    println!("  gpuminer scan <seal_hash_hex> <extra_nonce_hex> <target_hex> [start_nonce] [count]");
    println!("  gpuminer [RPC_URL] [BATCH_SIZE] [EXTRA_NONCE_HEX]");
    println!("  gpuminer stratum+tcp://WALLET.WORKER@host:port [BATCH_SIZE]");
    println!();
    println!("Esempio:");
    println!("  gpuminer scan 0x... 0x0000000000000000 0x00000052... 0x0 1000000");
    println!("  gpuminer http://localhost:8545 262144 0x0000000000000000");
    println!("  gpuminer stratum+tcp://0x...Wallet.Worker001@pool:31588 262144");
    println!();
    println!("Nota: se il backend CUDA non è disponibile, viene usato automaticamente il fallback CPU.");
}

struct Stats {
    hashes: u64,
    start: Instant,
    last_print: Instant,
}

impl Stats {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            hashes: 0,
            start: now,
            last_print: now,
        }
    }

    fn add_batch(&mut self, batch: u64) {
        self.hashes = self.hashes.saturating_add(batch);
    }

    fn print_maybe(&mut self) {
        if self.last_print.elapsed() < Duration::from_secs(5) {
            return;
        }
        let elapsed = self.start.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            println!(
                "  hashrate: {:.1} H/s  (totale: {} hash, {:.1}s)",
                self.hashes as f64 / elapsed,
                self.hashes,
                elapsed
            );
        }
        self.last_print = Instant::now();
    }
}

fn run_scan(args: &[String]) -> Result<()> {
    if args.len() < 5 {
        return Err(anyhow!(
            "scan richiede: <seal_hash_hex> <extra_nonce_hex> <target_hex> [start_nonce] [count]"
        ));
    }

    let seal_hash = decode32(&args[2]).ok_or_else(|| anyhow!("seal_hash non valido"))?;
    let extra_nonce = decode8(&args[3])?;
    let target = decode32(&args[4]).ok_or_else(|| anyhow!("target non valido"))?;
    let start_nonce = if let Some(s) = args.get(5) {
        parse_u64(s)?
    } else {
        0
    };
    let count = if let Some(s) = args.get(6) {
        parse_u64(s)?
    } else {
        1_000_000
    };

    println!(
        "[gpuminer] scan start_nonce=0x{:016x}, count={}, target=0x{}",
        start_nonce,
        count,
        hex::encode(target)
    );
    let found = cuda::scan_nonces(&seal_hash, &extra_nonce, start_nonce, count, &target);
    match found {
        Some(nonce) => println!("[gpuminer] nonce trovato: 0x{:016x}", nonce),
        None => println!("[gpuminer] nessun nonce valido nel range richiesto"),
    }
    Ok(())
}

fn run_getwork(args: &[String]) {
    let rpc_url = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "http://localhost:8545".to_string());
    let batch_size = args
        .get(2)
        .and_then(|s| parse_u64(s).ok())
        .unwrap_or(DEFAULT_BATCH_SIZE);
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

    println!("=== gpuminer (RPC) ===");
    println!("  RPC:         {}", rpc_url);
    println!("  Batch size:  {}", batch_size);
    println!("  Extra nonce: 0x{}", hex::encode(extra_nonce));
    println!();

    let rpc = Arc::new(RpcClient::new(rpc_url));
    let mut last_work: Option<[String; 4]> = None;

    loop {
        let work = match rpc.get_work() {
            Ok(w) => w,
            Err(e) => {
                eprintln!("[warn] get_work: {e}");
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
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

        let mut nonce: u64 = rand::thread_rng().r#gen();
        let mut stats = Stats::new();
        let mut solution: Option<u64> = None;

        loop {
            if let Some(found) = cuda::scan_nonces(&header_hash, &extra_nonce, nonce, batch_size, &target)
            {
                solution = Some(found);
                break;
            }
            nonce = nonce.wrapping_add(batch_size);
            stats.add_batch(batch_size);
            stats.print_maybe();

            match rpc.get_work() {
                Ok(new_work) if new_work != last_work.as_ref().expect("work presente").clone() => {
                    println!("[work] nuovo blocco disponibile, riavvio");
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("[warn] get_work check: {e}");
                    thread::sleep(Duration::from_millis(200));
                }
            }
        }

        if let Some(found_nonce) = solution {
            let nonce_bytes = found_nonce.to_be_bytes();
            println!(
                "[found] nonce=0x{} extra=0x{}",
                hex::encode(nonce_bytes),
                hex::encode(extra_nonce)
            );
            match rpc.submit_work(&nonce_bytes, &extra_nonce, &header_hash) {
                Ok(true) => println!("[ok] Blocco ACCETTATO"),
                Ok(false) => println!("[warn] Blocco rifiutato (stale?)"),
                Err(e) => eprintln!("[error] submit_work: {e}"),
            }
            last_work = None;
        }
    }
}

fn run_mining_round_pool(
    job: &StratumJob,
    batch_size: u64,
    stratum: &mut StratumClient,
    job_arc: &std::sync::Arc<std::sync::Mutex<Option<StratumJob>>>,
    last_job_id: &mut Option<String>,
) -> anyhow::Result<()> {
    println!("[work] job={} target=0x{}", job.job_id, hex::encode(job.target));
    let mut nonce: u64 = rand::thread_rng().r#gen();
    let mut stats = Stats::new();
    let solution = loop {
        {
            let guard = job_arc.lock().unwrap();
            if let Some(ref new_job) = *guard {
                if new_job.job_id != job.job_id {
                    println!("[work] nuovo job dalla pool, riavvio");
                    break None;
                }
            }
        }

        if let Some(found) =
            cuda::scan_nonces(&job.seal_hash, &job.extra_nonce, nonce, batch_size, &job.target)
        {
            break Some(found);
        }
        nonce = nonce.wrapping_add(batch_size);
        stats.add_batch(batch_size);
        stats.print_maybe();
    };

    if let Some(found_nonce) = solution {
        let nonce_bytes = found_nonce.to_be_bytes();
        println!(
            "[found] nonce=0x{} extra=0x{}",
            hex::encode(nonce_bytes),
            hex::encode(job.extra_nonce)
        );
        stratum.submit(&job.job_id, &nonce_bytes, &job.extra_nonce)?;
        println!("[ok] Share inviata alla pool");
        *last_job_id = None;
    }
    Ok(())
}

fn run_pool(args: &[String]) -> anyhow::Result<()> {
    let pool_url = args
        .get(1)
        .cloned()
        .ok_or_else(|| anyhow!("serve URL pool: stratum+tcp://WALLET.WORKER@host:port"))?;
    let batch_size = args
        .get(2)
        .and_then(|s| parse_u64(s).ok())
        .unwrap_or(DEFAULT_BATCH_SIZE);

    println!("=== gpuminer (POOL) ===");
    println!("  Pool:       {}", pool_url);
    println!("  Batch size: {}", batch_size);
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
        run_mining_round_pool(&job, batch_size, &mut stratum, &job_arc, &mut last_job_id)?;
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let first = args.get(1).map(String::as_str).unwrap_or("");
    if first == "scan" {
        return run_scan(&args);
    }
    if first.starts_with("stratum+tcp://") {
        run_pool(&args)
    } else if first.is_empty() || first.starts_with("http://") || first.starts_with("https://") {
        run_getwork(&args);
        Ok(())
    } else {
        print_usage();
        Ok(())
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("[gpuminer][error] {}", e);
        std::process::exit(1);
    }
}
