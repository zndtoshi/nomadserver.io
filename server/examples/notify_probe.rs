//! Dev/integration tool: verifies the watcher path end-to-end.
//!
//! Usage:
//!   cargo run --example notify_probe -- [pairing-url] [address] [listen-secs]
//!
//! Pairs, registers `address` in the watch set, then listens for
//! `notify/*` envelopes and prints them. Use with scripts/mock_electrum.py
//! (its scenario adds a mempool tx at +20s, confirmed at +45s) and
//! NOMAD_WATCH_INTERVAL_SECS=5 for the server.

use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use futures::{Stream, StreamExt};
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

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install rustls ring provider");

    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:3829/pairing".to_string());
    let watch_addr = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa".to_string());
    let listen_secs: u64 = std::env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);

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

    // one notification stream shared by the whole session
    let mut notifications = client.notifications();

    // helper: send a request and await its response on the shared stream
    async fn roundtrip(
        client: &Client,
        notifications: &mut std::pin::Pin<Box<dyn Stream<Item = ClientNotification> + Send>>,
        keys: &Keys,
        server_pk: &PublicKey,
        msg_type: &str,
        payload: serde_json::Value,
    ) -> anyhow::Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let request = serde_json::json!({
            "v": 1, "id": id, "ts": now(), "type": msg_type, "payload": payload
        });
        let rumor = EventBuilder::new(Kind::Custom(25078), request.to_string())
            .finalize_unsigned(keys.public_key());
        let wrap = GiftWrapBuilder::new(*server_pk, rumor)
            .expiration(WRAP_EXP)
            .finalize(keys)?;
        for attempt in 1..=3u8 {
            client.send_event(&wrap).await?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
            while let Ok(Some(n)) = tokio::time::timeout_at(deadline, notifications.next()).await {
                let ClientNotification::Event { event, .. } = n else {
                    continue;
                };
                if event.kind != Kind::GiftWrap {
                    continue;
                }
                let Ok(gift) = UnwrappedGift::from_gift_wrap(keys, &event) else {
                    continue;
                };
                if gift.sender != *server_pk {
                    continue;
                }
                let Ok(resp) = serde_json::from_str::<serde_json::Value>(&gift.rumor.content)
                else {
                    continue;
                };
                if resp["id"].as_str() != Some(id.as_str()) {
                    continue;
                }
                anyhow::ensure!(resp["ok"].as_bool() == Some(true),
                    "{msg_type} -> error: {}", resp["error"]);
                println!("{msg_type} ok: {}", resp["result"]);
                return Ok(());
            }
            eprintln!("  ({msg_type} attempt {attempt} timed out)");
        }
        anyhow::bail!("{msg_type} failed after retries")
    }

    roundtrip(
        &client,
        &mut notifications,
        &wallet,
        &server_pk,
        "pair",
        serde_json::json!({ "proof": proof, "client": "notify-probe/dev" }),
    )
    .await?;
    roundtrip(
        &client,
        &mut notifications,
        &wallet,
        &server_pk,
        "watch_addresses",
        serde_json::json!({ "addresses": [watch_addr] }),
    )
    .await?;

    println!("listening {listen_secs}s for notifications…");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(listen_secs);
    while let Ok(Some(n)) = tokio::time::timeout_at(deadline, notifications.next()).await {
        let ClientNotification::Event { event, .. } = n else {
            continue;
        };
        if event.kind != Kind::GiftWrap {
            continue;
        }
        let Ok(gift) = UnwrappedGift::from_gift_wrap(&wallet, &event) else {
            continue;
        };
        if gift.sender != server_pk {
            continue;
        }
        if let Ok(env) = serde_json::from_str::<serde_json::Value>(&gift.rumor.content) {
            if env["type"].as_str() == Some("notify") {
                println!("NOTIFY  {}", env["payload"]);
            }
        }
    }
    println!("done");
    Ok(())
}
