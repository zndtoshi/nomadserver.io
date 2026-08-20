//! Electrs backend adapter: all chain data flows through here.
//!
//! Reliability model (learned the hard way in the legacy prototype):
//! - global single-flight gate: at most one Electrs RPC batch in flight
//! - 100 ms spacing between calls
//! - per-call timeouts; a timeout trips a short cooldown circuit-breaker
//!   so a hung backend fast-fails instead of queueing unbounded work
//! - a fresh connection per call batch (electrum-client is blocking and
//!   not Sync; connection setup is cheap on LAN)
//!
//! Errors map to protocol codes at the handler layer; nothing here knows
//! about the protocol.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bitcoin::block::Header as BlockHeader;
use bitcoin::{BlockHash, ScriptBuf, Txid};
use electrum_client::{Client, ElectrumApi};
use thiserror::Error;
use tokio::sync::Semaphore;

const CALL_SPACING: Duration = Duration::from_millis(100);
const COOLDOWN: Duration = Duration::from_secs(15);
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("backend in cooldown after timeout")]
    Cooldown,
    #[error("backend call timed out")]
    Timeout,
    #[error("backend error: {0}")]
    Electrum(String),
}

/// Chain tip: (height, block hash, block time).
pub type Tip = (usize, BlockHash, u32);

pub struct Electrs {
    url: String,
    gate: Semaphore,
    cooldown_until: Mutex<Option<Instant>>,
    last_call: Mutex<Option<Instant>>,
}

impl Electrs {
    pub fn new(addr: &str) -> Self {
        // Accept "host:port" or an already-prefixed URL.
        let url = if addr.contains("://") {
            addr.to_string()
        } else {
            format!("tcp://{addr}")
        };
        Self {
            url,
            gate: Semaphore::new(1),
            cooldown_until: Mutex::new(None),
            last_call: Mutex::new(None),
        }
    }

    /// Startup/health probe: connect + tip within a short timeout.
    pub async fn preflight(&self) -> Result<(), BackendError> {
        self.tip_inner(Duration::from_secs(5)).await.map(|_| ())
    }

    async fn call<T, F>(&self, timeout: Duration, f: F) -> Result<T, BackendError>
    where
        T: Send + 'static,
        F: FnOnce(&Client) -> Result<T, electrum_client::Error> + Send + 'static,
    {
        if let Some(until) = *self.cooldown_until.lock().unwrap() {
            if Instant::now() < until {
                return Err(BackendError::Cooldown);
            }
        }
        let _permit = self.gate.acquire().await.expect("semaphore open");
        let wait = {
            let mut last = self.last_call.lock().unwrap();
            let w = last
                .map(|t: Instant| CALL_SPACING.saturating_sub(t.elapsed()))
                .unwrap_or_default();
            *last = Some(Instant::now());
            w
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }

        let url = self.url.clone();
        let work =
            tokio::task::spawn_blocking(move || Client::new(&url).and_then(|c| f(&c)));
        match tokio::time::timeout(timeout, work).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(BackendError::Electrum(e.to_string())),
            Ok(Err(join)) => Err(BackendError::Electrum(join.to_string())),
            Err(_) => {
                *self.cooldown_until.lock().unwrap() = Some(Instant::now() + COOLDOWN);
                tracing::warn!("electrs call timed out; cooldown {}s", COOLDOWN.as_secs());
                Err(BackendError::Timeout)
            }
        }
    }

    /// Current tip: height, block hash, block time.
    pub async fn tip(&self) -> Result<Tip, BackendError> {
        self.tip_inner(DEFAULT_TIMEOUT).await
    }

    async fn tip_inner(&self, timeout: Duration) -> Result<Tip, BackendError> {
        let h = self
            .call(timeout, |c| c.block_headers_subscribe())
            .await?;
        Ok((h.height, h.header.block_hash(), h.header.time))
    }

    /// Batch balances, one entry per script (same order).
    pub async fn balances(
        &self,
        scripts: Vec<ScriptBuf>,
    ) -> Result<Vec<electrum_client::GetBalanceRes>, BackendError> {
        self.call(DEFAULT_TIMEOUT, move |c| {
            c.batch_script_get_balance(scripts.iter().map(|s| s.as_script()))
        })
        .await
    }

    /// Batch histories, one entry per script (same order).
    pub async fn histories(
        &self,
        scripts: Vec<ScriptBuf>,
    ) -> Result<Vec<Vec<electrum_client::GetHistoryRes>>, BackendError> {
        self.call(DEFAULT_TIMEOUT, move |c| {
            c.batch_script_get_history(scripts.iter().map(|s| s.as_script()))
        })
        .await
    }

    /// Batch UTXOs, one entry per script (same order).
    pub async fn unspents(
        &self,
        scripts: Vec<ScriptBuf>,
    ) -> Result<Vec<Vec<electrum_client::ListUnspentRes>>, BackendError> {
        self.call(DEFAULT_TIMEOUT, move |c| {
            c.batch_script_list_unspent(scripts.iter().map(|s| s.as_script()))
        })
        .await
    }

    /// Block hash + time for a set of heights (deduped by caller).
    pub async fn anchors(
        &self,
        heights: HashSet<usize>,
    ) -> Result<HashMap<usize, (BlockHash, u32)>, BackendError> {
        let mut out = HashMap::new();
        for h in heights {
            let header: BlockHeader = self
                .call(DEFAULT_TIMEOUT, move |c| c.block_header(h))
                .await?;
            out.insert(h, (header.block_hash(), header.time));
        }
        Ok(out)
    }

    /// Raw transaction bytes by txid.
    pub async fn raw_tx(&self, txid: Txid) -> Result<Vec<u8>, BackendError> {
        self.call(DEFAULT_TIMEOUT, move |c| c.transaction_get_raw(&txid))
            .await
    }

    /// Fee estimate (BTC/kB) for confirmation within `n` blocks.
    pub async fn estimate(&self, n: usize) -> Result<f64, BackendError> {
        self.call(DEFAULT_TIMEOUT, move |c| c.estimate_fee(n, None))
            .await
    }

    /// Broadcast a raw transaction; returns its txid.
    pub async fn broadcast(&self, raw: Vec<u8>) -> Result<Txid, BackendError> {
        self.call(DEFAULT_TIMEOUT, move |c| c.transaction_broadcast_raw(&raw))
            .await
    }
}

/// sat/vB from a BTC/kB estimate, clamped to ≥1.
pub fn btc_kb_to_sat_vb(btc_per_kb: f64) -> u64 {
    (btc_per_kb * 100_000.0).ceil().max(1.0) as u64
}

/// Txid from raw tx bytes (None if the bytes don't deserialize).
pub fn txid_of(raw: &[u8]) -> Option<Txid> {
    use bitcoin::consensus::Decodable;
    bitcoin::Transaction::consensus_decode(&mut &raw[..])
        .ok()
        .map(|tx| tx.compute_txid())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fee_conversion() {
        assert_eq!(btc_kb_to_sat_vb(0.00001), 1);
        assert_eq!(btc_kb_to_sat_vb(0.0001), 10);
        assert_eq!(btc_kb_to_sat_vb(0.0), 1); // clamp
    }

    #[test]
    fn url_normalization() {
        let e = Electrs::new("192.168.1.10:50001");
        assert_eq!(e.url, "tcp://192.168.1.10:50001");
        let e2 = Electrs::new("ssl://x:50002");
        assert_eq!(e2.url, "ssl://x:50002");
    }
}
