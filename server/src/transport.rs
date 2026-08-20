//! Gift-wrap transport (PROTOCOL.md §2): relay lifecycle, subscription,
//! unwrap → authorize → route → respond.
//!
//! Flow per incoming event: kind check → NIP-59 unwrap (nostr-sdk verifies
//! the wrap and seal signatures and the rumor/seal sender match) → rumor
//! kind must be a request → parse envelope → replay check → rate limit →
//! route. Authorization (allowlist) happens immediately after unwrap,
//! before any other work (THREAT_MODEL.md §3).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nostr::nips::nip59::{GiftWrapBuilder, UnwrappedGift};
use nostr_sdk::prelude::*;

use crate::handlers::Handlers;
use crate::pairing::{now_secs, PairingManager};
use crate::protocol::{
    parse_request, ErrorCode, RequestEnvelope, ResponseEnvelope, KIND_REQUEST, KIND_RESPONSE,
    KNOWN_TYPES,
};
use crate::ratelimit::RateLimiter;
use crate::replay::ReplayCache;
use crate::store::{Allowlist, WatchStore};

/// NIP-40 expiration on gift wraps: 9 days from the (randomized, up to
/// 2 days in the past) created_at, i.e. ≥7 days from real send time
/// (PROTOCOL.md §2.2). Replay cache outlives this (replay.rs).
const WRAP_EXPIRATION: Duration = Duration::from_secs(9 * 24 * 3600);

pub const SERVER_VERSION: &str = concat!("nomad-server/", env!("CARGO_PKG_VERSION"));

pub struct Transport {
    keys: Keys,
    client: Client,
    relays: Vec<String>,
    allowlist: Arc<Allowlist>,
    watch: Arc<WatchStore>,
    pairing: Arc<Mutex<PairingManager>>,
    replay: ReplayCache,
    ratelimit: RateLimiter,
    handlers: Arc<Handlers>,
}

impl Transport {
    pub fn new(
        keys: Keys,
        relays: Vec<String>,
        allowlist: Arc<Allowlist>,
        watch: Arc<WatchStore>,
        pairing: Arc<Mutex<PairingManager>>,
        replay: ReplayCache,
        handlers: Arc<Handlers>,
    ) -> Self {
        Self {
            keys,
            client: Client::new(),
            relays,
            allowlist,
            watch,
            pairing,
            replay,
            ratelimit: RateLimiter::new(),
            handlers,
        }
    }

    /// Connect, subscribe, and process requests forever.
    pub async fn run(self) -> anyhow::Result<()> {
        for url in &self.relays {
            if let Err(e) = self.client.add_relay(url.as_str()).await {
                tracing::warn!("failed to add relay {url}: {e}");
            }
        }
        self.client
            .connect()
            .and_wait(std::time::Duration::from_secs(15))
            .await;

        // Backfill window: gift wraps are STORED events with randomized
        // created_at (±2 days), so subscribe since now-3d (tweak + margin)
        // instead of live-only. Wraps already handled are dropped by the
        // replay cache (envelope id), making this safe and gap-tolerant
        // (PROTOCOL.md §2.3).
        let filter = Filter::new()
            .kind(nostr::event::Kind::GiftWrap)
            .pubkey(self.keys.public_key())
            .since(Timestamp::now() - Duration::from_secs(3 * 24 * 3600));
        self.client.subscribe(filter).await?;
        tracing::info!(
            "transport listening: {} relays, pubkey={}",
            self.relays.len(),
            self.keys.public_key().to_hex()
        );

        let mut notifications = self.client.notifications();
        while let Some(notification) = notifications.next().await {
            if let ClientNotification::Event { event, .. } = notification {
                if event.kind == nostr::event::Kind::GiftWrap {
                    self.handle_event(&event).await;
                }
            }
        }
        Ok(())
    }

    async fn handle_event(&self, event: &Event) {
        let gift = match UnwrappedGift::from_gift_wrap(&self.keys, event) {
            Ok(g) => g,
            Err(e) => {
                // Expected noise: spam wraps addressed to us. Never log content.
                tracing::debug!("discarded unwrappable gift wrap: {e}");
                return;
            }
        };
        let sender = gift.sender.to_hex();
        if gift.rumor.kind != nostr::event::Kind::Custom(KIND_REQUEST) {
            tracing::debug!(sender = %sender, kind = %gift.rumor.kind, "discarded non-request rumor");
            return;
        }

        let response = match parse_request(&gift.rumor.content) {
            Ok(env) => self.route(&sender, &env).await,
            Err(Some(err_response)) => Some(err_response),
            Err(None) => None,
        };

        if let Some(resp) = response {
            if let Err(e) = self.send_response(&gift.sender, &resp).await {
                tracing::warn!(sender = %sender, "failed to send response: {e}");
            }
        }
    }

