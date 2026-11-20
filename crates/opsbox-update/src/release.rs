//! GitHub Releases lookup, archive download, SHA-256 and minisign verification.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use flate2::read::GzDecoder;
use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tar::Archive;

use crate::{MINISIGN_PUBKEY, UpdateError};

pub const DEFAULT_REPO: &str = "khangdev20/opsbox";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    #[default]
    Stable,
    Beta,
}

impl Channel {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "stable" => Some(Self::Stable),
            "beta" => Some(Self::Beta),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub version: String,
    pub tag: String,
    pub prerelease: bool,
    pub notes_url: String,
    pub published_at: Option<String>,
}

/// Why a check offered nothing. `None` on a [`CheckResult`] means an update is offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoOfferReason {
    /// The channel has no release at all (stable while only pre-releases exist).
    NoReleasesOnChannel,
    /// The newest release on the channel is older than or equal to the running version.
    UpToDate,
    /// GitHub could not be reached, or sent something unusable.
    NetworkError,
    /// GitHub is limiting requests; `retry_at` says when to ask again.
    RateLimited,
    /// The running build is a local development build.
    DevBuild,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckResult {
    pub current_version: String,
    pub channel: Channel,
    pub latest: Option<ReleaseInfo>,
    pub update_available: bool,
    pub checked_at: i64,
    /// The running build predates version tagging (it reports a plain `0.1.0`), so the
    /// offered pre-release only looks like a downgrade to semver.
    #[serde(default)]
    pub migration: bool,
    /// The running build is a local development build; it is never offered updates.
    #[serde(default)]
    pub dev_build: bool,
    /// Why nothing is offered; `None` when `update_available`.
    #[serde(default)]
    pub reason: Option<NoOfferReason>,
    /// Technical detail for `network_error` (what went wrong), shown under the plain text.
    #[serde(default)]
    pub detail: Option<String>,
    /// Unix seconds when GitHub may be asked again (`rate_limited`).
    #[serde(default)]
    pub retry_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    html_url: String,
    published_at: Option<String>,
}

/// Target triple for release downloads on this CPU architecture.
pub fn current_target() -> Result<&'static str, UpdateError> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("x86_64-unknown-linux-musl"),
        "aarch64" => Ok("aarch64-unknown-linux-musl"),
        other => Err(UpdateError::UnsupportedArch(other.to_owned())),
    }
}

fn releases_url() -> String {
    if let Ok(u) = std::env::var("OPSBOX_RELEASES_URL")
        && !u.is_empty()
    {
        return u;
    }
    let repo = std::env::var("OPSBOX_GITHUB_REPO").unwrap_or_else(|_| DEFAULT_REPO.to_owned());
    format!("https://api.github.com/repos/{repo}/releases?per_page=20")
}

fn release_download_base(version: &str) -> String {
    if let Ok(b) = std::env::var("OPSBOX_RELEASE_BASE")
        && !b.is_empty()
    {
        return b.trim_end_matches('/').to_owned();
    }
    let repo = std::env::var("OPSBOX_GITHUB_REPO").unwrap_or_else(|_| DEFAULT_REPO.to_owned());
    let v = version.trim_start_matches('v');
    format!("https://github.com/{repo}/releases/download/v{v}")
}

