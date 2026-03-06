//! Client Stratum per pool mining INIChain (VersaHash).
//!
//! Il repo chain non implementa il server Stratum (le pool sono esterne). I dati di lavoro
//! sono gli stessi del solo mining: seal_hash, target, extra_nonce (chain: SealHash, 2^256/diff, ExtraNonce).
//!
//! URL: stratum+tcp://WALLET.WORKER@host:port
//! Esempio: stratum+tcp://0x...Wallet.Worker001@pool-a.yatespool.com:31588
//!
//! Protocollo atteso (stessa semantica di eth_getWork):
//! - mining.notify params [job_id, seal_hash_hex, target_hex, extranonce_hex]
//! - mining.submit(worker, job_id, nonce_hex, extranonce_hex)
//! Se la pool usa un formato diverso (es. ini-miner), adattare parse_notify_params e submit.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Parametri di lavoro da mining.notify
#[derive(Clone, Debug)]
pub struct StratumJob {
    pub job_id: String,
    pub seal_hash: [u8; 32],
    pub target: [u8; 32],
    pub extra_nonce: [u8; 8],
}

fn strip_0x(s: &str) -> &str {
    s.strip_prefix("0x").unwrap_or(s)
}

fn decode32(hex_str: &str) -> Result<[u8; 32]> {
    let s = strip_0x(hex_str);
    let padded = format!("{:0>64}", s);
    let v: Vec<u8> = hex::decode(&padded).map_err(|e| anyhow!("decode32: {}", e))?;
    let arr: [u8; 32] = v
        .try_into()
        .map_err(|_| anyhow!("decode32: need 32 bytes"))?;
    Ok(arr)
}

fn decode8(hex_str: &str) -> Result<[u8; 8]> {
    let s = strip_0x(hex_str);
    let padded = format!("{:0>16}", s);
    let v: Vec<u8> = hex::decode(&padded).map_err(|e| anyhow!("decode8: {}", e))?;
    let arr: [u8; 8] = v
        .try_into()
        .map_err(|_| anyhow!("decode8: need 8 bytes"))?;
    Ok(arr)
}

/// Parsing di mining.notify: params come array [job_id, seal_hash, target, extranonce]
fn parse_notify_params(params: &[Value]) -> Result<StratumJob> {
    if params.len() < 4 {
        return Err(anyhow!(
            "mining.notify serve almeno 4 param: job_id, seal_hash, target, extranonce"
        ));
    }
    let job_id = params[0]
        .as_str()
        .ok_or_else(|| anyhow!("job_id non string"))?
        .to_string();
    let seal_hex = params[1]
        .as_str()
        .ok_or_else(|| anyhow!("seal_hash non string"))?;
    let target_hex = params[2]
        .as_str()
        .ok_or_else(|| anyhow!("target non string"))?;
    let extranonce_hex = params[3]
        .as_str()
        .ok_or_else(|| anyhow!("extranonce non string"))?;
    Ok(StratumJob {
        job_id,
        seal_hash: decode32(seal_hex)?,
        target: decode32(target_hex)?,
        extra_nonce: decode8(extranonce_hex)?,
    })
}

/// Messaggio JSON-RPC Stratum (senza jsonrpc per brevità)
#[derive(Serialize)]
struct StratumRequest<'a> {
    id: u64,
    method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct StratumResponse {
    id: Option<u64>,
    result: Option<Value>,
    error: Option<Value>,
    method: Option<String>,
    params: Option<Value>,
}

pub struct StratumClient {
    stream: TcpStream,
    next_id: AtomicU64,
    /// User per authorize (es. "0xWallet.Worker001")
    user: String,
    /// Ultimo job ricevuto
    current_job: Arc<Mutex<Option<StratumJob>>>,
}

