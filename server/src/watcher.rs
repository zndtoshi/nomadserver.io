//! The watcher: monitors paired wallets' watch sets and pushes
//! `notify/new_tx` gift wraps (PROTOCOL.md §5.8/§5.9).
//!
//! Model: every INTERVAL, snapshot each paired wallet's watch set, batch
//! query Electrs for per-address histories, and diff against the last
//! known (txid → height) map:
//! - txid not in the map          → new transaction (mempool if height 0)
//! - was height 0, now height > 0 → newly confirmed
//! - anything else                → no notification (dropped/RBF'd txs are
//!                                  silent in v1)
//!
//! First observation of a wallet is silent (no notifications for history
//! that predates watching). State is persisted so a server restart does
//! not re-notify.

use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bitcoin::Address;
use nostr::nips::nip59::GiftWrapBuilder;
use nostr_sdk::prelude::*;

use crate::config::Network;
use crate::electrs::Electrs;
use crate::pairing::now_secs;
use crate::protocol::KIND_NOTIFY;
use crate::store::{Allowlist, WatchStore};
use crate::transport::WRAP_EXPIRATION;

const STATE_FILE: &str = "watcher_state.json";
const DEFAULT_INTERVAL: Duration = Duration::from_secs(60);
/// Chunk size for batched history queries.
const BATCH: usize = 200;

/// txid -> last known height, per wallet pubkey.
type WalletState = HashMap<String, HashMap<String, u64>>;

/// Which notifications a state transition implies.
/// `known == None` means first observation: no notifications.
pub fn diff(
    known: Option<&HashMap<String, u64>>,
    current: &HashMap<String, u64>,
) -> Vec<(String, u64)> {
    let Some(known) = known else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (txid, height) in current {
        match known.get(txid) {
            None => out.push((txid.clone(), *height)),
            Some(0) if *height > 0 => out.push((txid.clone(), *height)),
            _ => {}
        }
    }
    out
}

pub struct Watcher {
    keys: Keys,
    client: Client,
    electrs: Arc<Electrs>,
    network: Network,
    watch: Arc<WatchStore>,
    allowlist: Arc<Allowlist>,
    state_path: std::path::PathBuf,
    state: Mutex<WalletState>,
    interval: Duration,
}

