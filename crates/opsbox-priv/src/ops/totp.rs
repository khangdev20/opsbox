//! TOTP (RFC 6238): SHA-1, 6 digits, 30 s step, as every authenticator app expects.
//!
//! Secrets are created and checked here so that `serve` never holds one, except the
//! single time it shows the QR code at enrolment.

use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, KeyInit, Mac};
use opsbox_proto::{PrivError, PrivErrorCode, PrivReply, Secret};
use sha1::Sha1;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::secrets::Secrets;

pub const STEP_SECS: u64 = 30;
const DIGITS: u32 = 6;
const SECRET_LEN: usize = 20; // 160 bits, what RFC 4226 recommends for SHA-1

fn internal(msg: impl Into<String>) -> PrivError {
    PrivError {
        code: PrivErrorCode::Internal,
        message: msg.into(),
    }
}

pub(super) fn new_secret(secrets: &Secrets, context: &str) -> Result<PrivReply, PrivError> {
    let mut raw = Zeroizing::new([0u8; SECRET_LEN]);
    getrandom::fill(&mut raw[..]).map_err(|_| internal("random number generator failed"))?;
    let sealed = secrets
        .seal(&raw[..], context)
        .map_err(|e| internal(e.to_string()))?;
    Ok(PrivReply::TotpSecret {
        sealed,
        secret_base32: Secret::new(base32_encode(&raw[..])),
    })
}

pub(super) fn verify(
    secrets: &Secrets,
    sealed: &str,
    context: &str,
    code: &str,
    after_step: u64,
) -> Result<PrivReply, PrivError> {
    let secret = secrets
        .open(sealed, context)
        .map_err(|e| internal(e.to_string()))?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| internal("the system clock is before 1970"))?
        .as_secs();
    Ok(PrivReply::TotpResult {
        matched_step: verify_at(&secret, code, after_step, now),
    })
}

/// The time step whose code equals `code`, looking one step either side of `now`,
/// but only steps strictly after `after_step` (replay protection).
pub fn verify_at(secret: &[u8], code: &str, after_step: u64, now_unix: u64) -> Option<u64> {
    if code.len() != DIGITS as usize || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let current = now_unix / STEP_SECS;
    let mut matched = None;
    // Evaluate all three candidates so the time taken does not depend on which matches.
    for step in [current.saturating_sub(1), current, current + 1] {
        let expected = code_at(secret, step);
        let hit: bool = expected.as_bytes().ct_eq(code.as_bytes()).into();
        if hit && step > after_step {
            matched = Some(matched.map_or(step, |m: u64| m.max(step)));
        }
    }
    matched
}

/// The 6-digit code for a time step (RFC 4226 dynamic truncation).
pub fn code_at(secret: &[u8], step: u64) -> String {
    let mut mac =
        <Hmac<Sha1> as KeyInit>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let bin = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    format!(
        "{:0width$}",
        bin % 10u32.pow(DIGITS),
        width = DIGITS as usize
    )
}

const B32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// RFC 4648 base32 without padding (what authenticator apps import).
pub fn base32_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(5) * 8);
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in data {
        buf = (buf << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            out.push(B32[((buf >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
        buf &= (1 << bits) - 1;
    }
    if bits > 0 {
        out.push(B32[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 appendix B (SHA-1, secret "12345678901234567890"); the RFC lists
    /// 8-digit codes, these are their last six digits.
    const RFC_SECRET: &[u8] = b"12345678901234567890";
    const VECTORS: [(u64, &str); 6] = [
        (59, "287082"),
        (1_111_111_109, "081804"),
        (1_111_111_111, "050471"),
        (1_234_567_890, "005924"),
        (2_000_000_000, "279037"),
        (20_000_000_000, "353130"),
    ];

    #[test]
    fn matches_the_rfc_6238_test_vectors() {
        for (t, want) in VECTORS {
            assert_eq!(code_at(RFC_SECRET, t / STEP_SECS), want, "t={t}");
        }
    }

    #[test]
    fn base32_matches_rfc_4648_vectors() {
        for (plain, enc) in [
            ("", ""),
            ("f", "MY"),
            ("fo", "MZXQ"),
            ("foo", "MZXW6"),
            ("foob", "MZXW6YQ"),
            ("fooba", "MZXW6YTB"),
            ("foobar", "MZXW6YTBOI"),
        ] {
            assert_eq!(base32_encode(plain.as_bytes()), enc);
        }
        assert_eq!(
            base32_encode(RFC_SECRET),
            "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"
        );
    }

    #[test]
    fn accepts_one_step_either_side_but_not_two() {
        let t = 1_111_111_109u64; // inside step 37037036
        let step = t / STEP_SECS;
        for off in [-1i64, 0, 1] {
            let code = code_at(RFC_SECRET, (step as i64 + off) as u64);
            assert_eq!(
                verify_at(RFC_SECRET, &code, 0, t),
                Some((step as i64 + off) as u64),
                "offset {off}"
            );
        }
        for off in [-2i64, 2] {
            let code = code_at(RFC_SECRET, (step as i64 + off) as u64);
            assert_eq!(verify_at(RFC_SECRET, &code, 0, t), None, "offset {off}");
        }
    }

    #[test]
    fn a_code_cannot_be_used_twice() {
        let now = 1_234_567_890u64;
        let step = now / STEP_SECS;
        let code = code_at(RFC_SECRET, step);
        let first = verify_at(RFC_SECRET, &code, 0, now).unwrap();
        assert_eq!(first, step);
        // Same code again, after recording `first`: refused.
        assert_eq!(verify_at(RFC_SECRET, &code, first, now), None);
        // The previous step's code is also refused (it is older than the last one used).
        let older = code_at(RFC_SECRET, step - 1);
        assert_eq!(verify_at(RFC_SECRET, &older, first, now), None);
        // The next step's code still works.
        let newer = code_at(RFC_SECRET, step + 1);
        assert_eq!(verify_at(RFC_SECRET, &newer, first, now), Some(step + 1));
    }

    #[test]
    fn malformed_codes_never_match() {
        let now = 59;
        for bad in [
            "",
            "12345",
            "1234567",
            "12345a",
            "287 82",
            "-87082",
            "２８７０８２",
        ] {
            assert_eq!(verify_at(RFC_SECRET, bad, 0, now), None, "{bad:?}");
        }
    }

    #[test]
    fn works_through_the_sealed_secret_path() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Secrets::new(dir.path().join("k"));
        let PrivReply::TotpSecret {
            sealed,
            secret_base32,
        } = new_secret(&secrets, "totp:u1").unwrap()
        else {
            panic!("wrong reply");
        };
        assert_eq!(secret_base32.expose().len(), 32);
        assert!(secret_base32.expose().bytes().all(|b| B32.contains(&b)));
        // The code for "now" verifies; the secret is bound to its context.
        let raw = secrets.open(&sealed, "totp:u1").unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let code = code_at(&raw, now / STEP_SECS);
        let PrivReply::TotpResult { matched_step } =
            verify(&secrets, &sealed, "totp:u1", &code, 0).unwrap()
        else {
            panic!("wrong reply");
        };
        assert!(matched_step.is_some());
        assert!(verify(&secrets, &sealed, "totp:other", &code, 0).is_err());
    }
}
