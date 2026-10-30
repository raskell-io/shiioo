//! Secrets for the office, encrypted at rest.
//!
//! Two files in `.shiioo/`: `secrets.key`, a random 256-bit key (hex, mode
//! 0600) made on first use, and `secrets.enc`, the name → value map as
//! JSON sealed with XChaCha20-Poly1305 under that key (a fresh random
//! nonce per write, stored in front of the ciphertext). Both are written
//! through a temporary file and a rename, so a crash leaves the old
//! version, never half of a new one.
//!
//! This keeps values out of config files, logs and backups of the data
//! file; it does not protect against someone who can read both files as
//! the owner's user (`docs/design/security.md` §3.2). The planned envelope
//! encryption (an `age` identity or a KMS wrapping the key) replaces the
//! key file, not the data format.

use chacha20poly1305::aead::{Aead, KeyInit, OsRng, rand_core::RngCore};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const KEY_FILE: &str = "secrets.key";
const DATA_FILE: &str = "secrets.enc";
const NONCE_LEN: usize = 24;

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{0} is corrupt (wrong key, or edited)")]
    Corrupt(PathBuf),
    #[error("invalid secret name {0:?}: use lower-case letters, digits, '.', '_' and '-'")]
    Name(String),
}

pub struct SecretStore {
    dir: PathBuf,
    cipher: XChaCha20Poly1305,
}

impl SecretStore {
    /// Opens the store in `dir` (which must exist), creating the key on
    /// first use.
    pub fn open(dir: &Path) -> Result<Self, SecretError> {
        let key_path = dir.join(KEY_FILE);
        let key: [u8; 32] = match std::fs::read_to_string(&key_path) {
            Ok(hex_key) => {
                let mut key = [0u8; 32];
                hex::decode_to_slice(hex_key.trim(), &mut key)
                    .map_err(|_| SecretError::Corrupt(key_path.clone()))?;
                key
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut key = [0u8; 32];
                OsRng.fill_bytes(&mut key);
                write_private(&key_path, hex::encode(key).as_bytes())?;
                key
            }
            Err(source) => {
                return Err(SecretError::Io {
                    path: key_path,
                    source,
                });
            }
        };
        Ok(Self {
            dir: dir.to_path_buf(),
            cipher: XChaCha20Poly1305::new((&key).into()),
        })
    }

    pub fn set(&self, name: &str, value: &str) -> Result<(), SecretError> {
        validate(name)?;
        let mut map = self.read_map()?;
        map.insert(name.to_owned(), value.to_owned());
        self.write_map(&map)
    }

    pub fn get(&self, name: &str) -> Result<Option<String>, SecretError> {
        Ok(self.read_map()?.remove(name))
    }

    /// Removes a secret; `Ok(false)` if there was none.
    pub fn remove(&self, name: &str) -> Result<bool, SecretError> {
        let mut map = self.read_map()?;
        let existed = map.remove(name).is_some();
        if existed {
            self.write_map(&map)?;
        }
        Ok(existed)
    }

    pub fn names(&self) -> Result<Vec<String>, SecretError> {
        Ok(self.read_map()?.into_keys().collect())
    }

    /// Every secret, for redaction.
    pub fn all(&self) -> Result<Vec<(String, String)>, SecretError> {
        Ok(self.read_map()?.into_iter().collect())
    }

    fn data_path(&self) -> PathBuf {
        self.dir.join(DATA_FILE)
    }

