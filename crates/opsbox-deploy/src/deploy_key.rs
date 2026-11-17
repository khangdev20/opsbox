//! Per-repo SSH Ed25519 deploy key generation and repository SSH URL validation.

use std::net::{IpAddr, Ipv6Addr};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use ring::signature::{Ed25519KeyPair, KeyPair};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::DeployError;

pub struct DeployKeyPair {
    pub public_key: String,
    pub fingerprint: String,
    pub private_key_pem: Zeroizing<String>,
}

impl std::fmt::Debug for DeployKeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeployKeyPair")
            .field("public_key", &self.public_key)
            .field("fingerprint", &self.fingerprint)
            .field("private_key_pem", &"[redacted]")
            .finish()
    }
}

fn push_bytes(buf: &mut Vec<u8>, data: &[u8]) {
    let len = u32::try_from(data.len()).unwrap_or(u32::MAX);
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(data);
}

fn push_str(buf: &mut Vec<u8>, s: &str) {
    push_bytes(buf, s.as_bytes());
}

fn wrap_pem(header: &str, footer: &str, raw: &[u8]) -> Zeroizing<String> {
    let b64 = Zeroizing::new(STANDARD.encode(raw));
    let mut out = Zeroizing::new(String::with_capacity(b64.len() + 128));
    out.push_str(header);
    out.push('\n');
    for chunk in b64.as_bytes().chunks(70) {
        if let Ok(line) = std::str::from_utf8(chunk) {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push_str(footer);
    out.push('\n');
    out
}

/// Generate a fresh Ed25519 SSH keypair in OpenSSH (`openssh-key-v1`) format.
pub fn generate_ed25519_deploy_key(comment: &str) -> Result<DeployKeyPair, DeployError> {
    let clean_comment: String = comment
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@'))
        .take(64)
        .collect();
    let comment = if clean_comment.is_empty() {
        "opsbox-deploy"
    } else {
        clean_comment.as_str()
    };

    let mut seed = Zeroizing::new([0u8; 32]);
    getrandom::fill(&mut seed[..]).map_err(|_| DeployError::Internal("RNG failure".into()))?;
    let pair = Ed25519KeyPair::from_seed_unchecked(&seed[..])
        .map_err(|_| DeployError::Internal("Ed25519 key generation failed".into()))?;
    let pub_bytes = pair.public_key().as_ref();

    let mut pub_blob = Vec::with_capacity(64);
    push_str(&mut pub_blob, "ssh-ed25519");
    push_bytes(&mut pub_blob, pub_bytes);

    let fingerprint = format!(
        "SHA256:{}",
        STANDARD_NO_PAD.encode(Sha256::digest(&pub_blob))
    );
    let public_key = format!("ssh-ed25519 {} {comment}", STANDARD.encode(&pub_blob));

    let mut check_bytes = [0u8; 4];
    getrandom::fill(&mut check_bytes).map_err(|_| DeployError::Internal("RNG failure".into()))?;

    let mut priv_block = Zeroizing::new(Vec::with_capacity(192));
    priv_block.extend_from_slice(&check_bytes);
    priv_block.extend_from_slice(&check_bytes);
    push_str(&mut priv_block, "ssh-ed25519");
    push_bytes(&mut priv_block, pub_bytes);
    let mut sk64 = Zeroizing::new([0u8; 64]);
    sk64[..32].copy_from_slice(&seed[..]);
    sk64[32..].copy_from_slice(pub_bytes);
    push_bytes(&mut priv_block, &sk64[..]);
    push_str(&mut priv_block, comment);

    let mut pad = 1u8;
    while priv_block.len() % 8 != 0 {
        priv_block.push(pad);
        pad = pad.wrapping_add(1);
    }

    let mut key_raw = Zeroizing::new(Vec::with_capacity(256));
    key_raw.extend_from_slice(b"openssh-key-v1\0");
    push_str(&mut key_raw, "none");
    push_str(&mut key_raw, "none");
    push_bytes(&mut key_raw, &[]);
    key_raw.extend_from_slice(&1u32.to_be_bytes());
    push_bytes(&mut key_raw, &pub_blob);
    push_bytes(&mut key_raw, &priv_block);

    let private_key_pem = wrap_pem(
        "-----BEGIN OPENSSH PRIVATE KEY-----",
        "-----END OPENSSH PRIVATE KEY-----",
        &key_raw,
    );

    Ok(DeployKeyPair {
        public_key,
        fingerprint,
        private_key_pem,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedDeployRepo {
    pub repo: String,
    pub ssh_url: String,
}

pub(crate) fn is_disallowed_host(host: &str) -> bool {
    let lower = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    if lower.is_empty()
        || lower == "localhost"
        || lower.ends_with(".localhost")
        || lower.ends_with(".local")
        || lower.ends_with(".internal")
    {
        return true;
    }
    if let Ok(ip) = lower.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => {
                v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_unspecified()
                    || v4.is_multicast()
            }
            IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_multicast()
                    || is_ipv6_ula_or_ll(v6)
                    || v6.to_ipv4_mapped().is_some_and(|v4| {
                        v4.is_loopback()
                            || v4.is_private()
                            || v4.is_link_local()
                            || v4.is_unspecified()
                    })
            }
        };
    }
    false
}

fn is_ipv6_ula_or_ll(v6: Ipv6Addr) -> bool {
    let seg0 = v6.segments()[0];
    (seg0 & 0xfe00) == 0xfc00 || (seg0 & 0xffc0) == 0xfe80
}

fn valid_repo_segment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && !s.starts_with('-')
        && !s.starts_with('.')
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn valid_host_label(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with('-')
        && host.contains('.')
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
}

/// Parse and normalize a repository reference for deploy-key access:
/// - `owner/repo` -> `git@github.com:owner/repo.git`
/// - `https://github.com/owner/repo(.git)` -> `git@github.com:owner/repo.git`
/// - `git@<host>:<owner>/<repo>(.git)` -> `git@<host>:<owner>/<repo>.git`
pub fn normalize_repo_source(raw: &str) -> Result<NormalizedDeployRepo, DeployError> {
    let input = raw.trim();
    let bad = |msg: &str| DeployError::Validation {
        field: Some("repo"),
        message: msg.to_owned(),
    };
    if input.is_empty() || input.len() > 256 || input.starts_with('-') {
        return Err(bad(
            "Enter a repository like owner/repo or git@github.com:owner/repo.git.",
        ));
    }
    if input.chars().any(|c| {
        c.is_whitespace()
            || c.is_control()
            || matches!(c, '?' | '#' | ';' | '|' | '&' | '$' | '`' | '\\')
    }) {
        return Err(bad("Repository contains unsupported characters."));
    }

    if let Some(rest) = input.strip_prefix("git@") {
        let (host, path) = rest
            .split_once(':')
            .ok_or_else(|| bad("SSH URL must look like git@github.com:owner/repo.git."))?;
        if !valid_host_label(host) || is_disallowed_host(host) {
            return Err(bad("Repository host is not allowed."));
        }
        let clean = path.trim_start_matches('/').trim_end_matches(".git");
        let (owner, repo) = clean
            .split_once('/')
            .ok_or_else(|| bad("SSH repository path must be owner/repo."))?;
        if !valid_repo_segment(owner) || !valid_repo_segment(repo) {
            return Err(bad("Repository owner and name contain invalid characters."));
        }
        let host_lower = host.to_ascii_lowercase();
        let display = if host_lower == "github.com" {
            format!("{owner}/{repo}")
        } else {
            format!("{host_lower}/{owner}/{repo}")
        };
        return Ok(NormalizedDeployRepo {
            repo: display,
            ssh_url: format!("git@{host_lower}:{owner}/{repo}.git"),
        });
    }

    if input.contains("://") {
        let parsed = url::Url::parse(input).map_err(|_| {
            bad("Enter a repository like owner/repo or https://github.com/owner/repo.")
        })?;
        if parsed.scheme() != "https" && parsed.scheme() != "ssh" {
            return Err(bad(
                "Only HTTPS or SSH repository references are supported.",
            ));
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| bad("Repository URL must include a host."))?;
        if !valid_host_label(host) || is_disallowed_host(host) {
            return Err(bad("Repository host is not allowed."));
        }
        let clean = parsed.path().trim_matches('/').trim_end_matches(".git");
        let parts: Vec<&str> = clean.split('/').collect();
        if parts.len() != 2 || !valid_repo_segment(parts[0]) || !valid_repo_segment(parts[1]) {
            return Err(bad("Repository URL path must be /owner/repo."));
        }
        let host_lower = host.to_ascii_lowercase();
        let display = if host_lower == "github.com" {
            format!("{}/{}", parts[0], parts[1])
        } else {
            format!("{host_lower}/{}/{}", parts[0], parts[1])
        };
        return Ok(NormalizedDeployRepo {
            repo: display,
            ssh_url: format!("git@{host_lower}:{}/{}.git", parts[0], parts[1]),
        });
    }

    let clean = input.trim_matches('/').trim_end_matches(".git");
    let parts: Vec<&str> = clean.split('/').collect();
    if parts.len() != 2 || !valid_repo_segment(parts[0]) || !valid_repo_segment(parts[1]) {
        return Err(bad(
            "Enter a repository like owner/repo or git@github.com:owner/repo.git.",
        ));
    }
    Ok(NormalizedDeployRepo {
        repo: format!("{}/{}", parts[0], parts[1]),
        ssh_url: format!("git@github.com:{}/{}.git", parts[0], parts[1]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn generates_valid_openssh_ed25519_keypair() {
        let kp = generate_ed25519_deploy_key("opsbox-deploy-test").unwrap();
        assert!(kp.public_key.starts_with("ssh-ed25519 "));
        assert!(kp.public_key.ends_with(" opsbox-deploy-test"));
        assert!(kp.fingerprint.starts_with("SHA256:"));
        assert!(
            kp.private_key_pem
                .starts_with("-----BEGIN OPENSSH PRIVATE KEY-----")
        );
        assert!(!format!("{kp:?}").contains("BEGIN OPENSSH PRIVATE KEY"));

        // Verify with system ssh-keygen when present that the OpenSSH binary format is exact.
        let dir = std::env::temp_dir().join(format!("opsbox-dk-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let priv_path = dir.join("id_ed25519");
        std::fs::write(&priv_path, kp.private_key_pem.as_bytes()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&priv_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        if let Ok(out) = Command::new("ssh-keygen")
            .args(["-y", "-f"])
            .arg(&priv_path)
            .output()
            && out.status.success()
        {
            let derived_pub = String::from_utf8_lossy(&out.stdout);
            let derived_parts: Vec<&str> = derived_pub.split_whitespace().collect();
            let orig_parts: Vec<&str> = kp.public_key.split_whitespace().collect();
            assert_eq!(derived_parts[..2], orig_parts[..2]);
        }
        if let Ok(out) = Command::new("ssh-keygen")
            .args(["-lf"])
            .arg(&priv_path)
            .output()
            && out.status.success()
        {
            let line = String::from_utf8_lossy(&out.stdout);
            assert!(
                line.contains(&kp.fingerprint),
                "{line} vs {}",
                kp.fingerprint
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalizes_repo_sources_and_blocks_injection_or_local_hosts() {
        assert_eq!(
            normalize_repo_source("acme/private-api").unwrap(),
            NormalizedDeployRepo {
                repo: "acme/private-api".into(),
                ssh_url: "git@github.com:acme/private-api.git".into(),
            }
        );
        assert_eq!(
            normalize_repo_source("https://github.com/acme/private-api.git").unwrap(),
            NormalizedDeployRepo {
                repo: "acme/private-api".into(),
                ssh_url: "git@github.com:acme/private-api.git".into(),
            }
        );
        assert_eq!(
            normalize_repo_source("git@gitlab.com:acme/web.git").unwrap(),
            NormalizedDeployRepo {
                repo: "gitlab.com/acme/web".into(),
                ssh_url: "git@gitlab.com:acme/web.git".into(),
            }
        );

        for bad in [
            "",
            "-oProxyCommand=id",
            "acme/repo;id",
            "git@localhost:acme/repo.git",
            "git@127.0.0.1:acme/repo.git",
            "https://10.0.0.1/acme/repo.git",
            "http://github.com/acme/repo",
            "acme/../etc",
        ] {
            assert!(normalize_repo_source(bad).is_err(), "{bad}");
        }
    }
}
