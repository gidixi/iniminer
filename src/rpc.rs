/// Client JSON-RPC per il nodo InitVerse (chain).
///
/// Contratto allineato a:
///   - chain/consensus/inihash/sealer.go (makeWork, submitWork)
///   - chain/consensus/inihash/api.go (GetWork, SubmitWork)
///
/// eth_getWork: ritorno [4]string come da makeWork():
///   [0] = SealHash(header).Hex()     — 32 byte hex (header pow-hash)
///   [1] = target.Hex()               — 2^256/difficulty, 32 byte hex
///   [2] = block number hex
///   [3] = block time hex
///
/// eth_submitWork(nonce_hex, extra_nonce_hex, sealhash_hex): come SubmitWork(nonce, extraNonce, hash).

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'a str,
    method: &'a str,
    params: Value,
    id: u64,
}

#[derive(Deserialize, Debug)]
struct JsonRpcResponse<T> {
    result: Option<T>,
    error: Option<Value>,
}

pub struct RpcClient {
    url: String,
    client: reqwest::blocking::Client,
    id: AtomicU64,
}

impl RpcClient {
    pub fn new(url: String) -> Self {
        Self {
            url,
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap(),
            id: AtomicU64::new(1),
        }
    }

    fn next_id(&self) -> u64 {
        self.id.fetch_add(1, Ordering::Relaxed)
    }

    fn call<T: for<'de> Deserialize<'de>>(&self, method: &str, params: Value) -> Result<T> {
        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            method,
            params,
            id: self.next_id(),
        };
        let resp: JsonRpcResponse<T> = self
            .client
            .post(&self.url)
            .json(&req)
            .send()?
            .json()?;
        resp.result
            .ok_or_else(|| anyhow!("RPC error da {}: {:?}", method, resp.error))
    }

    /// Ritorna [headerHash, target, blockNumber, timestamp] come stringhe "0x…"
    pub fn get_work(&self) -> Result<[String; 4]> {
        self.call("eth_getWork", serde_json::json!([]))
    }

    /// Invia la soluzione al nodo.
    /// nonce, extra_nonce: 8 byte big-endian
    /// header_hash: 32 byte (SealHash del blocco)
    pub fn submit_work(
        &self,
        nonce: &[u8; 8],
        extra_nonce: &[u8; 8],
        header_hash: &[u8; 32],
    ) -> Result<bool> {
        let nonce_hex = format!("0x{}", hex::encode(nonce));
        let extra_hex = format!("0x{}", hex::encode(extra_nonce));
        let hash_hex = format!("0x{}", hex::encode(header_hash));
        self.call(
            "eth_submitWork",
            serde_json::json!([nonce_hex, extra_hex, hash_hex]),
        )
    }
}
