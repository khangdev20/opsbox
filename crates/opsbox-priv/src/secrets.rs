//! The master key and sealed secrets (docs/BACKEND_SPEC.md sections 4 and 5).
//!
//! The key lives in a root-only file that only `priv` reads, so a copy of the
//! database alone reveals no secret. Sealed values look like
//! `enc:v1:<nonce>:<ciphertext>` (base64url, XChaCha20-Poly1305). The `context` is
//! authenticated but not stored, so a sealed value cannot be moved to another row.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use chacha20poly1305::aead::Payload;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce, aead::Aead};
use zeroize::Zeroizing;

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;
const PREFIX: &str = "enc:v1:";

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("master key file {path}: {reason}")]
    Key { path: String, reason: String },
    #[error("random number generator failed")]
    Random,
    #[error("sealed value is malformed or was sealed for something else")]
    Open,
}

/// Loads the master key on first use, creating it if the file does not exist.
pub struct Secrets {
    path: PathBuf,
    key: Mutex<Option<Zeroizing<[u8; KEY_LEN]>>>,
}

impl Secrets {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            key: Mutex::new(None),
        }
    }

    fn with_key<T>(&self, f: impl FnOnce(&[u8; KEY_LEN]) -> T) -> Result<T, SecretError> {
        let mut guard = self
            .key
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.is_none() {
            *guard = Some(load_or_create(&self.path)?);
        }
        Ok(f(guard.as_ref().expect("just loaded")))
    }

    pub fn seal(&self, plaintext: &[u8], context: &str) -> Result<String, SecretError> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce_bytes).map_err(|_| SecretError::Random)?;
        let nonce = XNonce::try_from(&nonce_bytes[..]).map_err(|_| SecretError::Random)?;
        let ct = self.with_key(|key| {
            let cipher = XChaCha20Poly1305::new(&(*key).into());
            cipher.encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: context.as_bytes(),
                },
            )
        })?;
        let ct = ct.map_err(|_| SecretError::Open)?;
        Ok(format!(
            "{PREFIX}{}:{}",
            B64.encode(nonce_bytes),
            B64.encode(ct)
        ))
    }

    pub fn open(&self, sealed: &str, context: &str) -> Result<Zeroizing<Vec<u8>>, SecretError> {
        let rest = sealed.strip_prefix(PREFIX).ok_or(SecretError::Open)?;
        let (n, c) = rest.split_once(':').ok_or(SecretError::Open)?;
        let nonce_bytes = B64.decode(n).map_err(|_| SecretError::Open)?;
        let ct = B64.decode(c).map_err(|_| SecretError::Open)?;
        let nonce = XNonce::try_from(&nonce_bytes[..]).map_err(|_| SecretError::Open)?;
        let plain = self.with_key(|key| {
            let cipher = XChaCha20Poly1305::new(&(*key).into());
            cipher.decrypt(
                &nonce,
                Payload {
                    msg: &ct,
                    aad: context.as_bytes(),
                },
            )
        })?;
        plain.map(Zeroizing::new).map_err(|_| SecretError::Open)
    }

    /// Derive the deterministic restic repository password from the 32-byte master key.
    pub fn derive_backup_password(&self) -> Result<Zeroizing<String>, SecretError> {
        self.with_key(opsbox_backup::derive_repo_password)
    }
}

fn key_err(path: &Path, reason: impl Into<String>) -> SecretError {
    SecretError::Key {
        path: path.display().to_string(),
        reason: reason.into(),
    }
}

fn load_or_create(path: &Path) -> Result<Zeroizing<[u8; KEY_LEN]>, SecretError> {
    match std::fs::read(path) {
        Ok(bytes) => {
            check_mode(path)?;
            let bytes = Zeroizing::new(bytes);
            let key: [u8; KEY_LEN] = bytes[..]
                .try_into()
                .map_err(|_| key_err(path, format!("must be exactly {KEY_LEN} bytes")))?;
            Ok(Zeroizing::new(key))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => create(path),
        Err(e) => Err(key_err(path, e.to_string())),
    }
}

fn create(path: &Path) -> Result<Zeroizing<[u8; KEY_LEN]>, SecretError> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| key_err(path, e.to_string()))?;
    }
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    getrandom::fill(&mut key[..]).map_err(|_| SecretError::Random)?;

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    match opts.open(path) {
        Ok(mut f) => {
            f.write_all(&key[..])
                .and_then(|()| f.sync_all())
                .map_err(|e| key_err(path, e.to_string()))?;
            Ok(key)
        }
        // Another process created it between our read and our create: use theirs.
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => load_or_create(path),
        Err(e) => Err(key_err(path, e.to_string())),
    }
}

fn check_mode(path: &Path) -> Result<(), SecretError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)
        .map_err(|e| key_err(path, e.to_string()))?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        return Err(key_err(
            path,
            format!(
                "mode is {:04o}; it must not be accessible to group or others (use 0600)",
                mode & 0o777
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn secrets() -> (tempfile::TempDir, Secrets) {
        let dir = tempfile::tempdir().unwrap();
        let s = Secrets::new(dir.path().join("etc").join("master.key"));
        (dir, s)
    }

    #[test]
    fn seal_and_open_round_trip() {
        let (_d, s) = secrets();
        let sealed = s.seal(b"hunter2", "totp:user-1").unwrap();
        assert!(sealed.starts_with("enc:v1:"));
        assert!(!sealed.contains("hunter2"));
        assert_eq!(&s.open(&sealed, "totp:user-1").unwrap()[..], b"hunter2");
    }

    #[test]
    fn same_plaintext_seals_differently_each_time() {
        let (_d, s) = secrets();
        assert_ne!(s.seal(b"x", "c").unwrap(), s.seal(b"x", "c").unwrap());
    }

    #[test]
    fn a_sealed_value_cannot_be_moved_to_another_context_or_tampered_with() {
        let (_d, s) = secrets();
        let sealed = s.seal(b"secret", "totp:alice").unwrap();
        assert!(s.open(&sealed, "totp:mallory").is_err());
        let mut bad = sealed.clone();
        bad.push('A');
        assert!(s.open(&bad, "totp:alice").is_err());
        for junk in ["", "enc:v1:", "enc:v1:abc", "enc:v2:a:b", "plain"] {
            assert!(s.open(junk, "c").is_err(), "{junk}");
        }
    }

    #[test]
    fn key_is_created_private_and_reused() {
        let (dir, s) = secrets();
        let sealed = s.seal(b"v", "c").unwrap();
        let path = dir.path().join("etc").join("master.key");
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.len(), 32);
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        // A fresh instance reads the same key.
        let again = Secrets::new(&path);
        assert_eq!(&again.open(&sealed, "c").unwrap()[..], b"v");
    }

    #[test]
    fn a_key_file_that_others_can_read_is_refused() {
        let (dir, s) = secrets();
        s.seal(b"v", "c").unwrap();
        let path = dir.path().join("etc").join("master.key");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let fresh = Secrets::new(&path);
        let err = fresh.seal(b"v", "c").unwrap_err().to_string();
        assert!(err.contains("0644"), "{err}");
    }

    #[test]
    fn a_wrong_sized_key_file_is_refused_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("master.key");
        std::fs::write(&path, b"short").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let s = Secrets::new(&path);
        assert!(s.seal(b"v", "c").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"short");
    }
}