/// Pick the highest matching release from a GitHub Releases JSON payload.
///
/// * A development build (`0.1.0-dev+<sha>`) is never offered anything.
/// * An old build that reports a plain `0.1.0` while the newest release on the channel is
///   a `0.1.0-alpha.N` is offered that alpha once (`migration`), because semver ranks
///   `0.1.0-alpha.N` below `0.1.0` even though the build is older.
pub fn select_release(
    current_version: &str,
    channel: Channel,
    releases_json: &str,
    checked_at: i64,
) -> Result<CheckResult, UpdateError> {
    let clean_current = current_version.trim_start_matches('v');
    let cur = Version::parse(clean_current)
        .map_err(|e| UpdateError::InvalidVersion(format!("{current_version}: {e}")))?;
    let items: Vec<GhRelease> =
        serde_json::from_str(releases_json).map_err(|e| UpdateError::Http(e.to_string()))?;

    let mut best: Option<(Version, ReleaseInfo)> = None;
    for r in items {
        if r.draft {
            continue;
        }
        if channel == Channel::Stable && r.prerelease {
            continue;
        }
        let ver_str = r.tag_name.trim_start_matches('v');
        let Ok(ver) = Version::parse(ver_str) else {
            continue;
        };
        let is_newer = best.as_ref().is_none_or(|(b, _)| ver > *b);
        if is_newer {
            best = Some((
                ver,
                ReleaseInfo {
                    version: ver_str.to_owned(),
                    tag: r.tag_name,
                    prerelease: r.prerelease,
                    notes_url: r.html_url,
                    published_at: r.published_at,
                },
            ));
        }
    }

    let mut result = CheckResult {
        current_version: clean_current.to_owned(),
        channel,
        latest: None,
        update_available: false,
        checked_at,
        migration: false,
        dev_build: false,
        reason: None,
        detail: None,
        retry_at: None,
    };
    if opsbox_version::is_dev(clean_current) {
        result.dev_build = true;
        result.reason = Some(NoOfferReason::DevBuild);
        return Ok(result);
    }
    if let Some((v, info)) = best {
        result.migration = channel == Channel::Beta
            && cur.pre.is_empty()
            && !v.pre.is_empty()
            && (v.major, v.minor, v.patch) == (cur.major, cur.minor, cur.patch);
        result.update_available = v > cur || result.migration;
        result.latest = Some(info);
    }
    result.reason = if result.update_available {
        None
    } else if result.latest.is_none() {
        Some(NoOfferReason::NoReleasesOnChannel)
    } else {
        Some(NoOfferReason::UpToDate)
    };
    Ok(result)
}

/// Like [`select_release`], for an install whose channel was never chosen. An old build
/// (plain `0.1.0`) has no stable release to move to, so when the stable channel is empty
/// the beta channel is checked, and only a migration offer from it is used.
pub fn select_release_default_channel(
    current_version: &str,
    releases_json: &str,
    checked_at: i64,
) -> Result<CheckResult, UpdateError> {
    let channel = default_channel(current_version);
    let res = select_release(current_version, channel, releases_json, checked_at)?;
    if channel == Channel::Stable && res.latest.is_none() && !res.dev_build {
        let beta = select_release(current_version, Channel::Beta, releases_json, checked_at)?;
        if beta.migration {
            return Ok(beta);
        }
    }
    Ok(res)
}

