//! Persistent stores: the wallet allowlist and per-wallet watch sets.
//!
//! Both are small JSON files written atomically (tmp + rename) with mode
//! 0600. They contain no secrets, but they are privacy-sensitive (wallet
//! pubkeys, watched addresses) — treat them accordingly. Watch sets use
//! replace semantics (PROTOCOL.md §5.8): a wallet re-asserts its full set
//! after every sync, so loss of this file is self-healing.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const ALLOWLIST_FILE: &str = "paired_wallets.json";
const WATCHSETS_FILE: &str = "watch_sets.json";

fn write_json_atomic<T: serde::Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_string_pretty(value)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Wallet pubkeys authorized to use this server (PROTOCOL.md §4).
pub struct Allowlist {
    path: PathBuf,
    wallets: Mutex<BTreeSet<String>>,
}

impl Allowlist {
    pub fn load(data_dir: &Path) -> anyhow::Result<Self> {
        let path = data_dir.join(ALLOWLIST_FILE);
        let wallets = match fs::read_to_string(&path) {
            Ok(data) => serde_json::from_str(&data)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            path,
            wallets: Mutex::new(wallets),
        })
    }

    pub fn is_paired(&self, pubkey_hex: &str) -> bool {
        self.wallets.lock().unwrap().contains(pubkey_hex)
    }

    pub fn add(&self, pubkey_hex: &str) -> anyhow::Result<()> {
        let mut w = self.wallets.lock().unwrap();
        w.insert(pubkey_hex.to_string());
        write_json_atomic(&self.path, &*w)
    }

    pub fn remove(&self, pubkey_hex: &str) -> anyhow::Result<bool> {
        let mut w = self.wallets.lock().unwrap();
        let removed = w.remove(pubkey_hex);
        if removed {
            write_json_atomic(&self.path, &*w)?;
        }
        Ok(removed)
    }

    pub fn list(&self) -> Vec<String> {
        self.wallets.lock().unwrap().iter().cloned().collect()
    }
}

/// Per-wallet address watch sets for the watcher (PROTOCOL.md §5.8).
pub struct WatchStore {
    path: PathBuf,
    sets: Mutex<BTreeMap<String, Vec<String>>>,
}

// `replace`/`snapshot` are used by the watch_addresses handler and the
// watcher (next phases); exercised by unit tests meanwhile.
#[allow(dead_code)]
impl WatchStore {
    pub fn load(data_dir: &Path) -> anyhow::Result<Self> {
        let path = data_dir.join(WATCHSETS_FILE);
        let sets = match fs::read_to_string(&path) {
            Ok(data) => serde_json::from_str(&data)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            path,
            sets: Mutex::new(sets),
        })
    }

    /// Replace the wallet's entire watch set. Empty vector clears it.
    pub fn replace(&self, pubkey_hex: &str, addresses: Vec<String>) -> anyhow::Result<usize> {
        let mut s = self.sets.lock().unwrap();
        let len = addresses.len();
        if addresses.is_empty() {
            s.remove(pubkey_hex);
        } else {
            s.insert(pubkey_hex.to_string(), addresses);
        }
        write_json_atomic(&self.path, &*s)?;
        Ok(len)
    }

    pub fn remove_wallet(&self, pubkey_hex: &str) -> anyhow::Result<()> {
        let mut s = self.sets.lock().unwrap();
        if s.remove(pubkey_hex).is_some() {
            write_json_atomic(&self.path, &*s)?;
        }
        Ok(())
    }

    /// Snapshot of every watched address with its owner, for the watcher.
    pub fn snapshot(&self) -> BTreeMap<String, Vec<String>> {
        self.sets.lock().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let a = Allowlist::load(dir.path()).unwrap();
        assert!(!a.is_paired("aa"));
        a.add("aa").unwrap();
        a.add("bb").unwrap();
        assert!(a.is_paired("aa"));
        assert_eq!(a.list(), vec!["aa".to_string(), "bb".to_string()]);

        let reloaded = Allowlist::load(dir.path()).unwrap();
        assert!(reloaded.is_paired("bb"));
        assert!(reloaded.remove("aa").unwrap());
        assert!(!reloaded.is_paired("aa"));
        assert!(!reloaded.remove("aa").unwrap());
    }

    #[test]
    fn watchset_replace_semantics() {
        let dir = tempfile::tempdir().unwrap();
        let w = WatchStore::load(dir.path()).unwrap();
        w.replace("pk", vec!["addr1".into(), "addr2".into()]).unwrap();
        assert_eq!(w.snapshot()["pk"].len(), 2);
        // replace, not append
        w.replace("pk", vec!["addr3".into()]).unwrap();
        assert_eq!(w.snapshot()["pk"], vec!["addr3".to_string()]);
        // empty clears
        w.replace("pk", vec![]).unwrap();
        assert!(!w.snapshot().contains_key("pk"));
    }
}
