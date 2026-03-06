mod cuda;
#[path = "../../src/versahash.rs"]
mod versahash;

use anyhow::{anyhow, Result};

fn decode32(hex_str: &str) -> Result<[u8; 32]> {
    let s = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let padded = format!("{:0>64}", s);
    let data = hex::decode(&padded).map_err(|e| anyhow!("decode32: {}", e))?;
    data.try_into()
        .map_err(|_| anyhow!("decode32: attesi 32 byte"))
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
    println!("=== gpuminer (CUDA-ready) ===");
    println!("Uso:");
    println!("  gpuminer scan <seal_hash_hex> <extra_nonce_hex> <target_hex> [start_nonce] [count]");
    println!();
    println!("Esempio:");
    println!("  gpuminer scan 0x... 0x0000000000000000 0x00000052... 0x0 1000000");
    println!();
    println!("Nota: se il backend CUDA non è disponibile, viene usato automaticamente il fallback CPU.");
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 || args.get(1).map(String::as_str) != Some("scan") {
        print_usage();
        return Ok(());
    }

    let seal_hash = decode32(&args[2])?;
    let extra_nonce = decode8(&args[3])?;
    let target = decode32(&args[4])?;
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

fn main() {
    if let Err(e) = run() {
        eprintln!("[gpuminer][error] {}", e);
        std::process::exit(1);
    }
}