/// The channel to use when neither the panel setting nor the config file names one:
/// beta while the running version is a pre-release, stable otherwise.
pub fn default_channel(current_version: &str) -> Channel {
    match Version::parse(current_version.trim_start_matches('v')) {
        Ok(v) if !v.pre.is_empty() => Channel::Beta,
        _ => Channel::Stable,
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// How long a fetched release list is reused before GitHub is asked again.
pub const CACHE_TTL_SECS: i64 = 3600;
/// Shortest wait after a rate-limit answer, whatever the headers say.
const MIN_BACKOFF_SECS: i64 = 60;
/// Longest wait after a rate-limit answer (GitHub's window is an hour).
const MAX_BACKOFF_SECS: i64 = 3600;

/// One answer from the releases endpoint, with the headers that matter for rate limits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HttpReply {
    pub status: u16,
    pub body: String,
    /// `X-RateLimit-Remaining`.
    pub rate_limit_remaining: Option<i64>,
    /// `X-RateLimit-Reset`, Unix seconds.
    pub rate_limit_reset: Option<i64>,
    /// `Retry-After`, seconds.
    pub retry_after: Option<i64>,
}

/// Fetches the releases list. `Err` is a transport failure (DNS, connect, TLS, timeout).
pub trait ReleaseFetcher: Send + Sync {
    fn get(&self, url: &str) -> Result<HttpReply, String>;
}

/// The real thing: `User-Agent` set, non-2xx answers returned (not turned into errors) so the
/// rate-limit headers can be read.
pub struct HttpFetcher;

impl ReleaseFetcher for HttpFetcher {
    fn get(&self, url: &str) -> Result<HttpReply, String> {
        if let Some(path) = url.strip_prefix("file://") {
            return fs::read_to_string(path)
                .map(|body| HttpReply {
                    status: 200,
                    body,
                    ..HttpReply::default()
                })
                .map_err(|e| e.to_string());
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(20)))
            .build()
            .into();
        let resp = agent
            .get(url)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|e| e.to_string())?;
        let header = |name: &str| -> Option<i64> {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse().ok())
        };
        let status = resp.status().as_u16();
        let (remaining, reset, retry_after) = (
            header("x-ratelimit-remaining"),
            header("x-ratelimit-reset"),
            header("retry-after"),
        );
        let body = resp
            .into_body()
            .read_to_string()
            .map_err(|e| e.to_string())?;
        Ok(HttpReply {
            status,
            body,
            rate_limit_remaining: remaining,
            rate_limit_reset: reset,
            retry_after,
        })
    }
}

const USER_AGENT: &str = "opsbox-updater";

struct CachedBody {
    url: String,
    body: String,
    fetched_at: i64,
}

#[derive(Default)]
struct CheckerState {
    cache: Option<CachedBody>,
    /// GitHub said "too many requests"; do not ask again before this time.
    backoff_until: Option<i64>,
}

/// Why the release list could not be had.
struct Unavailable {
    reason: NoOfferReason,
    detail: Option<String>,
    retry_at: Option<i64>,
}

impl Unavailable {
    fn network(detail: impl Into<String>) -> Self {
        Self {
            reason: NoOfferReason::NetworkError,
            detail: Some(detail.into()),
            retry_at: None,
        }
    }
}

/// Looks up releases with a one-hour cache and rate-limit backoff. A failed lookup is a
/// [`CheckResult`] with a `reason`, not an error, so callers can always say why.
pub struct ReleaseChecker {
    fetcher: Arc<dyn ReleaseFetcher>,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    state: Mutex<CheckerState>,
}

impl Default for ReleaseChecker {
    fn default() -> Self {
        Self::new(Arc::new(HttpFetcher), Arc::new(now_secs))
    }
}

impl ReleaseChecker {
    pub fn new(
        fetcher: Arc<dyn ReleaseFetcher>,
        clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    ) -> Self {
        Self {
            fetcher,
            clock,
            state: Mutex::new(CheckerState::default()),
        }
    }

    /// Check `channel` (or, with `None`, the default channel for this version, see
    /// [`select_release_default_channel`]). `force` ("Check now") skips the cache; it does
    /// not skip a rate-limit backoff, because asking again would only extend it.
    pub fn check(
        &self,
        current_version: &str,
        channel: Option<Channel>,
        force: bool,
    ) -> Result<CheckResult, UpdateError> {
        let now = (self.clock)();
        let pick = |body: &str, at: i64| match channel {
            Some(ch) => select_release(current_version, ch, body, at),
            None => select_release_default_channel(current_version, body, at),
        };
        // A development build is never offered anything, so there is nothing to ask GitHub.
        if opsbox_version::is_dev(current_version.trim_start_matches('v')) {
            return pick("[]", now);
        }
        match self.releases(force, now) {
            Ok((body, fetched_at)) => match pick(&body, fetched_at) {
                Ok(res) => Ok(res),
                Err(UpdateError::InvalidVersion(e)) => Err(UpdateError::InvalidVersion(e)),
                Err(_) => self.failed(
                    current_version,
                    channel,
                    now,
                    Unavailable::network("GitHub sent a reply opsbox could not read"),
                ),
            },
            Err(why) => self.failed(current_version, channel, now, why),
        }
    }

