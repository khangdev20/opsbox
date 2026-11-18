//! Offsite backup destination preset validation and restic repository URI builders
//! (docs/BACKEND_SPEC.md 7.12).

use std::net::IpAddr;

use opsbox_proto::BackupProviderId;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::BackupError;

/// Raw form values submitted by the UI or API when testing/saving a backup destination.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DestinationFieldInput {
    #[serde(default)]
    pub account: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub bucket: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub secret: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub path: String,
}

/// Validated non-secret configuration for an offsite backup destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidatedDestinationConfig {
    S3 {
        provider: BackupProviderId,
        name: String,
        endpoint: String,
        region: Option<String>,
        bucket: String,
        prefix: String,
        server: String,
        repo_path: String,
        display_loc: String,
    },
    Sftp {
        name: String,
        host: String,
        port: u16,
        user: String,
        path: String,
        server: String,
        repo_path: String,
        display_loc: String,
    },
}

/// Sanitize a hostname into a safe repository folder slug (`<server>`).
pub fn sanitize_server_slug(hostname: &str) -> String {
    let clean: String = hostname
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = clean.trim_matches('-');
    if trimmed.is_empty() {
        "opsbox-server".to_owned()
    } else {
        trimmed.chars().take(64).collect()
    }
}

pub fn validate_destination_name(
    raw: Option<&str>,
    provider: BackupProviderId,
) -> Result<String, BackupError> {
    let default = format!("Offsite ({})", provider.display_name());
    let name = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&default);
    if name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err(BackupError::Validation(
            "Destination name must be one line of at most 80 characters.".into(),
        ));
    }
    Ok(name.to_owned())
}

fn validate_id_token(raw: &str, label: &str) -> Result<String, BackupError> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(BackupError::Validation(format!("Missing: {label}")));
    }
    if s.len() > 64
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(BackupError::Validation(format!(
            "{label} may contain only letters, digits, dashes, and underscores."
        )));
    }
    Ok(s.to_owned())
}

pub fn validate_s3_bucket(raw: &str) -> Result<String, BackupError> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(BackupError::Validation("Missing: Bucket".into()));
    }
    let bytes = s.as_bytes();
    let ok = (3..=63).contains(&bytes.len())
        && bytes.first().is_some_and(u8::is_ascii_lowercase)
            | bytes.first().is_some_and(u8::is_ascii_digit)
        && bytes.last().is_some_and(u8::is_ascii_lowercase)
            | bytes.last().is_some_and(u8::is_ascii_digit)
        && !s.contains("..")
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-'));
    if !ok {
        return Err(BackupError::Validation(
            "Bucket names use lowercase letters, numbers, dots and dashes (3-63 characters)."
                .into(),
        ));
    }
    Ok(s.to_owned())
}

pub fn validate_prefix(raw: &str) -> Result<String, BackupError> {
    let s = raw.trim().trim_matches('/');
    if s.is_empty() {
        return Ok(String::new());
    }
    if s.len() > 256 || s.contains("..") {
        return Err(BackupError::Validation(
            "Folder prefix must be under 256 characters and cannot contain '..'.".into(),
        ));
    }
    for seg in s.split('/') {
        if seg.is_empty()
            || !seg
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            return Err(BackupError::Validation(
                "Folder prefix may contain only letters, digits, '.', '-', '_', and '/' separators.".into(),
            ));
        }
    }
    Ok(s.to_owned())
}

fn is_forbidden_metadata_ip(host: &str) -> bool {
    let clean = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = clean.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => v4.is_link_local() || v4.is_unspecified() || v4.is_multicast(),
            IpAddr::V6(v6) => v6.is_unspecified() || v6.is_multicast(),
        };
    }
    false
}

