//! Request handlers for the chain-data message types (PROTOCOL.md §5).
//! `pair`/`unpair` stay in the transport; everything else lands here.
//! Validation and limits per PROTOCOL.md §7 happen BEFORE any backend
//! call. Handlers never see key material and never log payload content.

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;

use bitcoin::{Address, ScriptBuf, Txid};
use serde_json::json;

use crate::config::Network;
use crate::electrs::{btc_kb_to_sat_vb, txid_of, BackendError, Electrs};
use crate::protocol::{ErrorCode, RequestEnvelope, ResponseEnvelope};
use crate::store::WatchStore;
use crate::transport::SERVER_VERSION;

const MAX_QUERY_ADDRESSES: usize = 200;
const MAX_WATCH_ADDRESSES: usize = 5000;
const MAX_TXIDS: usize = 25;
const MAX_TXHEX_LEN: usize = 200_000; // 100 KB hex
const DEFAULT_HISTORY_LIMIT: usize = 50;
const MAX_HISTORY_LIMIT: usize = 200;

pub struct Handlers {
    electrs: Arc<Electrs>,
    network: Network,
    watch: Arc<WatchStore>,
}

/// (address string, script) after format + network validation.
fn validate_addresses(
    value: &serde_json::Value,
    network: Network,
    max: usize,
) -> Result<Vec<(String, ScriptBuf)>, ResponseEnvelope> {
    let invalid = || {
        ResponseEnvelope::err("", ErrorCode::InvalidRequest, "invalid addresses")
    };
    let list = value
        .get("addresses")
        .and_then(|a| a.as_array())
        .ok_or_else(invalid)?;
    if list.len() > max {
        return Err(ResponseEnvelope::err(
            "",
            ErrorCode::InvalidRequest,
            format!("too many addresses (max {max})"),
        ));
    }
    let mut out = Vec::with_capacity(list.len());
    for a in list {
        let s = a.as_str().ok_or_else(invalid)?;
        let addr = Address::from_str(s)
            .ok()
            .and_then(|a| a.require_network(bitcoin_network(network)).ok())
            .ok_or_else(invalid)?;
        out.push((s.to_string(), addr.script_pubkey()));
    }
    Ok(out)
}

fn bitcoin_network(n: Network) -> bitcoin::Network {
    match n {
        Network::Bitcoin => bitcoin::Network::Bitcoin,
        Network::Testnet => bitcoin::Network::Testnet,
        Network::Signet => bitcoin::Network::Signet,
        Network::Regtest => bitcoin::Network::Regtest,
    }
}

fn backend_err(id: &str, e: BackendError) -> ResponseEnvelope {
    match e {
        BackendError::Cooldown | BackendError::Timeout => ResponseEnvelope::err(
            id,
            ErrorCode::BackendUnavailable,
            "backend temporarily unavailable",
        ),
        BackendError::Electrum(msg) => {
            tracing::warn!("electrs error: {msg}");
            ResponseEnvelope::err(id, ErrorCode::BackendUnavailable, "backend error")
        }
    }
}

impl Handlers {
    pub fn new(electrs: Arc<Electrs>, network: Network, watch: Arc<WatchStore>) -> Self {
        Self {
            electrs,
            network,
            watch,
        }
    }

    pub async fn handle(&self, sender: &str, env: &RequestEnvelope) -> ResponseEnvelope {
        match env.msg_type.as_str() {
            "ping" => self.ping(env).await,
            "get_balance" => self.get_balance(env).await,
            "get_utxos" => self.get_utxos(env).await,
            "get_history" => self.get_history(env).await,
            "get_transactions" => self.get_transactions(env).await,
            "get_fee_estimates" => self.get_fee_estimates(env).await,
            "broadcast_tx" => self.broadcast_tx(env).await,
            "watch_addresses" => self.watch_addresses(sender, env).await,
            _ => ResponseEnvelope::err(&env.id, ErrorCode::UnknownType, "unknown type"),
        }
    }

    async fn ping(&self, env: &RequestEnvelope) -> ResponseEnvelope {
        match self.electrs.tip().await {
            Ok((height, hash, time)) => ResponseEnvelope::ok(
                &env.id,
                json!({
                    "server": SERVER_VERSION,
                    "tip": { "height": height, "hash": hash.to_string(), "time": time }
                }),
            ),
            Err(e) => backend_err(&env.id, e),
        }
    }

