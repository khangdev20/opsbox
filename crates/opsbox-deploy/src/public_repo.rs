//! Public HTTPS Git repository URL validation and GitHub repo parsing.

use crate::DeployError;
use crate::deploy_key::is_disallowed_host;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicRepoTarget {
    GitHub {
        owner: String,
        repo: String,
        canonical_url: String,
    },
    GenericGit {
        host: String,
        display_name: String,
        canonical_url: String,
    },
}

fn valid_segment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && !s.starts_with('-')
        && !s.starts_with('.')
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Validate a public repository URL (`https://...`), rejecting credentials, fragments,
/// and private/loopback/local hosts.
pub fn validate_public_repo_url(raw: &str) -> Result<PublicRepoTarget, DeployError> {
    let bad = |msg: &str| DeployError::Validation {
        field: Some("url"),
        message: msg.to_owned(),
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 512 {
        return Err(bad(
            "Enter a public HTTPS repository URL (for example https://github.com/owner/repo).",
        ));
    }
    if trimmed.chars().any(|c| {
        c.is_whitespace() || c.is_control() || matches!(c, ';' | '|' | '&' | '$' | '`' | '\\')
    }) {
        return Err(bad("Repository URL contains unsupported characters."));
    }
    let parsed = url::Url::parse(trimmed).map_err(|_| {
        bad("Enter a valid HTTPS repository URL (for example https://github.com/owner/repo).")
    })?;
    if parsed.scheme() != "https" {
        return Err(bad("Public repository URL must use https://."));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(bad(
            "Repository URL must not include a username or password.",
        ));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(bad(
            "Remove query parameters and fragments from the repository URL.",
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| bad("Repository URL must include a host name."))?
        .to_ascii_lowercase();
    if !host.contains('.') || is_disallowed_host(&host) {
        return Err(bad("That repository host is not allowed."));
    }

    let path = parsed.path().trim_matches('/').trim_end_matches(".git");
    let segments: Vec<&str> = path.split('/').collect();
    if segments.is_empty() || segments.iter().any(|s| !valid_segment(s)) {
        return Err(bad("Repository URL path is not valid."));
    }

    if host == "github.com" || host == "www.github.com" {
        if segments.len() != 2 {
            return Err(bad(
                "GitHub repository URL must look like https://github.com/owner/repo.",
            ));
        }
        let owner = segments[0].to_owned();
        let repo = segments[1].to_owned();
        return Ok(PublicRepoTarget::GitHub {
            canonical_url: format!("https://github.com/{owner}/{repo}"),
            owner,
            repo,
        });
    }

    if segments.len() < 2 {
        return Err(bad(
            "Repository URL must include an owner and repository name.",
        ));
    }
    let display_name = segments.join("/");
    Ok(PublicRepoTarget::GenericGit {
        canonical_url: format!("https://{host}/{display_name}"),
        host,
        display_name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_github_and_generic_https_urls() {
        assert_eq!(
            validate_public_repo_url("https://github.com/acme/shop.git").unwrap(),
            PublicRepoTarget::GitHub {
                owner: "acme".into(),
                repo: "shop".into(),
                canonical_url: "https://github.com/acme/shop".into(),
            }
        );
        assert_eq!(
            validate_public_repo_url("https://codeberg.org/alice/app/").unwrap(),
            PublicRepoTarget::GenericGit {
                host: "codeberg.org".into(),
                display_name: "alice/app".into(),
                canonical_url: "https://codeberg.org/alice/app".into(),
            }
        );
        for bad in [
            "",
            "http://github.com/acme/shop",
            "git://github.com/acme/shop",
            "https://user:pass@github.com/acme/shop",
            "https://localhost/acme/shop",
            "https://127.0.0.1/acme/shop",
            "https://192.168.1.10/acme/shop",
            "https://169.254.169.254/latest/meta-data",
            "https://github.com/acme",
            "https://github.com/acme/shop?tab=readme",
        ] {
            assert!(validate_public_repo_url(bad).is_err(), "{bad}");
        }
    }
}