    /// Pure routing: returns the response to send, or None to drop.
    /// Unit-tested without any network.
    async fn route(&self, sender_hex: &str, env: &RequestEnvelope) -> Option<ResponseEnvelope> {
        let now = now_secs();
        if !self.replay.check_and_insert(&env.id, now) {
            tracing::debug!(sender = %sender_hex, "dropped replayed envelope id");
            return None;
        }
        if !self.ratelimit.allow(sender_hex, now) {
            return Some(ResponseEnvelope::err(
                &env.id,
                ErrorCode::RateLimited,
                "slow down",
            ));
        }
        Some(self.dispatch(sender_hex, env, now).await)
    }

    async fn dispatch(&self, sender_hex: &str, env: &RequestEnvelope, now: u64) -> ResponseEnvelope {
        // `pair` is the only message type open to non-allowlisted keys.
        if env.msg_type == "pair" {
            return self.handle_pair(sender_hex, env, now);
        }
        if !self.allowlist.is_paired(sender_hex) {
            return ResponseEnvelope::err(&env.id, ErrorCode::NotPaired, "not paired");
        }
        match env.msg_type.as_str() {
            "unpair" => self.handle_unpair(sender_hex, env),
            t if KNOWN_TYPES.contains(&t) => self.handlers.handle(sender_hex, env).await,
            _ => ResponseEnvelope::err(&env.id, ErrorCode::UnknownType, "unknown type"),
        }
    }

    fn handle_pair(&self, sender_hex: &str, env: &RequestEnvelope, now: u64) -> ResponseEnvelope {
        // Already-paired keys get no second chance and don't burn attempts.
        if self.allowlist.is_paired(sender_hex) {
            return ResponseEnvelope::err(&env.id, ErrorCode::PairingFailed, "pairing failed");
        }
        let proof = env
            .payload
            .get("proof")
            .and_then(|p| p.as_str())
            .unwrap_or("");
        let ok = self
            .pairing
            .lock()
            .unwrap()
            .verify_at(now, sender_hex, proof);
        if !ok {
            tracing::info!(sender = %sender_hex, "pairing attempt failed");
            return ResponseEnvelope::err(&env.id, ErrorCode::PairingFailed, "pairing failed");
        }
        if let Err(e) = self.allowlist.add(sender_hex) {
            tracing::error!("failed to persist pairing: {e}");
            return ResponseEnvelope::err(&env.id, ErrorCode::Internal, "internal error");
        }
        let client = env
            .payload
            .get("client")
            .and_then(|c| c.as_str())
            .unwrap_or("unknown");
        tracing::info!(sender = %sender_hex, client = %client, "wallet paired");
        ResponseEnvelope::ok(
            &env.id,
            serde_json::json!({ "server": SERVER_VERSION }),
        )
    }

    fn handle_unpair(&self, sender_hex: &str, env: &RequestEnvelope) -> ResponseEnvelope {
        match self.allowlist.remove(sender_hex) {
            Ok(true) => {
                if let Err(e) = self.watch.remove_wallet(sender_hex) {
                    tracing::error!("failed to clear watch set on unpair: {e}");
                }
                tracing::info!(sender = %sender_hex, "wallet unpaired");
                ResponseEnvelope::ok(&env.id, serde_json::json!({}))
            }
            Ok(false) => ResponseEnvelope::err(&env.id, ErrorCode::NotPaired, "not paired"),
            Err(e) => {
                tracing::error!("failed to persist unpair: {e}");
                ResponseEnvelope::err(&env.id, ErrorCode::Internal, "internal error")
            }
        }
    }