    fn read_map(&self) -> Result<BTreeMap<String, String>, SecretError> {
        let path = self.data_path();
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(source) => return Err(SecretError::Io { path, source }),
        };
        if bytes.len() < NONCE_LEN {
            return Err(SecretError::Corrupt(path));
        }
        let (nonce, ciphertext) = bytes.split_at(NONCE_LEN);
        let plain = self
            .cipher
            .decrypt(XNonce::from_slice(nonce), ciphertext)
            .map_err(|_| SecretError::Corrupt(path.clone()))?;
        serde_json::from_slice(&plain).map_err(|_| SecretError::Corrupt(path))
    }

    fn write_map(&self, map: &BTreeMap<String, String>) -> Result<(), SecretError> {
        let path = self.data_path();
        let plain = serde_json::to_vec(map).map_err(|_| SecretError::Corrupt(path.clone()))?;
        let mut nonce = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);
        let ciphertext = self
            .cipher
            .encrypt(XNonce::from_slice(&nonce), plain.as_slice())
            .map_err(|_| SecretError::Corrupt(path.clone()))?;
        let mut bytes = nonce.to_vec();
        bytes.extend(ciphertext);
        write_private(&path, &bytes)
    }
}

/// Lower-case letters, digits, `.`, `_` and `-`, starting with a letter or
/// digit: safe in `secret:<name>` references, file names and shell lines.
pub fn validate(name: &str) -> Result<(), SecretError> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        && name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    if ok {
        Ok(())
    } else {
        Err(SecretError::Name(name.to_owned()))
    }
}

/// Writes `bytes` to `path` with mode 0600, through a temporary file in
/// the same directory and a rename.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), SecretError> {
    let io = |source| SecretError::Io {
        path: path.to_path_buf(),
        source,
    };
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).map_err(io)?;
    }
    std::fs::rename(&tmp, path).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_persists_across_opens() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let store = SecretStore::open(dir.path()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            store.get("gh-token").unwrap_or_else(|e| panic!("{e}")),
            None
        );
        store
            .set("gh-token", "ghp_secret")
            .unwrap_or_else(|e| panic!("{e}"));
        store
            .set("claude-code-oauth-token", "sk-ant-oat01-x")
            .unwrap_or_else(|e| panic!("{e}"));

        let again = SecretStore::open(dir.path()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            again.get("gh-token").unwrap_or_else(|e| panic!("{e}")),
            Some("ghp_secret".to_owned())
        );
        assert_eq!(
            again.names().unwrap_or_else(|e| panic!("{e}")),
            vec!["claude-code-oauth-token".to_owned(), "gh-token".to_owned()]
        );
        assert!(again.remove("gh-token").unwrap_or_else(|e| panic!("{e}")));
        assert!(!again.remove("gh-token").unwrap_or_else(|e| panic!("{e}")));
        assert_eq!(again.all().unwrap_or_else(|e| panic!("{e}")).len(), 1);
    }

    #[test]
    fn files_are_private_and_the_value_is_not_in_the_clear() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let store = SecretStore::open(dir.path()).unwrap_or_else(|e| panic!("{e}"));
        store
            .set("pw", "hunter2hunter2")
            .unwrap_or_else(|e| panic!("{e}"));
        let data = std::fs::read(dir.path().join(DATA_FILE)).unwrap_or_else(|e| panic!("{e}"));
        assert!(!data.windows(14).any(|w| w == b"hunter2hunter2"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for file in [KEY_FILE, DATA_FILE] {
                let mode = std::fs::metadata(dir.path().join(file))
                    .unwrap_or_else(|e| panic!("{e}"))
                    .permissions()
                    .mode();
                assert_eq!(mode & 0o777, 0o600, "{file}");
            }
        }
    }

    #[test]
    fn a_different_key_cannot_read_the_data() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let store = SecretStore::open(dir.path()).unwrap_or_else(|e| panic!("{e}"));
        store.set("a", "value1").unwrap_or_else(|e| panic!("{e}"));
        std::fs::remove_file(dir.path().join(KEY_FILE)).unwrap_or_else(|e| panic!("{e}"));
        let other = SecretStore::open(dir.path()).unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(other.get("a"), Err(SecretError::Corrupt(_))));
    }

    #[test]
    fn names_are_validated() {
        assert!(validate("gh-token").is_ok());
        assert!(validate("claude.oauth_1").is_ok());
        assert!(validate("").is_err());
        assert!(validate("GH").is_err());
        assert!(validate("-x").is_err());
        assert!(validate("a b").is_err());
    }
}