pub fn validate_endpoint_url(raw: &str, allow_http: bool) -> Result<String, BackupError> {
    let s = raw.trim().trim_end_matches('/');
    if s.is_empty() {
        return Err(BackupError::Validation("Missing: Endpoint URL".into()));
    }
    if s.len() > 512
        || s.bytes()
            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
    {
        return Err(BackupError::Validation(
            "Endpoint URL contains invalid characters.".into(),
        ));
    }
    let parsed = Url::parse(s).map_err(|_| {
        BackupError::Validation(
            "Enter a valid endpoint URL (for example https://minio.example.com:9000).".into(),
        )
    })?;
    let scheme = parsed.scheme();
    if scheme != "https" && !(allow_http && scheme == "http") {
        return Err(BackupError::Validation(
            if allow_http {
                "Endpoint URL must start with https:// or http://."
            } else {
                "Endpoint URL must use https://."
            }
            .into(),
        ));
    }
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !parsed.path().trim_matches('/').is_empty()
    {
        return Err(BackupError::Validation(
            "Endpoint URL must contain only scheme, host, and optional port (no credentials or path).".into(),
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| BackupError::Validation("Endpoint URL must include a host.".into()))?;
    if is_forbidden_metadata_ip(host) {
        return Err(BackupError::Validation(
            "That endpoint host address is not allowed.".into(),
        ));
    }
    let port_part = parsed.port().map(|p| format!(":{p}")).unwrap_or_default();
    let host_part = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    Ok(format!("{scheme}://{host_part}{port_part}"))
}

pub fn validate_sftp_host(raw: &str) -> Result<String, BackupError> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(BackupError::Validation("Missing: Host".into()));
    }
    if s.len() > 253
        || s.starts_with('-')
        || s.bytes().any(|b| {
            b.is_ascii_control()
                || b.is_ascii_whitespace()
                || matches!(
                    b,
                    b'\'' | b'"' | b';' | b'|' | b'&' | b'$' | b'`' | b'\\' | b'/'
                )
        })
        || !(s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']')))
    {
        return Err(BackupError::Validation(
            "Host must be a valid hostname or IP address.".into(),
        ));
    }
    if is_forbidden_metadata_ip(s) {
        return Err(BackupError::Validation(
            "That SFTP host address is not allowed.".into(),
        ));
    }
    Ok(s.to_owned())
}

pub fn validate_sftp_port(raw: &str) -> Result<u16, BackupError> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(22);
    }
    let p: u16 = s.parse().map_err(|_| {
        BackupError::Validation("SSH port must be a number between 1 and 65535.".into())
    })?;
    if p == 0 {
        return Err(BackupError::Validation(
            "SSH port must be between 1 and 65535.".into(),
        ));
    }
    Ok(p)
}

pub fn validate_sftp_user(raw: &str) -> Result<String, BackupError> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(BackupError::Validation("Missing: User".into()));
    }
    if s.len() > 64
        || s.starts_with('-')
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(BackupError::Validation(
            "SFTP username contains invalid characters.".into(),
        ));
    }
    Ok(s.to_owned())
}

pub fn validate_sftp_path(raw: &str) -> Result<String, BackupError> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(BackupError::Validation(
            "Missing: Folder on that server".into(),
        ));
    }
    if !s.starts_with('/')
        || s.len() > 512
        || s.contains("..")
        || s.bytes().any(|b| {
            b.is_ascii_control()
                || b.is_ascii_whitespace()
                || matches!(
                    b,
                    b'\'' | b'"' | b';' | b'|' | b'&' | b'$' | b'`' | b'\\' | b'<' | b'>'
                )
        })
    {
        return Err(BackupError::Validation(
            "Remote SFTP folder must be an absolute path without spaces, '..', or shell characters.".into(),
        ));
    }
    let norm = if s == "/" {
        "/".to_owned()
    } else {
        s.trim_end_matches('/').to_owned()
    };
    Ok(norm)
}