    async fn get_balance(&self, env: &RequestEnvelope) -> ResponseEnvelope {
        let addrs = match validate_addresses(&env.payload, self.network, MAX_QUERY_ADDRESSES) {
            Ok(a) => a,
            Err(mut e) => {
                e.id = env.id.clone();
                return e;
            }
        };
        if addrs.is_empty() {
            return ResponseEnvelope::err(
                &env.id,
                ErrorCode::InvalidRequest,
                "addresses must be non-empty",
            );
        }
        let scripts: Vec<ScriptBuf> = addrs.iter().map(|(_, s)| s.clone()).collect();
        match self.electrs.balances(scripts).await {
            Ok(res) => {
                let confirmed: u64 = res.iter().map(|b| b.confirmed).sum();
                let unconfirmed: i64 = res.iter().map(|b| b.unconfirmed).sum();
                ResponseEnvelope::ok(
                    &env.id,
                    json!({ "confirmed": confirmed, "unconfirmed": unconfirmed }),
                )
            }
            Err(e) => backend_err(&env.id, e),
        }
    }

    async fn get_utxos(&self, env: &RequestEnvelope) -> ResponseEnvelope {
        let addrs = match validate_addresses(&env.payload, self.network, MAX_QUERY_ADDRESSES) {
            Ok(a) => a,
            Err(mut e) => {
                e.id = env.id.clone();
                return e;
            }
        };
        if addrs.is_empty() {
            return ResponseEnvelope::err(
                &env.id,
                ErrorCode::InvalidRequest,
                "addresses must be non-empty",
            );
        }
        let tip = match self.electrs.tip().await {
            Ok((h, _, _)) => h,
            Err(e) => return backend_err(&env.id, e),
        };
        let scripts: Vec<ScriptBuf> = addrs.iter().map(|(_, s)| s.clone()).collect();
        match self.electrs.unspents(scripts).await {
            Ok(per_addr) => {
                let mut utxos = Vec::new();
                for ((addr, _), unsp) in addrs.iter().zip(per_addr.into_iter()) {
                    for u in unsp {
                        let confirmations = if u.height == 0 {
                            0
                        } else {
                            tip.saturating_sub(u.height) + 1
                        };
                        utxos.push(json!({
                            "txid": u.tx_hash.to_string(),
                            "vout": u.tx_pos,
                            "value": u.value,
                            "address": addr,
                            "confirmations": confirmations,
                        }));
                    }
                }
                ResponseEnvelope::ok(&env.id, json!({ "utxos": utxos }))
            }
            Err(e) => backend_err(&env.id, e),
        }
    }

    async fn get_history(&self, env: &RequestEnvelope) -> ResponseEnvelope {
        let addrs = match validate_addresses(&env.payload, self.network, MAX_QUERY_ADDRESSES) {
            Ok(a) => a,
            Err(mut e) => {
                e.id = env.id.clone();
                return e;
            }
        };
        if addrs.is_empty() {
            return ResponseEnvelope::err(
                &env.id,
                ErrorCode::InvalidRequest,
                "addresses must be non-empty",
            );
        }
        let limit = env
            .payload
            .get("limit")
            .and_then(|l| l.as_u64())
            .map(|l| (l as usize).clamp(1, MAX_HISTORY_LIMIT))
            .unwrap_or(DEFAULT_HISTORY_LIMIT);

        let scripts: Vec<ScriptBuf> = addrs.iter().map(|(_, s)| s.clone()).collect();
        let histories = match self.electrs.histories(scripts).await {
            Ok(h) => h,
            Err(e) => return backend_err(&env.id, e),
        };

        // merge + dedupe by txid (a tx can touch several of the wallet's
        // addresses), newest first
        let mut seen = std::collections::HashMap::new();
        for hist in histories {
            for h in hist {
                let height = h.height.max(0) as usize;
                seen.entry(h.tx_hash)
                    .and_modify(|ht: &mut usize| *ht = (*ht).max(height))
                    .or_insert(height);
            }
        }
        let mut txs: Vec<(Txid, usize)> = seen.into_iter().collect();
        txs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        txs.truncate(limit);

        // block anchors for the confirmed heights in play
        let heights: HashSet<usize> = txs.iter().map(|(_, h)| *h).filter(|h| *h > 0).collect();
        let anchors = match self.electrs.anchors(heights).await {
            Ok(a) => a,
            Err(e) => return backend_err(&env.id, e),
        };

        let out: Vec<_> = txs
            .into_iter()
            .map(|(txid, height)| {
                if height == 0 {
                    json!({ "txid": txid.to_string(), "height": 0 })
                } else {
                    let (hash, time) = anchors[&height];
                    json!({
                        "txid": txid.to_string(),
                        "height": height,
                        "block_hash": hash.to_string(),
                        "block_time": time,
                    })
                }
            })
            .collect();
        ResponseEnvelope::ok(&env.id, json!({ "txs": out }))
    }

