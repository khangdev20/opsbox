//! GitHub REST API client (manifest exchange, installations, repository listing, build detection).

use std::time::Duration;

use serde::Deserialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use zeroize::Zeroizing;

use crate::DeployError;
use crate::detect::detect_build_method;

pub const DEFAULT_API_BASE: &str = "https://api.github.com";
pub const DEFAULT_WEB_BASE: &str = "https://github.com";

#[derive(Debug, Clone)]
pub struct GitHubConfig {
    pub api_base: String,
    pub web_base: String,
}

impl Default for GitHubConfig {
    fn default() -> Self {
        Self {
            api_base: std::env::var("OPSBOX_GITHUB_API_BASE")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
            web_base: std::env::var("OPSBOX_GITHUB_WEB_BASE")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_WEB_BASE.to_owned()),
        }
    }
}

#[derive(Clone)]
pub struct GitHubClient {
    agent: ureq::Agent,
    pub config: GitHubConfig,
}

impl GitHubClient {
    pub fn new(config: GitHubConfig) -> Self {
        let cfg = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(cfg),
            config,
        }
    }

    fn api_url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.config.api_base.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    /// Exchange a temporary manifest `code` for the newly created GitHub App credentials.
    pub fn exchange_manifest_code(&self, code: &str) -> Result<ManifestConversion, DeployError> {
        let url = self.api_url(&format!("/app-manifests/{code}/conversions"));
        let resp = self
            .agent
            .post(&url)
            .header("User-Agent", "opsbox")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send_empty()
            .map_err(|e| DeployError::GitHub(format!("Could not reach GitHub: {e}")))?;

        let status = resp.status().as_u16();
        let body = resp
            .into_body()
            .with_config()
            .limit(1024 * 1024)
            .read_to_string()
            .map_err(|e| DeployError::GitHub(format!("Could not read GitHub response: {e}")))?;

        if status == 404 || status == 422 {
            return Err(DeployError::Validation {
                field: Some("code"),
                message: "The GitHub setup code expired or was already used. Start connecting GitHub again."
                    .into(),
            });
        }
        if !(200..300).contains(&status) {
            return Err(github_http_error(status, &body));
        }

        let raw: RawManifestConversion = serde_json::from_str(&body).map_err(|e| {
            DeployError::GitHub(format!("Unexpected GitHub manifest response: {e}"))
        })?;
        Ok(ManifestConversion {
            id: raw.id,
            slug: raw.slug,
            name: raw.name,
            owner_login: raw.owner.login,
            owner_type: raw.owner.kind.unwrap_or_else(|| "User".to_owned()),
            html_url: raw.html_url,
            pem: Zeroizing::new(raw.pem),
            webhook_secret: raw.webhook_secret.map(Zeroizing::new),
        })
    }

    /// List installations for the authenticated GitHub App (`Authorization: Bearer <jwt>`).
    pub fn list_installations(&self, app_jwt: &str) -> Result<Vec<AppInstallation>, DeployError> {
        let url = self.api_url("/app/installations?per_page=100");
        let body = self.get_json(&url, Some(app_jwt))?;
        let raw: Vec<RawInstallation> = serde_json::from_str(&body).map_err(|e| {
            DeployError::GitHub(format!("Unexpected GitHub installations response: {e}"))
        })?;
        Ok(raw
            .into_iter()
            .map(|i| AppInstallation {
                id: i.id,
                account_login: i.account.login,
                account_type: i.account.kind.unwrap_or_else(|| "User".to_owned()),
                html_url: i.html_url,
                repository_selection: i.repository_selection,
            })
            .collect())
    }

    /// Update the GitHub App's webhook configuration (`PATCH /app/hook/config`).
    pub fn update_webhook_config(
        &self,
        app_jwt: &str,
        webhook_url: &str,
        webhook_secret: &str,
    ) -> Result<(), DeployError> {
        let url = self.api_url("/app/hook/config");
        let payload = serde_json::to_vec(&serde_json::json!({
            "url": webhook_url,
            "content_type": "json",
            "secret": webhook_secret,
            "insecure_ssl": "0",
        }))
        .map_err(|e| DeployError::Internal(format!("webhook payload error: {e}")))?;
        let resp = self
            .agent
            .patch(&url)
            .header("User-Agent", "opsbox")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("Authorization", &format!("Bearer {app_jwt}"))
            .header("Content-Type", "application/json")
            .send(payload)
            .map_err(|e| DeployError::GitHub(format!("Could not reach GitHub: {e}")))?;

        let status = resp.status().as_u16();
        let body = resp
            .into_body()
            .with_config()
            .limit(256 * 1024)
            .read_to_string()
            .unwrap_or_default();
        if !(200..300).contains(&status) {
            return Err(github_http_error(status, &body));
        }
        Ok(())
    }

    /// Mint an installation access token (`POST /app/installations/{id}/access_tokens`).
    pub fn create_installation_token(
        &self,
        app_jwt: &str,
        installation_id: i64,
    ) -> Result<Zeroizing<String>, DeployError> {
        let url = self.api_url(&format!(
            "/app/installations/{installation_id}/access_tokens"
        ));
        let resp = self
            .agent
            .post(&url)
            .header("User-Agent", "opsbox")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("Authorization", &format!("Bearer {app_jwt}"))
            .send_empty()
            .map_err(|e| DeployError::GitHub(format!("Could not reach GitHub: {e}")))?;

        let status = resp.status().as_u16();
        let body = resp
            .into_body()
            .with_config()
            .limit(256 * 1024)
            .read_to_string()
            .map_err(|e| DeployError::GitHub(format!("Could not read GitHub response: {e}")))?;
        if !(200..300).contains(&status) {
            return Err(github_http_error(status, &body));
        }
        let parsed: RawAccessToken = serde_json::from_str(&body)
            .map_err(|e| DeployError::GitHub(format!("Unexpected GitHub token response: {e}")))?;
        Ok(Zeroizing::new(parsed.token))
    }

    /// List repositories accessible to the GitHub App installation (up to 5 pages / 500 repos).
    pub fn list_installation_repos(
        &self,
        installation_token: &str,
    ) -> Result<Vec<GitHubRepoMeta>, DeployError> {
        let mut all = Vec::new();
        for page in 1..=5 {
            let url = self.api_url(&format!(
                "/installation/repositories?per_page=100&page={page}"
            ));
            let body = self.get_json(&url, Some(installation_token))?;
            let parsed: RawInstallationRepos = serde_json::from_str(&body).map_err(|e| {
                DeployError::GitHub(format!("Unexpected GitHub repositories response: {e}"))
            })?;
            let count = parsed.repositories.len();
            for r in parsed.repositories {
                all.push(into_repo_meta(r));
            }
            if count < 100 || all.len() >= parsed.total_count {
                break;
            }
        }
        all.sort_by_key(|a| std::cmp::Reverse(a.pushed_at));
        Ok(all)
    }

    /// Inspect a public GitHub repository (`owner/repo`).
    pub fn inspect_public_repo(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<PublicGitHubRepo, DeployError> {
        let url = self.api_url(&format!("/repos/{owner}/{repo}"));
        let body = self.get_json(&url, None)?;
        let raw: RawRepo = serde_json::from_str(&body).map_err(|e| {
            DeployError::GitHub(format!("Unexpected GitHub repository response: {e}"))
        })?;
        if raw.private {
            return Err(DeployError::Validation {
                field: Some("url"),
                message: "This repository is private. Use the GitHub App or a Deploy key instead."
                    .into(),
            });
        }
        let meta = into_repo_meta(raw);
        let build =
            self.detect_repo_build(None, &meta.owner, &meta.repo_name, meta.language.as_deref());
        Ok(PublicGitHubRepo { meta, build })
    }

    /// Detect the build method for `owner/repo` using GitHub's root contents API (plus `package.json`
    /// when present). Falls back cleanly to language heuristics if contents lookup fails.
    pub fn detect_repo_build(
        &self,
        bearer_token: Option<&str>,
        owner: &str,
        repo: &str,
        language: Option<&str>,
    ) -> String {
        let url = self.api_url(&format!("/repos/{owner}/{repo}/contents/"));
        let Ok(body) = self.get_json(&url, bearer_token) else {
            return detect_build_method(&[], None, language);
        };
        let Ok(entries) = serde_json::from_str::<Vec<RawContentEntry>>(&body) else {
            return detect_build_method(&[], None, language);
        };
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        let pkg_json = if names.contains(&"package.json") {
            self.fetch_raw_file(bearer_token, owner, repo, "package.json")
        } else {
            None
        };
        detect_build_method(&names, pkg_json.as_deref(), language)
    }

    fn fetch_raw_file(
        &self,
        bearer_token: Option<&str>,
        owner: &str,
        repo: &str,
        path: &str,
    ) -> Option<String> {
        let url = self.api_url(&format!("/repos/{owner}/{repo}/contents/{path}"));
        let mut req = self
            .agent
            .get(&url)
            .header("User-Agent", "opsbox")
            .header("Accept", "application/vnd.github.raw+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(tok) = bearer_token {
            req = req.header("Authorization", &format!("Bearer {tok}"));
        }
        let resp = req.call().ok()?;
        if !(200..300).contains(&resp.status().as_u16()) {
            return None;
        }
        resp.into_body()
            .with_config()
            .limit(256 * 1024)
            .read_to_string()
            .ok()
    }

    fn get_json(&self, url: &str, bearer_token: Option<&str>) -> Result<String, DeployError> {
        let mut req = self
            .agent
            .get(url)
            .header("User-Agent", "opsbox")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(tok) = bearer_token {
            req = req.header("Authorization", &format!("Bearer {tok}"));
        }
        let resp = req
            .call()
            .map_err(|e| DeployError::GitHub(format!("Could not reach GitHub: {e}")))?;
        let status = resp.status().as_u16();
        let body = resp
            .into_body()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| DeployError::GitHub(format!("Could not read GitHub response: {e}")))?;
        if !(200..300).contains(&status) {
            return Err(github_http_error(status, &body));
        }
        Ok(body)
    }
}

