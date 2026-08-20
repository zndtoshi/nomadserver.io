//! Pairing secret lifecycle and proof verification (PROTOCOL.md §4).
//!
//! The secret is the one-time authorization inside the pairing QR. It lives
//! only in memory, is valid for one successful pairing or 10 minutes, and
//! is burned after 5 failed attempts. Verification is HMAC-SHA-256 with a
//! constant-time comparison. The secret is never logged.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::Serialize;
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

pub const SECRET_TTL_SECS: u64 = 600;
pub const MAX_ATTEMPTS: u8 = 5;

#[derive(Debug, Clone, Serialize)]
pub struct PairingPayload {
    pub v: u8,
    pub app: &'static str,
    #[serde(rename = "nodePubkey")]
    pub node_pubkey: String,
    pub relays: Vec<String>,
    #[serde(rename = "pairSecret")]
    pub pair_secret: String,
    pub exp: u64,
}

struct SecretState {
    secret: [u8; 32],
    expires_at: u64,
    used: bool,
    attempts: u8,
}

#[derive(Default)]
pub struct PairingManager {
    state: Option<SecretState>,
}

impl PairingManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// The payload to display in the QR / serve at /pairing. Generates a
    /// fresh secret if there is none, or the current one expired or was
    /// used; otherwise re-shows the live one (the UI polls).
    pub fn payload_at(
        &mut self,
        now: u64,
        node_pubkey: &str,
        relays: &[String],
    ) -> PairingPayload {
        let live = self
            .state
            .as_ref()
            .is_some_and(|s| !s.used && s.expires_at > now);
        if !live {
            let mut secret = [0u8; 32];
            rand::rng().fill_bytes(&mut secret);
            self.state = Some(SecretState {
                secret,
                expires_at: now + SECRET_TTL_SECS,
                used: false,
                attempts: 0,
            });
        }
        let s = self.state.as_ref().expect("state just set");
        PairingPayload {
            v: 1,
            app: "nomad-server",
            node_pubkey: node_pubkey.to_string(),
            relays: relays.to_vec(),
            pair_secret: URL_SAFE_NO_PAD.encode(s.secret),
            exp: s.expires_at,
        }
    }

    /// Verify a wallet's pairing proof. Returns true exactly once per
    /// secret: on success the secret is burned. Failures count toward
    /// MAX_ATTEMPTS, after which the secret is burned and the user must
    /// reload the QR. Gives no indication of which check failed.
    pub fn verify_at(&mut self, now: u64, wallet_pubkey_hex: &str, proof_hex: &str) -> bool {
        let Some(s) = self.state.as_mut() else {
            return false;
        };
        if s.used || s.expires_at <= now || s.attempts >= MAX_ATTEMPTS {
            return false;
        }

        let mut mac = HmacSha256::new_from_slice(&s.secret).expect("HMAC accepts any key length");
        mac.update(wallet_pubkey_hex.as_bytes());
        let expected = mac.finalize().into_bytes();

        let ok = hex::decode(proof_hex)
            .map(|proof| proof.as_slice().ct_eq(expected.as_slice()).into())
            .unwrap_or(false);

        if ok {
            s.used = true;
        } else {
            s.attempts += 1;
            if s.attempts >= MAX_ATTEMPTS {
                self.state = None;
            }
        }
        ok
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proof_for(secret_b64: &str, pubkey: &str) -> String {
        let secret = URL_SAFE_NO_PAD.decode(secret_b64).unwrap();
        let mut mac = HmacSha256::new_from_slice(&secret).unwrap();
        mac.update(pubkey.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    #[test]
    fn happy_path_single_use() {
        let mut pm = PairingManager::new();
        let p = pm.payload_at(1000, "nodepk", &["wss://r".into()]);
        assert_eq!(p.v, 1);
        assert_eq!(p.exp, 1000 + SECRET_TTL_SECS);

        // same live secret is re-shown
        let p2 = pm.payload_at(1001, "nodepk", &["wss://r".into()]);
        assert_eq!(p.pair_secret, p2.pair_secret);

        let proof = proof_for(&p.pair_secret, "walletpk");
        assert!(pm.verify_at(1002, "walletpk", &proof));
        // single-use: a second success with the same proof fails
        assert!(!pm.verify_at(1003, "walletpk", &proof));
        // next payload carries a fresh secret
        let p3 = pm.payload_at(1004, "nodepk", &["wss://r".into()]);
        assert_ne!(p.pair_secret, p3.pair_secret);
    }

    #[test]
    fn wrong_proof_and_expiry() {
        let mut pm = PairingManager::new();
        let _p = pm.payload_at(1000, "nodepk", &[]);
        assert!(!pm.verify_at(1001, "walletpk", "00".repeat(32).as_str()));
        // expired
        assert!(!pm.verify_at(1000 + SECRET_TTL_SECS, "walletpk", "00".repeat(32).as_str()));
    }

    #[test]
    fn burns_after_max_attempts() {
        let mut pm = PairingManager::new();
        let p = pm.payload_at(1000, "nodepk", &[]);
        for _ in 0..MAX_ATTEMPTS {
            assert!(!pm.verify_at(1001, "walletpk", "ff".repeat(32).as_str()));
        }
        // secret is gone: even the correct proof now fails
        let proof = proof_for(&p.pair_secret, "walletpk");
        assert!(!pm.verify_at(1002, "walletpk", &proof));
    }
}
