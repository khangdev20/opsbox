//! Namespaced secret sealing/opening for non-TOTP subsystems (proxy keys and deploy secrets).
//!
//! `totp:*` contexts are unconditionally refused here so a compromised caller can
//! never read TOTP secrets via generic `SecretOpen`.

use opsbox_proto::{PrivError, PrivErrorCode, PrivReply, Secret};

use crate::secrets::{SecretError, Secrets};

const ALLOWED_PREFIXES: &[&str] = &["proxy:", "deploy:", "backup:", "alerts:"];
const MAX_PLAINTEXT_BYTES: usize = 64 * 1024;

pub(crate) fn valid_context(context: &str) -> Result<(), PrivError> {
    let bad = |msg: &str| PrivError {
        code: PrivErrorCode::Refused,
        message: msg.to_owned(),
    };
    if context.len() > 128 || !ALLOWED_PREFIXES.iter().any(|p| context.starts_with(p)) {
        return Err(bad("This secret context is not allowed."));
    }
    let suffix = context.split_once(':').map_or("", |(_, s)| s);
    if suffix.is_empty()
        || suffix.contains("..")
        || !suffix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'.' | b'_' | b'-'))
    {
        return Err(bad("This secret context contains disallowed characters."));
    }
    Ok(())
}

pub(crate) fn seal(
    secrets: &Secrets,
    context: &str,
    plaintext: &Secret,
) -> Result<PrivReply, PrivError> {
    valid_context(context)?;
    if plaintext.expose().len() > MAX_PLAINTEXT_BYTES {
        return Err(PrivError {
            code: PrivErrorCode::Invalid,
            message: "Secret payload is too large.".into(),
        });
    }
    let sealed = secrets
        .seal(plaintext.expose().as_bytes(), context)
        .map_err(secret_err)?;
    Ok(PrivReply::Sealed { sealed })
}

pub(crate) fn open_secret(
    secrets: &Secrets,
    context: &str,
    sealed: &str,
) -> Result<Secret, PrivError> {
    valid_context(context)?;
    let plain = secrets.open(sealed, context).map_err(secret_err)?;
    let text = String::from_utf8(plain.to_vec()).map_err(|_| PrivError {
        code: PrivErrorCode::Invalid,
        message: "Sealed value is not valid UTF-8.".into(),
    })?;
    Ok(Secret::new(text))
}

pub(crate) fn open(secrets: &Secrets, context: &str, sealed: &str) -> Result<PrivReply, PrivError> {
    let plaintext = open_secret(secrets, context, sealed)?;
    Ok(PrivReply::Opened { plaintext })
}

fn secret_err(e: SecretError) -> PrivError {
    let code = match e {
        SecretError::Open => PrivErrorCode::Invalid,
        SecretError::Key { .. } | SecretError::Random => PrivErrorCode::Internal,
    };
    PrivError {
        code,
        message: e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_totp_and_malformed_contexts() {
        for bad in [
            "",
            "totp:user-1",
            "totp:admin",
            "other:key",
            "proxy:",
            "proxy:../escape",
            "proxy:a/b",
            "proxy:a b",
            &format!("proxy:{}", "a".repeat(130)),
        ] {
            assert_eq!(
                valid_context(bad).unwrap_err().code,
                PrivErrorCode::Refused,
                "{bad}"
            );
        }
        assert!(valid_context("proxy:acme_account:prod").is_ok());
        assert!(valid_context("proxy:cert:panel.example.com").is_ok());
        assert!(valid_context("deploy:github_app:private_key").is_ok());
        assert!(valid_context("deploy:github_app:webhook_secret").is_ok());
        assert!(valid_context("deploy:deploy_key:01J9ABCDEFGHJKMNPQRSTVWXYZ").is_ok());
        assert!(valid_context("alerts:channel:c4").is_ok());
        assert!(valid_context("alerts:").is_err());
        assert!(valid_context("alerts:../totp").is_err());
        assert!(valid_context("backup:dest:01J9ABCDEFGHJKMNPQRSTVWXYZ").is_ok());
        assert_eq!(
            valid_context("backup:").unwrap_err().code,
            PrivErrorCode::Refused
        );
        assert_eq!(
            valid_context("backup:../totp").unwrap_err().code,
            PrivErrorCode::Refused
        );
        assert_eq!(
            valid_context("deploy:").unwrap_err().code,
            PrivErrorCode::Refused
        );
        assert_eq!(
            valid_context("deploy:../totp").unwrap_err().code,
            PrivErrorCode::Refused
        );
    }

    #[test]
    fn alert_secrets_are_context_bound_and_cannot_open_totp() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Secrets::new(dir.path().join("master.key"));
        for id in ["c1", "c2", "c3", "c4"] {
            let context = format!("alerts:channel:{id}");
            let PrivReply::Sealed { sealed } =
                seal(&secrets, &context, &Secret::new("channel credential")).unwrap()
            else {
                panic!("expected sealed");
            };
            let PrivReply::Opened { plaintext } = open(&secrets, &context, &sealed).unwrap() else {
                panic!("expected opened");
            };
            assert_eq!(plaintext.expose(), "channel credential");
            assert!(open(&secrets, "alerts:channel:other", &sealed).is_err());
            assert!(open(&secrets, "totp:user", &sealed).is_err());
        }
    }

    #[test]
    fn seals_and_opens_deploy_secrets_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Secrets::new(dir.path().join("master.key"));
        let PrivReply::Sealed { sealed } = seal(
            &secrets,
            "deploy:github_app:webhook_secret",
            &Secret::new("whsec_test_secret"),
        )
        .unwrap() else {
            panic!("expected Sealed");
        };
        let PrivReply::Opened { plaintext } =
            open(&secrets, "deploy:github_app:webhook_secret", &sealed).unwrap()
        else {
            panic!("expected Opened");
        };
        assert_eq!(plaintext.expose(), "whsec_test_secret");
        assert!(open(&secrets, "deploy:github_app:private_key", &sealed).is_err());
    }

    #[test]
    fn seals_and_opens_proxy_secrets_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Secrets::new(dir.path().join("master.key"));
        let PrivReply::Sealed { sealed } = seal(
            &secrets,
            "proxy:cert:panel.example.com",
            &Secret::new("PRIVATE KEY DATA"),
        )
        .unwrap() else {
            panic!("expected Sealed");
        };
        let PrivReply::Opened { plaintext } =
            open(&secrets, "proxy:cert:panel.example.com", &sealed).unwrap()
        else {
            panic!("expected Opened");
        };
        assert_eq!(plaintext.expose(), "PRIVATE KEY DATA");
        // Context mismatch fails.
        assert!(open(&secrets, "proxy:cert:other.example.com", &sealed).is_err());
        // TOTP context is always refused.
        assert_eq!(
            open(&secrets, "totp:user-1", &sealed).unwrap_err().code,
            PrivErrorCode::Refused
        );
    }
}