    async fn get_transactions(&self, env: &RequestEnvelope) -> ResponseEnvelope {
        let txids: Vec<Txid> = match env
            .payload
            .get("txids")
            .and_then(|t| t.as_array())
            .filter(|a| !a.is_empty() && a.len() <= MAX_TXIDS)
        {
            Some(list) => {
                let mut v = Vec::with_capacity(list.len());
                for t in list {
                    match t.as_str().and_then(|s| s.parse().ok()) {
                        Some(id) => v.push(id),
                        None => {
                            return ResponseEnvelope::err(
                                &env.id,
                                ErrorCode::InvalidRequest,
                                "invalid txid",
                            )
                        }
                    }
                }
                v
            }
            None => {
                return ResponseEnvelope::err(
                    &env.id,
                    ErrorCode::InvalidRequest,
                    format!("txids must be 1..={MAX_TXIDS}"),
                )
            }
        };
        let mut txs = Vec::with_capacity(txids.len());
        for txid in txids {
            match self.electrs.raw_tx(txid).await {
                Ok(raw) => txs.push(json!({
                    "txid": txid.to_string(),
                    "hex": hex::encode(raw),
                })),
                Err(e) => return backend_err(&env.id, e),
            }
        }
        ResponseEnvelope::ok(&env.id, json!({ "txs": txs }))
    }

    async fn get_fee_estimates(&self, env: &RequestEnvelope) -> ResponseEnvelope {
        let fast = match self.electrs.estimate(1).await {
            Ok(f) => f,
            Err(e) => return backend_err(&env.id, e),
        };
        let medium = match self.electrs.estimate(6).await {
            Ok(f) => f,
            Err(e) => return backend_err(&env.id, e),
        };
        let slow = match self.electrs.estimate(12).await {
            Ok(f) => f,
            Err(e) => return backend_err(&env.id, e),
        };
        ResponseEnvelope::ok(
            &env.id,
            json!({
                "fast": btc_kb_to_sat_vb(fast),
                "medium": btc_kb_to_sat_vb(medium),
                "slow": btc_kb_to_sat_vb(slow),
            }),
        )
    }

    async fn broadcast_tx(&self, env: &RequestEnvelope) -> ResponseEnvelope {
        let tx_hex = match env.payload.get("txHex").and_then(|t| t.as_str()) {
            Some(h) if !h.is_empty() && h.len() <= MAX_TXHEX_LEN => h,
            _ => {
                return ResponseEnvelope::err(
                    &env.id,
                    ErrorCode::InvalidRequest,
                    format!("txHex must be 1..={MAX_TXHEX_LEN} hex chars"),
                )
            }
        };
        let raw = match hex::decode(tx_hex) {
            Ok(r) => r,
            Err(_) => {
                return ResponseEnvelope::err(&env.id, ErrorCode::InvalidTx, "invalid hex")
            }
        };
        if txid_of(&raw).is_none() {
            return ResponseEnvelope::err(&env.id, ErrorCode::InvalidTx, "invalid transaction");
        }
        match self.electrs.broadcast(raw).await {
            Ok(txid) => ResponseEnvelope::ok(&env.id, json!({ "txid": txid.to_string() })),
            Err(BackendError::Electrum(msg)) => ResponseEnvelope::err(
                &env.id,
                ErrorCode::BroadcastFailed,
                format!("broadcast failed: {msg}"),
            ),
            Err(e) => backend_err(&env.id, e),
        }
    }

    async fn watch_addresses(&self, sender: &str, env: &RequestEnvelope) -> ResponseEnvelope {
        let addrs = match validate_addresses(&env.payload, self.network, MAX_WATCH_ADDRESSES) {
            Ok(a) => a,
            Err(mut e) => {
                e.id = env.id.clone();
                return e;
            }
        };
        let list: Vec<String> = addrs.into_iter().map(|(s, _)| s).collect();
        match self.watch.replace(sender, list) {
            Ok(n) => {
                tracing::info!(sender = %sender, watching = n, "watch set updated");
                ResponseEnvelope::ok(&env.id, json!({ "watching": n }))
            }
            Err(e) => {
                tracing::error!("failed to persist watch set: {e}");
                ResponseEnvelope::err(&env.id, ErrorCode::Internal, "internal error")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_network_and_counts() {
        // testnet address against a mainnet server
        let payload = json!({ "addresses": ["tb1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh"] });
        assert!(validate_addresses(&payload, Network::Bitcoin, 200).is_err());
        // too many
        let many: Vec<String> = (0..201)
            .map(|_| "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh".to_string())
            .collect();
        let payload = json!({ "addresses": many });
        let err = validate_addresses(&payload, Network::Bitcoin, 200).unwrap_err();
        assert_eq!(err.error.unwrap().code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn accepts_mainnet_p2wpkh() {
        let payload = json!({ "addresses": ["bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh"] });
        let out = validate_addresses(&payload, Network::Bitcoin, 200).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1.len(), 22); // P2WPKH scriptPubKey
    }
}