    async fn send_response(
        &self,
        recipient: &nostr::key::PublicKey,
        resp: &ResponseEnvelope,
    ) -> anyhow::Result<()> {
        let content = serde_json::to_string(resp)?;
        let rumor =
            EventBuilder::new(nostr::event::Kind::Custom(KIND_RESPONSE), content)
                .finalize_unsigned(self.keys.public_key());
        let wrap = GiftWrapBuilder::new(*recipient, rumor)
            .expiration(WRAP_EXPIRATION)
            .finalize(&self.keys)?;
        let output = self.client.send_event(&wrap).await?;
        if output.failed.is_empty() {
            tracing::debug!(
                ok = output.success.len(),
                "response published to {} relay(s)",
                output.success.len()
            );
        } else {
            tracing::warn!(
                ok = output.success.len(),
                failed = ?output.failed,
                "response publish had relay failures"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Transport, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let watch = Arc::new(WatchStore::load(dir.path()).unwrap());
        let handlers = Arc::new(Handlers::new(
            Arc::new(crate::electrs::Electrs::new("127.0.0.1:9")), // instant refusal
            crate::config::Network::Regtest,
            watch.clone(),
        ));
        let t = Transport::new(
            Keys::generate(),
            vec![],
            Arc::new(Allowlist::load(dir.path()).unwrap()),
            watch,
            Arc::new(Mutex::new(PairingManager::new())),
            ReplayCache::load(dir.path()).unwrap(),
            handlers,
        );
        (t, dir)
    }

    fn envelope(id: &str, msg_type: &str, payload: serde_json::Value) -> RequestEnvelope {
        RequestEnvelope {
            v: 1,
            id: id.to_string(),
            ts: 1,
            msg_type: msg_type.to_string(),
            payload,
        }
    }

    fn uuid(i: u128) -> String {
        uuid::Uuid::from_u128(i).to_string()
    }

    fn make_proof(t: &Transport, wallet: &str) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        let payload = t
            .pairing
            .lock()
            .unwrap()
            .payload_at(now_secs(), "nodepk", &[]);
        let secret = base64::engine::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            &payload.pair_secret,
        )
        .unwrap();
        let mut mac = Hmac::<Sha256>::new_from_slice(&secret).unwrap();
        mac.update(wallet.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    #[tokio::test]
    async fn pair_then_ping_then_unpair() {
        let (t, _dir) = setup();
        let wallet = Keys::generate().public_key().to_hex();

        // unknown type from stranger: not_paired beats unknown_type
        let r = t.route(&wallet, &envelope(&uuid(1), "bogus", serde_json::json!({}))).await;
        assert_eq!(r.unwrap().error.unwrap().code, ErrorCode::NotPaired);

        // pair with the correct proof
        let proof = make_proof(&t, &wallet);
        let r = t.route(&wallet, &envelope(&uuid(2), "pair", serde_json::json!({"proof": proof}))).await;
        assert!(r.unwrap().ok);
        assert!(t.allowlist.is_paired(&wallet));

        // known type passes authz; dummy backend fast-fails
        let r = t.route(&wallet, &envelope(&uuid(3), "get_balance", serde_json::json!({"addresses":[]}))).await;
        assert_eq!(r.unwrap().error.unwrap().code, ErrorCode::InvalidRequest);

        // unknown type from paired wallet
        let r = t.route(&wallet, &envelope(&uuid(4), "bogus", serde_json::json!({}))).await;
        assert_eq!(r.unwrap().error.unwrap().code, ErrorCode::UnknownType);

        // replayed id is dropped
        let r = t.route(&wallet, &envelope(&uuid(4), "bogus", serde_json::json!({}))).await;
        assert!(r.is_none());

        // unpair works, and a second unpair is not_paired
        let r = t.route(&wallet, &envelope(&uuid(5), "unpair", serde_json::json!({}))).await;
        assert!(r.unwrap().ok);
        let r = t.route(&wallet, &envelope(&uuid(6), "unpair", serde_json::json!({}))).await;
        assert_eq!(r.unwrap().error.unwrap().code, ErrorCode::NotPaired);
    }

    #[tokio::test]
    async fn pair_rejects_bad_proof_and_second_pairing() {
        let (t, _dir) = setup();
        let wallet = Keys::generate().public_key().to_hex();
        let r = t.route(
            &wallet,
            &envelope(&uuid(1), "pair", serde_json::json!({"proof": "00"})),
        ).await;
        assert_eq!(r.unwrap().error.unwrap().code, ErrorCode::PairingFailed);
        assert!(!t.allowlist.is_paired(&wallet));

        let proof = make_proof(&t, &wallet);
        assert!(t
            .route(&wallet, &envelope(&uuid(2), "pair", serde_json::json!({"proof": proof})))
            .await
            .unwrap()
            .ok);
        // already paired: rejected even with a fresh-looking attempt
        let r = t.route(
            &wallet,
            &envelope(&uuid(3), "pair", serde_json::json!({"proof": "00"})),
        ).await;
        assert_eq!(r.unwrap().error.unwrap().code, ErrorCode::PairingFailed);
    }

    #[test]
    fn gift_wrap_roundtrip_offline() {
        let server = Keys::generate();
        let wallet = Keys::generate();
        let resp = ResponseEnvelope::ok(&uuid(9), serde_json::json!({"server": SERVER_VERSION}));
        let rumor = EventBuilder::new(
            nostr::event::Kind::Custom(KIND_RESPONSE),
            serde_json::to_string(&resp).unwrap(),
        )
        .finalize_unsigned(server.public_key());
        let wrap = GiftWrapBuilder::new(wallet.public_key(), rumor)
            .expiration(WRAP_EXPIRATION)
            .finalize(&server)
            .unwrap();
        let gift = UnwrappedGift::from_gift_wrap(&wallet, &wrap).unwrap();
        assert_eq!(gift.sender, server.public_key());
        assert_eq!(gift.rumor.kind, nostr::event::Kind::Custom(KIND_RESPONSE));
        let parsed: ResponseEnvelope = serde_json::from_str(&gift.rumor.content).unwrap();
        assert!(parsed.ok);
    }
}