    fn failed(
        &self,
        current_version: &str,
        channel: Option<Channel>,
        now: i64,
        why: Unavailable,
    ) -> Result<CheckResult, UpdateError> {
        let mut res = match channel {
            Some(ch) => select_release(current_version, ch, "[]", now)?,
            None => select_release_default_channel(current_version, "[]", now)?,
        };
        res.reason = Some(why.reason);
        res.detail = why.detail;
        res.retry_at = why.retry_at;
        Ok(res)
    }

    /// The release list and the time it was fetched.
    fn releases(&self, force: bool, now: i64) -> Result<(String, i64), Unavailable> {
        let url = releases_url();
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !force
            && let Some(c) = st.cache.as_ref().filter(|c| c.url == url)
            && now - c.fetched_at < CACHE_TTL_SECS
        {
            return Ok((c.body.clone(), c.fetched_at));
        }
        if let Some(until) = st.backoff_until {
            if until > now {
                return Err(Unavailable {
                    reason: NoOfferReason::RateLimited,
                    detail: None,
                    retry_at: Some(until),
                });
            }
            st.backoff_until = None;
        }
        let reply = self.fetcher.get(&url).map_err(Unavailable::network)?;
        match reply.status {
            200..=299 => {
                st.cache = Some(CachedBody {
                    url,
                    body: reply.body.clone(),
                    fetched_at: now,
                });
                Ok((reply.body, now))
            }
            403 | 429
                if reply.rate_limit_remaining == Some(0)
                    || reply.rate_limit_reset.is_some()
                    || reply.retry_after.is_some() =>
            {
                let wait_until = match (reply.retry_after, reply.rate_limit_reset) {
                    (Some(secs), _) => now + secs,
                    (None, Some(reset)) => reset,
                    (None, None) => now + MAX_BACKOFF_SECS,
                };
                let until = wait_until.clamp(now + MIN_BACKOFF_SECS, now + MAX_BACKOFF_SECS);
                st.backoff_until = Some(until);
                Err(Unavailable {
                    reason: NoOfferReason::RateLimited,
                    detail: None,
                    retry_at: Some(until),
                })
            }
            code => Err(Unavailable::network(format!("GitHub answered HTTP {code}"))),
        }
    }
}

/// Query GitHub Releases for the latest version on `channel`. Uses a fresh [`ReleaseChecker`],
/// so it always asks GitHub; a long-running caller should keep its own checker instead.
pub fn check_latest(current_version: &str, channel: Channel) -> Result<CheckResult, UpdateError> {
    ReleaseChecker::default().check(current_version, Some(channel), true)
}

/// Query GitHub Releases for an install whose channel was never chosen
/// (see [`select_release_default_channel`]).
pub fn check_latest_default_channel(current_version: &str) -> Result<CheckResult, UpdateError> {
    ReleaseChecker::default().check(current_version, None, true)
}

fn http_get_text(url: &str) -> Result<String, UpdateError> {
    if let Some(path) = url.strip_prefix("file://") {
        return fs::read_to_string(path).map_err(UpdateError::Io);
    }
    let resp = ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| UpdateError::Http(e.to_string()))?;
    resp.into_body()
        .read_to_string()
        .map_err(|e| UpdateError::Http(e.to_string()))
}

fn http_get_bytes(url: &str) -> Result<Vec<u8>, UpdateError> {
    if let Some(path) = url.strip_prefix("file://") {
        return fs::read(path).map_err(UpdateError::Io);
    }
    let resp = ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| UpdateError::Http(e.to_string()))?;
    resp.into_body()
        .with_config()
        .limit(256 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| UpdateError::Http(e.to_string()))
}

