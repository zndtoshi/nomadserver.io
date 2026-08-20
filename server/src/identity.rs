//! Server Nostr identity: one long-lived keypair, the server's only secret.
//!
//! Stored as raw secret-key bytes, hex-encoded, file mode 0600. The secret
//! never leaves this module in loggable form — only the pubkey is exposed
//! to the rest of the app.

use std::fs;
use std::io;
use std::path::Path;

use nostr::key::Keys;

const KEY_FILE: &str = "nostr_secret.hex";

/// Load the server keypair from `<data_dir>/nostr_secret.hex`, creating it
/// with a fresh CSPRNG key on first run. Creates `data_dir` (mode 0700) if
/// missing. New key files are written with mode 0600.
pub fn load_or_create(data_dir: &Path) -> anyhow::Result<Keys> {
    fs::create_dir_all(data_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(data_dir, fs::Permissions::from_mode(0o700))?;
    }

    let path = data_dir.join(KEY_FILE);
    match fs::read_to_string(&path) {
        Ok(hex) => {
            let keys = Keys::parse(hex.trim())
                .map_err(|e| anyhow::anyhow!("corrupt key file {}: {e}", path.display()))?;
            Ok(keys)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let keys = Keys::generate();
            write_secret(&path, &keys)?;
            tracing::info!(
                "generated new server identity; pubkey={}",
                keys.public_key().to_hex()
            );
            Ok(keys)
        }
        Err(e) => Err(e.into()),
    }
}

#[cfg(unix)]
fn write_secret(path: &Path, keys: &Keys) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(hex::encode(keys.secret_key().secret_bytes()).as_bytes())?;
    Ok(())
}

#[cfg(not(unix))]
fn write_secret(path: &Path, keys: &Keys) -> anyhow::Result<()> {
    fs::write(path, hex::encode(keys.secret_key().secret_bytes()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_reloads_same_identity() {
        let dir = tempfile::tempdir().unwrap();
        let k1 = load_or_create(dir.path()).unwrap();
        let k2 = load_or_create(dir.path()).unwrap();
        assert_eq!(k1.public_key(), k2.public_key());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.path().join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
