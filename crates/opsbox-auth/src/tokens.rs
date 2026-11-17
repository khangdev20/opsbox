//! Bearer tokens, their stored hashes and the CSRF token derived from them.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 256 random bits, URL-safe.
pub fn new_token() -> Result<String, getrandom::Error> {
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw)?;
    Ok(B64.encode(raw))
}

/// What is stored instead of the token: a database leak does not yield sessions.
pub fn hash_token(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// The CSRF token for a session: derived from the session token, so it cannot be
/// guessed or planted by someone who does not already hold the session. The page
/// reads it from the `opsbox_csrf` cookie and sends it back in `X-Opsbox-CSRF`.
pub fn csrf_token(session_token: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"opsbox-csrf-v1:");
    h.update(session_token.as_bytes());
    hex(&h.finalize())
}

pub fn ct_eq(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_unique_and_url_safe() {
        let a = new_token().unwrap();
        let b = new_token().unwrap();
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert!(
            a.bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        );
    }

    #[test]
    fn stored_hash_is_not_the_token_and_is_stable() {
        let t = new_token().unwrap();
        assert_eq!(hash_token(&t), hash_token(&t));
        assert_ne!(hash_token(&t), t);
        assert_eq!(hash_token(&t).len(), 64);
        assert_eq!(
            hash_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn csrf_token_is_bound_to_the_session() {
        let (a, b) = (new_token().unwrap(), new_token().unwrap());
        assert_eq!(csrf_token(&a), csrf_token(&a));
        assert_ne!(csrf_token(&a), csrf_token(&b));
        assert_ne!(
            csrf_token(&a),
            hash_token(&a),
            "must differ from the stored hash"
        );
    }

    #[test]
    fn constant_time_compare() {
        assert!(ct_eq("same", "same"));
        assert!(!ct_eq("same", "sama"));
        assert!(!ct_eq("short", "longer value"));
        assert!(!ct_eq("", "x"));
    }
}