impl Watcher {
    pub fn new(
        keys: Keys,
        client: Client,
        electrs: Arc<Electrs>,
        network: Network,
        watch: Arc<WatchStore>,
        allowlist: Arc<Allowlist>,
        data_dir: &Path,
    ) -> anyhow::Result<Self> {
        let state_path = data_dir.join(STATE_FILE);
        let state: WalletState = match std::fs::read_to_string(&state_path) {
            Ok(data) => serde_json::from_str(&data)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => return Err(e.into()),
        };
        let interval = std::env::var("NOMAD_WATCH_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_INTERVAL);
        Ok(Self {
            keys,
            client,
            electrs,
            network,
            watch,
            allowlist,
            state_path,
            state: Mutex::new(state),
            interval,
        })
    }

    pub async fn run(self) {
        tracing::info!("watcher running: interval {}s", self.interval.as_secs());
        loop {
            if let Err(e) = self.tick().await {
                tracing::warn!("watcher tick failed: {e}");
            }
            tokio::time::sleep(self.interval).await;
        }
    }

    async fn tick(&self) -> anyhow::Result<()> {
        let snapshot = self.watch.snapshot();
        for (wallet, addrs) in &snapshot {
            if !self.allowlist.is_paired(wallet) {
                continue;
            }
            if let Err(e) = self.tick_wallet(wallet, addrs).await {
                tracing::warn!(wallet = %wallet, "watch tick failed: {e}");
            }
        }
        // forget wallets that unpaired or cleared their sets
        let mut state = self.state.lock().unwrap();
        let before = state.len();
        state.retain(|w, _| snapshot.contains_key(w));
        if state.len() != before {
            drop(state);
            self.persist();
        }
        Ok(())
    }

    async fn tick_wallet(&self, wallet: &str, addrs: &[String]) -> anyhow::Result<()> {
        if addrs.is_empty() {
            return Ok(());
        }
        // scripts for the watched addresses (validated at registration;
        // re-validated defensively — a bad stored address is skipped)
        let mut scripts = Vec::with_capacity(addrs.len());
        let mut addr_of = Vec::with_capacity(addrs.len());
        for a in addrs {
            if let Ok(addr) = Address::from_str(a)
                .ok()
                .and_then(|x| {
                    x.require_network(match self.network {
                        Network::Bitcoin => bitcoin::Network::Bitcoin,
                        Network::Testnet => bitcoin::Network::Testnet,
                        Network::Signet => bitcoin::Network::Signet,
                        Network::Regtest => bitcoin::Network::Regtest,
                    })
                    .ok()
                })
                .ok_or(())
            {
                scripts.push(addr.script_pubkey());
                addr_of.push(a.clone());
            }
        }

        // current txid -> height map + which watched addresses each tx touches
        let mut current: HashMap<String, u64> = HashMap::new();
        let mut touched: HashMap<String, Vec<String>> = HashMap::new();
        for (i, chunk) in scripts.chunks(BATCH).enumerate() {
            let histories = self.electrs.histories(chunk.to_vec()).await?;
            for (j, hist) in histories.into_iter().enumerate() {
                let addr = &addr_of[i * BATCH + j];
                for h in hist {
                    let txid = h.tx_hash.to_string();
                    let height = h.height.max(0) as u64;
                    current
                        .entry(txid.clone())
                        .and_modify(|ht| *ht = (*ht).max(height))
                        .or_insert(height);
                    touched.entry(txid).or_default().push(addr.clone());
                }
            }
        }

        let deltas = {
            let mut state = self.state.lock().unwrap();
            let d = diff(state.get(wallet), &current);
            state.insert(wallet.to_string(), current);
            d
        };
        self.persist();

        for (txid, height) in deltas {
            let addresses = touched.get(&txid).cloned().unwrap_or_default();
            if let Err(e) = self.notify(wallet, &txid, &addresses, height).await {
                tracing::warn!(wallet = %wallet, "notify send failed: {e}");
            }
        }
        Ok(())
    }

    fn persist(&self) {
        let state = self.state.lock().unwrap();
        if let Ok(data) = serde_json::to_string(&*state) {
            let tmp = self.state_path.with_extension("tmp");
            if std::fs::write(&tmp, data).is_ok() {
                let _ = std::fs::rename(&tmp, &self.state_path);
            }
        }
    }

    async fn notify(
        &self,
        wallet_hex: &str,
        txid: &str,
        addresses: &[String],
        height: u64,
    ) -> anyhow::Result<()> {
        let wallet_pk = PublicKey::from_hex(wallet_hex)?;
        let envelope = serde_json::json!({
            "v": 1,
            "id": uuid::Uuid::new_v4().to_string(),
            "ts": now_secs(),
            "type": "notify",
            "payload": {
                "kind": "new_tx",
                "txid": txid,
                "addresses": addresses,
                "height": height,
            }
        });
        let rumor = EventBuilder::new(Kind::Custom(KIND_NOTIFY), envelope.to_string())
            .finalize_unsigned(self.keys.public_key());
        let wrap = GiftWrapBuilder::new(wallet_pk, rumor)
            .expiration(WRAP_EXPIRATION)
            .finalize(&self.keys)?;
        self.client.send_event(&wrap).await?;
        tracing::info!(wallet = %wallet_hex, %txid, height, "notify/new_tx sent");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, u64)]) -> HashMap<String, u64> {
        pairs.iter().map(|(t, h)| (t.to_string(), *h)).collect()
    }

    #[test]
    fn first_observation_is_silent() {
        let current = map(&[("tx1", 800000), ("tx2", 0)]);
        assert!(diff(None, &current).is_empty());
    }

    #[test]
    fn new_and_confirmed_detected() {
        let known = map(&[("tx1", 800000), ("tx2", 0)]);
        let current = map(&[("tx1", 800000), ("tx2", 800003), ("tx3", 0)]);
        let mut d = diff(Some(&known), &current);
        d.sort();
        assert_eq!(
            d,
            vec![
                ("tx2".to_string(), 800003), // confirmed
                ("tx3".to_string(), 0),      // new in mempool
            ]
        );
    }

    #[test]
    fn dropped_and_unchanged_are_silent() {
        let known = map(&[("tx1", 0)]);
        let current = map(&[]); // dropped from mempool
        assert!(diff(Some(&known), &current).is_empty());
        let same = map(&[("tx1", 0)]);
        assert!(diff(Some(&known), &same).is_empty());
    }
}
