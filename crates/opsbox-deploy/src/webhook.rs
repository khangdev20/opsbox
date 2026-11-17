//! GitHub webhook HMAC-SHA256 verification and push event parsing (docs/BACKEND_SPEC.md 7.5 step 2).

use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use sha2::Sha256;

use crate::DeployError;

type HmacSha256 = Hmac<Sha256>;

/// Constant-time verification of GitHub's `X-Hub-Signature-256: sha256=<hex>` header.
pub fn verify_github_signature(
    secret: &[u8],
    body: &[u8],
    signature_header: Option<&str>,
) -> Result<(), DeployError> {
    if secret.is_empty() {
        return Err(DeployError::Unauthorized(
            "Webhook secret is not configured.".into(),
        ));
    }
    let header = signature_header
        .map(str::trim)
        .ok_or_else(|| DeployError::Unauthorized("Missing X-Hub-Signature-256 header.".into()))?;
    let hex = header
        .strip_prefix("sha256=")
        .ok_or_else(|| DeployError::Unauthorized("Invalid webhook signature format.".into()))?;
    let sig_bytes = decode_hex_32(hex)
        .ok_or_else(|| DeployError::Unauthorized("Invalid webhook signature format.".into()))?;

    let mut mac = HmacSha256::new_from_slice(secret)
        .map_err(|_| DeployError::Internal("HMAC key error".into()))?;
    mac.update(body);
    mac.verify_slice(&sig_bytes)
        .map_err(|_| DeployError::Unauthorized("Invalid webhook signature.".into()))
}

fn decode_hex_32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookEvent {
    Ping {
        hook_id: Option<i64>,
        zen: Option<String>,
    },
    Push {
        repo_full_name: String,
        git_ref: String,
        branch: Option<String>,
        after: String,
        deleted: bool,
        pusher: Option<String>,
        commit_message: Option<String>,
    },
    Installation {
        action: String,
        installation_id: i64,
        account_login: Option<String>,
        account_html_url: Option<String>,
        repository_selection: Option<String>,
    },
    InstallationRepositories {
        action: String,
        installation_id: i64,
        repository_selection: Option<String>,
    },
    Other {
        event: String,
    },
}

#[derive(Deserialize)]
struct PingPayload {
    hook_id: Option<i64>,
    zen: Option<String>,
}

#[derive(Deserialize)]
struct PushPayload {
    #[serde(rename = "ref")]
    git_ref: String,
    after: String,
    #[serde(default)]
    deleted: bool,
    repository: PushRepo,
    pusher: Option<PushPusher>,
    head_commit: Option<PushCommit>,
}

#[derive(Deserialize)]
struct PushRepo {
    full_name: String,
}

#[derive(Deserialize)]
struct PushPusher {
    name: Option<String>,
}

#[derive(Deserialize)]
struct PushCommit {
    message: Option<String>,
}

#[derive(Deserialize)]
struct InstallationPayload {
    action: String,
    installation: InstallationInfo,
}

#[derive(Deserialize)]
struct InstallationInfo {
    id: i64,
    account: Option<InstallationAccount>,
    html_url: Option<String>,
    repository_selection: Option<String>,
}

#[derive(Deserialize)]
struct InstallationAccount {
    login: String,
}

/// Parse an already HMAC-verified GitHub webhook payload.
pub fn parse_webhook_event(event: &str, body: &[u8]) -> Result<WebhookEvent, DeployError> {
    let ev = event.trim();
    if ev.is_empty() || ev.len() > 64 {
        return Err(DeployError::Validation {
            field: Some("X-GitHub-Event"),
            message: "Missing or invalid X-GitHub-Event header.".into(),
        });
    }
    match ev {
        "ping" => {
            let p: PingPayload =
                serde_json::from_slice(body).map_err(|_| DeployError::Validation {
                    field: None,
                    message: "Malformed GitHub ping payload.".into(),
                })?;
            Ok(WebhookEvent::Ping {
                hook_id: p.hook_id,
                zen: p.zen,
            })
        }
        "push" => {
            let p: PushPayload =
                serde_json::from_slice(body).map_err(|_| DeployError::Validation {
                    field: None,
                    message: "Malformed GitHub push payload.".into(),
                })?;
            let branch = p.git_ref.strip_prefix("refs/heads/").map(str::to_owned);
            Ok(WebhookEvent::Push {
                repo_full_name: p.repository.full_name,
                git_ref: p.git_ref,
                branch,
                after: p.after,
                deleted: p.deleted,
                pusher: p.pusher.and_then(|u| u.name),
                commit_message: p.head_commit.and_then(|c| c.message),
            })
        }
        "installation" => {
            let p: InstallationPayload =
                serde_json::from_slice(body).map_err(|_| DeployError::Validation {
                    field: None,
                    message: "Malformed GitHub installation payload.".into(),
                })?;
            Ok(WebhookEvent::Installation {
                action: p.action,
                installation_id: p.installation.id,
                account_login: p.installation.account.map(|a| a.login),
                account_html_url: p.installation.html_url,
                repository_selection: p.installation.repository_selection,
            })
        }
        "installation_repositories" => {
            let p: InstallationPayload =
                serde_json::from_slice(body).map_err(|_| DeployError::Validation {
                    field: None,
                    message: "Malformed GitHub installation_repositories payload.".into(),
                })?;
            Ok(WebhookEvent::InstallationRepositories {
                action: p.action,
                installation_id: p.installation.id,
                repository_selection: p.installation.repository_selection,
            })
        }
        other => Ok(WebhookEvent::Other {
            event: other.to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &[u8], body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(body);
        let bytes = mac.finalize().into_bytes();
        let mut hex = String::from("sha256=");
        for b in bytes {
            use std::fmt::Write as _;
            let _ = write!(hex, "{b:02x}");
        }
        hex
    }

    #[test]
    fn signature_verification_accepts_valid_and_rejects_tampered() {
        let secret = b"top-secret-webhook-key";
        let body =
            br#"{"ref":"refs/heads/main","after":"abc123","repository":{"full_name":"acme/shop"}}"#;
        let sig = sign(secret, body);

        verify_github_signature(secret, body, Some(&sig)).unwrap();
        assert!(verify_github_signature(secret, body, None).is_err());
        assert!(verify_github_signature(secret, body, Some("sha1=deadbeef")).is_err());
        assert!(verify_github_signature(secret, b"{}", Some(&sig)).is_err());
        assert!(verify_github_signature(b"wrong", body, Some(&sig)).is_err());
    }

    #[test]
    fn parses_push_and_ping_events() {
        let push = br#"{"ref":"refs/heads/main","after":"a3f9c21","deleted":false,"repository":{"full_name":"acme/shop"},"pusher":{"name":"alice"},"head_commit":{"message":"Deploy fix"}}"#;
        let ev = parse_webhook_event("push", push).unwrap();
        assert_eq!(
            ev,
            WebhookEvent::Push {
                repo_full_name: "acme/shop".into(),
                git_ref: "refs/heads/main".into(),
                branch: Some("main".into()),
                after: "a3f9c21".into(),
                deleted: false,
                pusher: Some("alice".into()),
                commit_message: Some("Deploy fix".into()),
            }
        );

        let ping = br#"{"hook_id":123,"zen":"Keep it logically awesome."}"#;
        assert!(matches!(
            parse_webhook_event("ping", ping).unwrap(),
            WebhookEvent::Ping {
                hook_id: Some(123),
                ..
            }
        ));
    }
}