fn github_http_error(status: u16, body: &str) -> DeployError {
    #[derive(Deserialize)]
    struct GhErr {
        message: Option<String>,
    }
    let detail = serde_json::from_str::<GhErr>(body)
        .ok()
        .and_then(|e| e.message)
        .unwrap_or_default();
    match status {
        401 => DeployError::GitHub(
            "GitHub rejected the app credentials. Try reconnecting GitHub in Settings.".into(),
        ),
        403 if detail.to_ascii_lowercase().contains("rate limit") => DeployError::GitHub(
            "GitHub API rate limit reached. Wait a few minutes and try again.".into(),
        ),
        404 => DeployError::NotFound(if detail.is_empty() {
            "GitHub resource not found.".into()
        } else {
            format!("GitHub returned not found: {detail}")
        }),
        _ => DeployError::GitHub(if detail.is_empty() {
            format!("GitHub returned HTTP {status}.")
        } else {
            format!("GitHub returned HTTP {status}: {detail}")
        }),
    }
}

pub struct ManifestConversion {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub owner_login: String,
    pub owner_type: String,
    pub html_url: String,
    pub pem: Zeroizing<String>,
    pub webhook_secret: Option<Zeroizing<String>>,
}

impl std::fmt::Debug for ManifestConversion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManifestConversion")
            .field("id", &self.id)
            .field("slug", &self.slug)
            .field("owner_login", &self.owner_login)
            .field("pem", &"[redacted]")
            .field("webhook_secret", &"[redacted]")
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct AppInstallation {
    pub id: i64,
    pub account_login: String,
    pub account_type: String,
    pub html_url: Option<String>,
    pub repository_selection: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GitHubRepoMeta {
    pub full_name: String,
    pub owner: String,
    pub repo_name: String,
    pub private: bool,
    pub language: Option<String>,
    pub default_branch: String,
    pub html_url: String,
    pub pushed_at_raw: Option<String>,
    pub pushed_at: i64,
}

#[derive(Debug, Clone)]
pub struct PublicGitHubRepo {
    pub meta: GitHubRepoMeta,
    pub build: String,
}

#[derive(Deserialize)]
struct RawManifestConversion {
    id: i64,
    slug: String,
    name: String,
    owner: RawOwner,
    html_url: String,
    pem: String,
    webhook_secret: Option<String>,
}

#[derive(Deserialize)]
struct RawOwner {
    login: String,
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Deserialize)]
struct RawInstallation {
    id: i64,
    account: RawOwner,
    html_url: Option<String>,
    repository_selection: Option<String>,
}

