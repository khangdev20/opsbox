//! GitHub App RS256 JWT signing using `ring` (zero native OpenSSL dependencies).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use ring::rand::SystemRandom;
use ring::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use serde_json::json;

use crate::DeployError;

/// Parse an RSA private key PEM (`RSA PRIVATE KEY` PKCS#1 from GitHub App manifest conversions,
/// or `PRIVATE KEY` PKCS#8).
pub fn parse_rsa_keypair(pem_str: &str) -> Result<RsaKeyPair, DeployError> {
    let parsed = pem::parse(pem_str.trim())
        .map_err(|_| DeployError::InvalidKey("GitHub App private key is not valid PEM.".into()))?;
    match parsed.tag() {
        "RSA PRIVATE KEY" => RsaKeyPair::from_der(parsed.contents()).map_err(|_| {
            DeployError::InvalidKey("GitHub App PKCS#1 private key is invalid.".into())
        }),
        "PRIVATE KEY" => RsaKeyPair::from_pkcs8(parsed.contents()).map_err(|_| {
            DeployError::InvalidKey("GitHub App PKCS#8 private key is invalid.".into())
        }),
        _ => RsaKeyPair::from_der(parsed.contents())
            .or_else(|_| RsaKeyPair::from_pkcs8(parsed.contents()))
            .map_err(|_| {
                DeployError::InvalidKey("Unsupported GitHub App private key format.".into())
            }),
    }
}

/// Sign a short-lived GitHub App JWT (`RS256`, 10-minute window with 60 s clock-skew cushion).
pub fn sign_app_jwt(app_id: i64, pem_str: &str, now_secs: i64) -> Result<String, DeployError> {
    if app_id <= 0 {
        return Err(DeployError::InvalidKey("Invalid GitHub App ID.".into()));
    }
    let key_pair = parse_rsa_keypair(pem_str)?;
    let header = B64URL.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let claims = json!({
        "iat": now_secs.saturating_sub(60),
        "exp": now_secs.saturating_add(540),
        "iss": app_id.to_string(),
    });
    let claims_json = serde_json::to_vec(&claims)
        .map_err(|e| DeployError::Internal(format!("JWT claims encoding failed: {e}")))?;
    let claims_b64 = B64URL.encode(claims_json);
    let signing_input = format!("{header}.{claims_b64}");

    let rng = SystemRandom::new();
    let mut sig = vec![0u8; key_pair.public().modulus_len()];
    key_pair
        .sign(&RSA_PKCS1_SHA256, &rng, signing_input.as_bytes(), &mut sig)
        .map_err(|_| DeployError::InvalidKey("Could not sign GitHub App JWT.".into()))?;

    let sig_b64 = B64URL.encode(&sig);
    Ok(format!("{signing_input}.{sig_b64}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn signs_verifiable_rs256_jwt_from_pkcs1_and_pkcs8() {
        // Generate an ephemeral 2048-bit RSA key via openssl in test if available;
        // otherwise verify malformed keys are rejected cleanly.
        let dir = std::env::temp_dir().join(format!("opsbox-jwt-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let key_path = dir.join("app.pem");
        let pkcs8_path = dir.join("app_pkcs8.pem");
        let pub_der_path = dir.join("app_pub.der");

        let ok = Command::new("openssl")
            .args(["genrsa", "-out"])
            .arg(&key_path)
            .arg("2048")
            .output()
            .is_ok_and(|o| o.status.success());
        if !ok {
            assert!(parse_rsa_keypair("not-a-pem").is_err());
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }

        let _ = Command::new("openssl")
            .args(["pkcs8", "-topk8", "-nocrypt", "-in"])
            .arg(&key_path)
            .args(["-out"])
            .arg(&pkcs8_path)
            .output()
            .unwrap();
        let _ = Command::new("openssl")
            .args(["rsa", "-in"])
            .arg(&key_path)
            .args(["-pubout", "-outform", "DER", "-out"])
            .arg(&pub_der_path)
            .output()
            .unwrap();

        let pem_pkcs1 = std::fs::read_to_string(&key_path).unwrap();
        let pem_pkcs8 = std::fs::read_to_string(&pkcs8_path).unwrap();
        let pub_spki = std::fs::read(&pub_der_path).unwrap();

        // Extract raw RSAPublicKey (PKCS#1) inside SubjectPublicKeyInfo for ring verification:
        let key_pair = parse_rsa_keypair(&pem_pkcs1).unwrap();
        let pub_bytes = key_pair.public().as_ref();
        assert!(!pub_spki.is_empty());

        for pem in [&pem_pkcs1, &pem_pkcs8] {
            let jwt = sign_app_jwt(12345, pem, 1_700_000_000).unwrap();
            let parts: Vec<&str> = jwt.split('.').collect();
            assert_eq!(parts.len(), 3);
            let claims_raw = B64URL.decode(parts[1]).unwrap();
            let claims: serde_json::Value = serde_json::from_slice(&claims_raw).unwrap();
            assert_eq!(claims["iss"], "12345");
            assert_eq!(claims["iat"], 1_699_999_940);
            assert_eq!(claims["exp"], 1_700_000_540);

            let sig_bytes = B64URL.decode(parts[2]).unwrap();
            let signing_input = format!("{}.{}", parts[0], parts[1]);
            let verifier = ring::signature::UnparsedPublicKey::new(
                &ring::signature::RSA_PKCS1_2048_8192_SHA256,
                pub_bytes,
            );
            verifier
                .verify(signing_input.as_bytes(), &sig_bytes)
                .unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_malformed_pem_and_invalid_app_id() {
        assert!(sign_app_jwt(0, "bad", 1000).is_err());
        assert!(
            parse_rsa_keypair(
                "-----BEGIN RSA PRIVATE KEY-----\nAAAA\n-----END RSA PRIVATE KEY-----"
            )
            .is_err()
        );
    }
}