/// Validate destination fields for `provider` and compute canonical repository config.
pub fn validate_destination_input(
    provider: BackupProviderId,
    name: Option<&str>,
    vals: &DestinationFieldInput,
    server_hostname: &str,
    require_secrets: bool,
) -> Result<ValidatedDestinationConfig, BackupError> {
    let clean_name = validate_destination_name(name, provider)?;
    let server = sanitize_server_slug(server_hostname);

    if provider.is_sftp() {
        let host = validate_sftp_host(&vals.host)?;
        let port = validate_sftp_port(&vals.port)?;
        let user = validate_sftp_user(&vals.user)?;
        let path = validate_sftp_path(&vals.path)?;
        let suffix = format!("/{server}");
        let repo_path = if path == "/" {
            suffix
        } else if path.ends_with(&suffix) {
            path.clone()
        } else {
            format!("{path}{suffix}")
        };
        let display_loc = if port == 22 {
            format!("sftp://{user}@{host}:{repo_path}")
        } else {
            format!("sftp://{user}@{host}:{port}{repo_path}")
        };
        return Ok(ValidatedDestinationConfig::Sftp {
            name: clean_name,
            host,
            port,
            user,
            path,
            server,
            repo_path,
            display_loc,
        });
    }

    let (endpoint, region) = match provider {
        BackupProviderId::R2 => {
            let acct = validate_id_token(&vals.account, "Account ID")?;
            (
                format!("https://{acct}.r2.cloudflarestorage.com"),
                Some("auto".to_owned()),
            )
        }
        BackupProviderId::S3 => {
            let reg = validate_id_token(&vals.region, "Region")?;
            (format!("https://s3.{reg}.amazonaws.com"), Some(reg))
        }
        BackupProviderId::B2 => {
            let reg = validate_id_token(&vals.region, "Region")?;
            (format!("https://s3.{reg}.backblazeb2.com"), Some(reg))
        }
        BackupProviderId::Wasabi => {
            let reg = validate_id_token(&vals.region, "Region")?;
            (format!("https://s3.{reg}.wasabisys.com"), Some(reg))
        }
        BackupProviderId::Do => {
            let reg = validate_id_token(&vals.region, "Region")?;
            (format!("https://{reg}.digitaloceanspaces.com"), Some(reg))
        }
        BackupProviderId::Hetzner => {
            let reg = validate_id_token(&vals.region, "Location")?;
            (format!("https://{reg}.your-objectstorage.com"), Some(reg))
        }
        BackupProviderId::Gcs => ("https://storage.googleapis.com".to_owned(), None),
        BackupProviderId::Minio => (validate_endpoint_url(&vals.endpoint, true)?, None),
        BackupProviderId::Custom => {
            let ep = validate_endpoint_url(&vals.endpoint, true)?;
            let reg = if vals.region.trim().is_empty() {
                None
            } else {
                Some(validate_id_token(&vals.region, "Region")?)
            };
            (ep, reg)
        }
        BackupProviderId::Sftp => unreachable!("handled above"),
    };

    let bucket = validate_s3_bucket(&vals.bucket)?;
    let prefix = validate_prefix(&vals.prefix)?;
    if require_secrets {
        if vals.key.trim().is_empty()
            || vals.key.len() > 256
            || vals.key.chars().any(char::is_control)
        {
            return Err(BackupError::Validation("Missing: Access Key ID".into()));
        }
        if vals.secret.trim().is_empty()
            || vals.secret.len() > 512
            || vals.secret.chars().any(char::is_control)
        {
            return Err(BackupError::Validation("Missing: Secret Access Key".into()));
        }
    }

    let repo_path = if prefix.is_empty() {
        server.clone()
    } else {
        format!("{prefix}/{server}")
    };
    let host_clean = endpoint
        .strip_prefix("https://")
        .or_else(|| endpoint.strip_prefix("http://"))
        .unwrap_or(&endpoint);
    let display_loc = format!("{host_clean}/{bucket}/{repo_path}");

    Ok(ValidatedDestinationConfig::S3 {
        provider,
        name: clean_name,
        endpoint,
        region,
        bucket,
        prefix,
        server,
        repo_path,
        display_loc,
    })
}

