//! Dev/integration tool: acts as a wallet against a running nomad-server.
//!
//! Usage:
//!   cargo run --example pair_client -- [pairing-url] [address]
//! defaults: pairing-url http://127.0.0.1:3829/pairing, no chain probes
//!
//! Performs the pairing handshake (PROTOCOL.md §4), then, if an address is
//! given, probes ping/get_balance/get_utxos/get_history/get_fee_estimates
//! and prints the results. Exit code 0 = everything ok.

use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use nostr::nips::nip59::{GiftWrapBuilder, UnwrappedGift};
use nostr_sdk::prelude::*;
use serde::Deserialize;
use sha2::Sha256;

const WRAP_EXP: Duration = Duration::from_secs(9 * 24 * 3600);

#[derive(Deserialize)]
struct PairingPayload {
    #[serde(rename = "nodePubkey")]
    node_pubkey: String,
    relays: Vec<String>,
    #[serde(rename = "pairSecret")]
    pair_secret: String,
    exp: u64,
}

struct Wallet {
    keys: Keys,
    client: Client,
    server_pk: PublicKey,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

impl Wallet {
    async fn request(&self, msg_type: &str, payload: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let id = uuid::Uuid::new_v4().to_string();
        let request = serde_json::json!({
            "v": 1, "id": id, "ts": now(), "type": msg_type, "payload": payload
        });
        let rumor = EventBuilder::new(Kind::Custom(25078), serde_json::to_string(&request)?)
            .finalize_unsigned(self.keys.public_key());
        let wrap = GiftWrapBuilder::new(self.server_pk, rumor)
            .expiration(WRAP_EXP)
            .finalize(&self.keys)?;
        self.client.send_event(&wrap).await?;

        let mut notifications = self.client.notifications();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let n = match tokio::time::timeout_at(deadline, notifications.next()).await {
                Ok(Some(n)) => n,
                _ => anyhow::bail!("timeout waiting for {msg_type} response"),
            };
            let ClientNotification::Event { event, .. } = n else {
                continue;
            };
            if event.kind != Kind::GiftWrap {
                continue;
            }
            let Ok(gift) = UnwrappedGift::from_gift_wrap(&self.keys, &event) else {
                continue;
            };
            if gift.sender != self.server_pk {
                continue;
            }
            let resp: serde_json::Value = serde_json::from_str(&gift.rumor.content)?;
            if resp["id"].as_str() != Some(id.as_str()) {
                continue; // backfilled response to an older request
            }
            if resp["ok"].as_bool() == Some(true) {
                return Ok(resp["result"].clone());
            }
            anyhow::bail!("{} -> error: {}", msg_type, resp["error"]);
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install rustls ring provider");

    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:3829/pairing".to_string());
    let probe_addr = std::env::args().nth(2);

    println!("fetching pairing payload from {url}");
    let payload: PairingPayload = ureq::get(&url).call()?.body_mut().read_json()?;
    anyhow::ensure!(payload.exp > now(), "pairing secret expired; retry");

    let wallet = Keys::generate();
    println!("wallet pubkey: {}", wallet.public_key().to_hex());

    let secret = URL_SAFE_NO_PAD.decode(&payload.pair_secret)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(&secret)?;
    mac.update(wallet.public_key().to_hex().as_bytes());
    let proof = hex::encode(mac.finalize().into_bytes());

    let server_pk = PublicKey::from_hex(&payload.node_pubkey)?;
    let client = Client::new();
    for r in &payload.relays {
        client.add_relay(r.as_str()).await?;
    }
    client.connect().and_wait(Duration::from_secs(10)).await;
    let filter = Filter::new()
        .kind(Kind::GiftWrap)
        .pubkey(wallet.public_key())
        .since(Timestamp::now() - Duration::from_secs(3 * 24 * 3600));
    client.subscribe(filter).await?;

    let w = Wallet {
        keys: wallet,
        client,
        server_pk,
    };

    let result = w
        .request(
            "pair",
            serde_json::json!({ "proof": proof, "client": "pair-client/dev" }),
        )
        .await?;
    println!("PAIRED ok  server={}", result["server"]);

    if let Some(addr) = probe_addr {
        let ping = w.request("ping", serde_json::json!({})).await?;
        println!("PING  {}", ping);

        let bal = w
            .request("get_balance", serde_json::json!({ "addresses": [addr] }))
            .await?;
        println!("BALANCE  {}", bal);

        let utxos = w
            .request("get_utxos", serde_json::json!({ "addresses": [addr] }))
            .await?;
        println!("UTXOS  {}", serde_json::to_string_pretty(&utxos)?);

        let hist = w
            .request(
                "get_history",
                serde_json::json!({ "addresses": [addr], "limit": 5 }),
            )
            .await?;
        println!("HISTORY  {}", serde_json::to_string_pretty(&hist)?);

        let fees = w.request("get_fee_estimates", serde_json::json!({})).await?;
        println!("FEES  {}", fees);

        let watch = w
            .request("watch_addresses", serde_json::json!({ "addresses": [addr] }))
            .await?;
        println!("WATCH  {}", watch);
    }
    Ok(())
}