#[derive(Deserialize)]
struct RawAccessToken {
    token: String,
}

#[derive(Deserialize)]
struct RawInstallationRepos {
    #[serde(default)]
    total_count: usize,
    repositories: Vec<RawRepo>,
}

#[derive(Deserialize)]
struct RawRepo {
    full_name: String,
    name: String,
    owner: RawOwner,
    private: bool,
    language: Option<String>,
    default_branch: Option<String>,
    html_url: Option<String>,
    pushed_at: Option<String>,
}

#[derive(Deserialize)]
struct RawContentEntry {
    name: String,
}

fn into_repo_meta(r: RawRepo) -> GitHubRepoMeta {
    let pushed_at = r
        .pushed_at
        .as_deref()
        .and_then(|s| OffsetDateTime::parse(s, &Rfc3339).ok())
        .map_or(0, OffsetDateTime::unix_timestamp);
    let html_url = r
        .html_url
        .unwrap_or_else(|| format!("https://github.com/{}", r.full_name));
    GitHubRepoMeta {
        full_name: r.full_name,
        owner: r.owner.login,
        repo_name: r.name,
        private: r.private,
        language: r.language,
        default_branch: r.default_branch.unwrap_or_else(|| "main".to_owned()),
        html_url,
        pushed_at_raw: r.pushed_at,
        pushed_at,
    }
}

/// Format elapsed seconds as `"2 min ago"`, `"4 days ago"`, `"never"`.
pub fn format_pushed_ago(pushed_at: i64, now_secs: i64) -> String {
    if pushed_at <= 0 {
        return "unknown".to_owned();
    }
    let delta = now_secs.saturating_sub(pushed_at);
    match delta {
        s if s < 60 => "just now".to_owned(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86_400 => format!("{} h ago", s / 3600),
        s if s < 7 * 86_400 => {
            let d = s / 86_400;
            if d == 1 {
                "1 day ago".to_owned()
            } else {
                format!("{d} days ago")
            }
        }
        s if s < 30 * 86_400 => {
            let w = s / (7 * 86_400);
            if w == 1 {
                "last week".to_owned()
            } else {
                format!("{w} weeks ago")
            }
        }
        s => {
            let m = (s / (30 * 86_400)).max(1);
            if m == 1 {
                "1 month ago".to_owned()
            } else {
                format!("{m} months ago")
            }
        }
    }
}
