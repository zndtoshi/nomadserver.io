//! Replay protection: seen-envelope-id cache (PROTOCOL.md §2.4).
//!
//! Offline delivery means legitimate messages can arrive days late, so
//! timestamps can't be the replay defense — envelope ids are. Duplicates
//! are dropped. The cache is file-persisted so a server restart doesn't
//! reopen the window, and entries are pruned after RETENTION_SECS
//! (gift wraps expire well before that, so a pruned id can't be replayed
//! from relays; a manually re-published ancient wrap is harmless because
//! all message types are idempotent).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const FILE: &str = "seen_ids.json";
/// 10 days: outlives the gift-wrap expiration (9 days incl. timestamp
/// tweak) with margin.
pub const RETENTION_SECS: u64 = 10 * 24 * 3600;

pub struct ReplayCache {
    path: PathBuf,
    seen: Mutex<BTreeMap<String, u64>>, // id -> first-seen unix ts
}

impl ReplayCache {
    pub fn load(data_dir: &Path) -> anyhow::Result<Self> {
        let path = data_dir.join(FILE);
        let seen: BTreeMap<String, u64> = match fs::read_to_string(&path) {
            Ok(data) => serde_json::from_str(&data)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            path,
            // stale entries are pruned lazily on the next check_and_insert
            seen: Mutex::new(seen),
        })
    }

    /// Returns true if the id is NEW (and records it); false if it's a
    /// replay and must be dropped.
    pub fn check_and_insert(&self, id: &str, now: u64) -> bool {
        let mut seen = self.seen.lock().unwrap();
        seen.retain(|_, ts| now.saturating_sub(*ts) < RETENTION_SECS);
        if seen.contains_key(id) {
            return false;
        }
        seen.insert(id.to_string(), now);
        // Persist best-effort: a failed write loses restart-protection
        // only; correctness of the in-memory cache is unaffected.
        if let Ok(data) = serde_json::to_string(&*seen) {
            let tmp = self.path.with_extension("tmp");
            if fs::write(&tmp, data).is_ok() {
                let _ = fs::rename(&tmp, &self.path);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_replays_and_prunes() {
        let dir = tempfile::tempdir().unwrap();
        let c = ReplayCache::load(dir.path()).unwrap();
        let now = 1_000_000u64;
        assert!(c.check_and_insert("id-a", now));
        assert!(!c.check_and_insert("id-a", now + 1));
        assert!(c.check_and_insert("id-b", now + 2));
        // after retention, old ids are pruned (and message idempotency
        // makes this safe)
        assert!(c.check_and_insert("id-a", now + RETENTION_SECS + 1));
    }

    #[test]
    fn persists_across_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let c = ReplayCache::load(dir.path()).unwrap();
        assert!(c.check_and_insert("id-x", 1_000_000));
        drop(c);
        let c2 = ReplayCache::load(dir.path()).unwrap();
        assert!(!c2.check_and_insert("id-x", 1_000_001));
    }
}