/// Verify `archive_bytes` against the `<hex>  <filename>` `.sha256` file content.
pub fn verify_sha256(archive_bytes: &[u8], sha256_text: &str) -> Result<(), UpdateError> {
    let expected = sha256_text
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(UpdateError::Checksum(
            "invalid .sha256 file format".to_owned(),
        ));
    }
    let digest = Sha256::digest(archive_bytes);
    let actual: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    if actual != expected {
        return Err(UpdateError::Checksum(format!(
            "checksum mismatch (expected {expected}, got {actual})"
        )));
    }
    Ok(())
}

/// Verify `archive_bytes` against a `.minisig` signature using `pubkey_b64`.
pub fn verify_minisign(
    archive_bytes: &[u8],
    minisig_text: &str,
    pubkey_b64: &str,
) -> Result<(), UpdateError> {
    let pk = PublicKey::from_base64(pubkey_b64.trim())
        .map_err(|e| UpdateError::Signature(format!("invalid public key: {e}")))?;
    let sig = Signature::decode(minisig_text)
        .map_err(|e| UpdateError::Signature(format!("invalid signature format: {e}")))?;
    pk.verify(archive_bytes, &sig, false)
        .map_err(|e| UpdateError::Signature(format!("signature verification failed: {e}")))
}

/// Download `opsbox-<version>-<target>.tar.gz`, verify SHA-256 + minisign signature,
/// extract the `opsbox` binary into `staging_dir`, and run `selftest` on it.
pub fn download_and_verify(
    version: &str,
    staging_dir: &Path,
    pubkey_b64: Option<&str>,
) -> Result<PathBuf, UpdateError> {
    let target = current_target()?;
    download_and_verify_for_target(
        version,
        target,
        staging_dir,
        pubkey_b64.unwrap_or(MINISIGN_PUBKEY),
        true,
    )
}

pub fn download_and_verify_for_target(
    version: &str,
    target: &str,
    staging_dir: &Path,
    pubkey_b64: &str,
    run_selftest: bool,
) -> Result<PathBuf, UpdateError> {
    let clean_ver = version.trim_start_matches('v');
    let pkg = format!("opsbox-{clean_ver}-{target}");
    let base = release_download_base(clean_ver);

    let archive_bytes = http_get_bytes(&format!("{base}/{pkg}.tar.gz"))?;
    let sha_text = http_get_text(&format!("{base}/{pkg}.tar.gz.sha256"))?;
    let sig_text = http_get_text(&format!("{base}/{pkg}.tar.gz.minisig"))?;

    verify_sha256(&archive_bytes, &sha_text)?;
    verify_minisign(&archive_bytes, &sig_text, pubkey_b64)?;

    // The installer/root helper owns this directory. Never create it as the
    // panel user or follow a planted directory symlink during an update.
    if !fs::symlink_metadata(staging_dir)?.file_type().is_dir() {
        return Err(std::io::Error::other(
            "The transfer folder must be a real directory; re-run the installer.",
        )
        .into());
    }
    let bin_name = format!("opsbox-update-{clean_ver}.bin");
    let staged_path = staging_dir.join(bin_name);
    let expected_entry = format!("{pkg}/opsbox");

    let decoder = GzDecoder::new(&archive_bytes[..]);
    let mut archive = Archive::new(decoder);
    let mut extracted = false;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        if path == Path::new(&expected_entry) {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            let mut f = File::create(&staged_path)?;
            f.write_all(&buf)?;
            f.sync_all()?;
            fs::set_permissions(&staged_path, fs::Permissions::from_mode(0o755))?;
            extracted = true;
            break;
        }
    }
    if !extracted {
        return Err(UpdateError::Archive(format!(
            "archive did not contain {expected_entry}"
        )));
    }

    if run_selftest {
        let out = Command::new(&staged_path).arg("selftest").output()?;
        if !out.status.success() {
            let _ = fs::remove_file(&staged_path);
            return Err(UpdateError::Selftest(
                String::from_utf8_lossy(&out.stderr).trim().to_owned(),
            ));
        }
    }

    Ok(staged_path)
}