impl StratumClient {
    /// Parse URL stratum+tcp://user@host:port
    pub fn from_url(url: &str) -> Result<(Self, String)> {
        let url = url.strip_prefix("stratum+tcp://").ok_or_else(|| {
            anyhow!("URL deve iniziare con stratum+tcp://")
        })?;
        let (user_info, host_port) = url
            .split_once('@')
            .ok_or_else(|| anyhow!("URL deve essere stratum+tcp://USER@HOST:PORT"))?;
        let (host, port_str) = host_port
            .rsplit_once(':')
            .ok_or_else(|| anyhow!("manca :PORT in {}", host_port))?;
        let port: u16 = port_str
            .parse()
            .map_err(|_| anyhow!("porta non valida: {}", port_str))?;
        let addr = format!("{}:{}", host, port);
        let stream = TcpStream::connect(&addr)
            .map_err(|e| anyhow!("connessione a {}: {}", addr, e))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(120)))
            .ok();
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .ok();
        let next_id = AtomicU64::new(1);
        let user = user_info.to_string();
        let current_job = Arc::new(Mutex::new(None));
        Ok((
            Self {
                stream,
                next_id,
                user: user.clone(),
                current_job: current_job.clone(),
            },
            user,
        ))
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn send_request(&mut self, method: &str, params: Option<Value>) -> Result<u64> {
        let id = self.next_id();
        let req = StratumRequest {
            id,
            method,
            params,
        };
        let line = serde_json::to_string(&req)? + "\n";
        self.stream.write_all(line.as_bytes())?;
        self.stream.flush()?;
        Ok(id)
    }

    /// Esegue handshake: avvia thread lettura, subscribe + authorize, attende primo job
    pub fn connect(&mut self) -> Result<()> {
        let stream_reader = self.stream.try_clone()?;
        let job_arc = self.current_job.clone();
        thread::spawn(move || {
            let _ = Self::reader_loop(stream_reader, job_arc);
        });
        self.send_request("mining.subscribe", Some(serde_json::json!([])))?;
        self.send_request(
            "mining.authorize",
            Some(serde_json::json!([self.user.clone(), "x"])),
        )?;
        for _ in 0..30 {
            thread::sleep(Duration::from_secs(1));
            if self.current_job.lock().unwrap().is_some() {
                return Ok(());
            }
        }
        Err(anyhow!("timeout: nessun job dalla pool"))
    }

    /// Loop che legge messaggi dal server e aggiorna current_job su mining.notify
    pub fn reader_loop(
        stream: TcpStream,
        current_job: Arc<Mutex<Option<StratumJob>>>,
    ) -> Result<()> {
        stream.set_read_timeout(Some(Duration::from_secs(300)))?;
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                break;
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let msg: StratumResponse = match serde_json::from_str(line) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if let Some(method) = &msg.method {
                if method == "mining.notify" {
                    if let Some(Value::Array(params)) = msg.params {
                        match parse_notify_params(&params) {
                            Ok(job) => {
                                *current_job.lock().unwrap() = Some(job.clone());
                                println!("[stratum] nuovo job: {}", job.job_id);
                            }
                            Err(e) => eprintln!("[warn] mining.notify: {}", e),
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn current_job(&self) -> Arc<Mutex<Option<StratumJob>>> {
        self.current_job.clone()
    }

    /// Invia mining.submit(worker, job_id, nonce_hex, extranonce_hex)
    pub fn submit(
        &mut self,
        job_id: &str,
        nonce: &[u8; 8],
        extra_nonce: &[u8; 8],
    ) -> Result<u64> {
        let nonce_hex = hex::encode(nonce);
        let extra_hex = hex::encode(extra_nonce);
        let id = self.next_id();
        let req = StratumRequest {
            id,
            method: "mining.submit",
            params: Some(serde_json::json!([
                self.user,
                job_id,
                nonce_hex,
                extra_hex
            ])),
        };
        let line = serde_json::to_string(&req)? + "\n";
        self.stream.write_all(line.as_bytes())?;
        self.stream.flush()?;
        Ok(id)
    }

}
