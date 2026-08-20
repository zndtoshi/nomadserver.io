//! Dev/integration tool: acts as a wallet performing the pairing handshake
//! against a running nomad-server (PROTOCOL.md §4).
//!
//! Usage:
//!   cargo run --example pair_client -- [pairing-url]
//! default pairing-url: http://127.0.0.1:3829/pairing
//!
//! Fetches the pairing payload, gift-wraps a `pair` request to the server
//! over the listed relays, waits for the response, prints the result.

use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use nostr::nips::nip59::{GiftWrapBuilder, UnwrappedGift};
use nostr_sdk::prelude::*;
use serde::Deserialize;
use sha2::Sha256;

#[derive(Deserialize)]
struct PairingPayload {
    #[serde(rename = "nodePubkey")]
    node_pubkey: String,
    relays: Vec<String>,
    #[serde(rename = "pairSecret")]
    pair_secret: String,
    exp: u64,
}

#[derive(Deserialize)]
struct ResponseEnvelope {
    ok: bool,
    result: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:3829/pairing".to_string());
    println!("fetching pairing payload from {url}");
    let payload: PairingPayload = ureq::get(&url).call()?.body_mut().read_json()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    anyhow::ensure!(payload.exp > now, "pairing secret expired; retry");

    let wallet = Keys::generate();
    let wallet_pk = wallet.public_key().to_hex();
    println!("wallet pubkey: {wallet_pk}");

    // proof = HMAC-SHA256(key = pairSecret, msg = wallet pubkey hex)
    let secret = URL_SAFE_NO_PAD.decode(&payload.pair_secret)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(&secret)?;
    mac.update(wallet_pk.as_bytes());
    let proof = hex::encode(mac.finalize().into_bytes());

    let server_pk = nostr::key::PublicKey::from_hex(&payload.node_pubkey)?;

    let client = Client::new();
    for r in &payload.relays {
        client.add_relay(r.as_str()).await?;
    }
    client.connect().and_wait(Duration::from_secs(10)).await;

    let filter = Filter::new()
        .kind(Kind::GiftWrap)
        .pubkey(wallet.public_key())
        // backfill stored wraps (created_at is randomized ±2 days)
        .since(Timestamp::now() - Duration::from_secs(3 * 24 * 3600));
    client.subscribe(filter).await?;

    let id = uuid::Uuid::new_v4().to_string();
    let request = serde_json::json!({
        "v": 1, "id": id, "ts": now, "type": "pair",
        "payload": { "proof": proof, "client": "pair-client/dev" }
    });
    let rumor = EventBuilder::new(Kind::Custom(25078), serde_json::to_string(&request)?)
        .finalize_unsigned(wallet.public_key());
    let wrap = GiftWrapBuilder::new(server_pk, rumor)
        .expiration(Duration::from_secs(9 * 24 * 3600))
        .finalize(&wallet)?;
    client.send_event(&wrap).await?;
    println!("pair request sent (id {id}); waiting for response…");

    let mut notifications = client.notifications();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let n = match tokio::time::timeout_at(deadline, notifications.next()).await {
            Ok(Some(n)) => n,
            _ => anyhow::bail!("timed out waiting for server response"),
        };
        let ClientNotification::Event { event, .. } = n else {
            continue;
        };
        if event.kind != Kind::GiftWrap {
            continue;
        }
        let Ok(gift) = UnwrappedGift::from_gift_wrap(&wallet, &event) else {
            continue; // spam wrap
        };
        if gift.sender.to_hex() != payload.node_pubkey {
            continue;
        }
        let resp: ResponseEnvelope = serde_json::from_str(&gift.rumor.content)?;
        if resp.ok {
            println!("PAIRED ✔  server: {}", resp.result.unwrap()["server"]);
        } else {
            println!("PAIRING FAILED: {}", resp.error.unwrap());
            std::process::exit(1);
        }
        break;
    }
    Ok(())
}