/// Build the exact `restic -r` repository URI for S3-compatible destinations.
pub fn s3_restic_repo_uri(endpoint: &str, bucket: &str, repo_path: &str) -> String {
    let ep = endpoint.trim_end_matches('/');
    let rp = repo_path.trim_matches('/');
    if rp.is_empty() {
        format!("s3:{ep}/{bucket}")
    } else {
        format!("s3:{ep}/{bucket}/{rp}")
    }
}

/// Build the exact `restic -r` repository URI for SFTP destinations.
pub fn sftp_restic_repo_uri(user: &str, host: &str, repo_path: &str) -> String {
    format!("sftp:{user}@{host}:{repo_path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_r2_minio_and_sftp_repos_and_rejects_bad_inputs() {
        let r2 = validate_destination_input(
            BackupProviderId::R2,
            None,
            &DestinationFieldInput {
                account: "acct123".into(),
                bucket: "opsbox-backups".into(),
                prefix: "/prod/".into(),
                key: "ak".into(),
                secret: "sk".into(),
                ..Default::default()
            },
            "srv-sgn-01",
            true,
        )
        .unwrap();
        let ValidatedDestinationConfig::S3 {
            endpoint,
            repo_path,
            display_loc,
            ..
        } = r2
        else {
            panic!("expected S3");
        };
        assert_eq!(endpoint, "https://acct123.r2.cloudflarestorage.com");
        assert_eq!(repo_path, "prod/srv-sgn-01");
        assert_eq!(
            s3_restic_repo_uri(&endpoint, "opsbox-backups", &repo_path),
            "s3:https://acct123.r2.cloudflarestorage.com/opsbox-backups/prod/srv-sgn-01"
        );
        assert_eq!(
            display_loc,
            "acct123.r2.cloudflarestorage.com/opsbox-backups/prod/srv-sgn-01"
        );

        let minio = validate_destination_input(
            BackupProviderId::Minio,
            Some("Local MinIO"),
            &DestinationFieldInput {
                endpoint: "http://127.0.0.1:9000/".into(),
                bucket: "backups".into(),
                key: "minioadmin".into(),
                secret: "minioadmin".into(),
                ..Default::default()
            },
            "srv-1",
            true,
        )
        .unwrap();
        let ValidatedDestinationConfig::S3 {
            endpoint: m_ep,
            repo_path: m_rp,
            ..
        } = minio
        else {
            panic!("expected S3");
        };
        assert_eq!(m_ep, "http://127.0.0.1:9000");
        assert_eq!(m_rp, "srv-1");

        let sftp = validate_destination_input(
            BackupProviderId::Sftp,
            Some("Hetzner Storage Box"),
            &DestinationFieldInput {
                host: "backup.example.com".into(),
                port: "2222".into(),
                user: "u12345".into(),
                path: "/srv/backups".into(),
                ..Default::default()
            },
            "srv-1",
            false,
        )
        .unwrap();
        let ValidatedDestinationConfig::Sftp {
            port,
            repo_path: s_rp,
            ..
        } = sftp
        else {
            panic!("expected SFTP");
        };
        assert_eq!(port, 2222);
        assert_eq!(s_rp, "/srv/backups/srv-1");
        assert_eq!(
            sftp_restic_repo_uri("u12345", "backup.example.com", &s_rp),
            "sftp:u12345@backup.example.com:/srv/backups/srv-1"
        );

        // Reject metadata IP, bad bucket, shell injection in SFTP path.
        assert!(validate_endpoint_url("http://169.254.169.254", true).is_err());
        assert!(validate_endpoint_url("file:///tmp/bucket", true).is_err());
        assert!(validate_s3_bucket("Bad_Bucket").is_err());
        assert!(validate_sftp_path("/srv/backups;rm -rf /").is_err());
        assert!(validate_sftp_host("host;id").is_err());
    }
}
