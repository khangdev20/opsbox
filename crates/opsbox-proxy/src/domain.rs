//! Domain name normalization and validation.

use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct DomainError(pub String);

/// Normalize user input (such as `"https://Panel.Example.com/"`) into a canonical bare
/// domain name (`"panel.example.com"`), rejecting paths, ports, IP addresses, wildcards,
/// and invalid hostnames.
pub fn normalize_domain(input: &str) -> Result<String, DomainError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(DomainError(
            "Enter a domain name like panel.example.com.".into(),
        ));
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("http://") {
        return Err(DomainError(
            "Enter the domain name without http:// (for example panel.example.com).".into(),
        ));
    }
    let without_scheme = lower.strip_prefix("https://").unwrap_or(&lower);
    let without_trailing_slash = without_scheme.strip_suffix('/').unwrap_or(without_scheme);

    if without_trailing_slash.contains(['/', '?', '#', '@', '\\'])
        || without_trailing_slash.chars().any(char::is_whitespace)
    {
        return Err(DomainError(
            "Enter only the domain name with no path, query, or spaces (for example panel.example.com).".into(),
        ));
    }
    if without_trailing_slash.contains(':') {
        return Err(DomainError(
            "Enter only the domain name with no port number (for example panel.example.com)."
                .into(),
        ));
    }

    validate_domain(without_trailing_slash)?;
    Ok(without_trailing_slash.to_owned())
}

/// Strictly validate an already-normalized bare ASCII hostname.
pub fn validate_domain(domain: &str) -> Result<(), DomainError> {
    if domain.is_empty() || domain.len() > 253 {
        return Err(DomainError(
            "Enter a valid domain name (1 to 253 characters, for example panel.example.com)."
                .into(),
        ));
    }
    if domain.starts_with("*.") || domain.contains('*') {
        return Err(DomainError(
            "Wildcard domains require DNS-01 and are not supported yet.".into(),
        ));
    }
    if domain.parse::<IpAddr>().is_ok() {
        return Err(DomainError(
            "Enter a domain name like panel.example.com, not an IP address.".into(),
        ));
    }
    if domain.starts_with('.') || domain.ends_with('.') || domain.contains("..") {
        return Err(DomainError(
            "Domain name cannot start or end with a dot or contain empty parts.".into(),
        ));
    }
    let labels: Vec<&str> = domain.split('.').collect();
    if labels.len() < 2 {
        return Err(DomainError(
            "Enter a full domain name with a dot (for example panel.example.com).".into(),
        ));
    }
    for label in &labels {
        let valid_chars = label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !valid_chars
        {
            return Err(DomainError(
                "Domain parts may contain only letters, digits, and hyphens, and cannot start or end with a hyphen.".into(),
            ));
        }
    }
    let tld = labels[labels.len() - 1];
    if tld.len() < 2 || tld.bytes().all(|b| b.is_ascii_digit()) {
        return Err(DomainError(
            "The domain must end with a valid top-level domain (such as .com or .dev).".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_validates_domains() {
        assert_eq!(
            normalize_domain("  https://Panel.Example.COM/  ").unwrap(),
            "panel.example.com"
        );
        assert_eq!(
            normalize_domain("ops.my-server.dev").unwrap(),
            "ops.my-server.dev"
        );
        for bad in [
            "",
            "   ",
            "http://panel.example.com",
            "https://panel.example.com/path",
            "panel.example.com:8443",
            "*.example.com",
            "127.0.0.1",
            "203.0.113.10",
            "localhost",
            "-bad.example.com",
            "bad-.example.com",
            "a..b.com",
            ".example.com",
            "example.com.",
            "foo.123",
            "user@panel.example.com",
        ] {
            assert!(normalize_domain(bad).is_err(), "{bad:?}");
        }
    }
}
