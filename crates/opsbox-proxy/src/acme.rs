//! ACME HTTP-01 issuance, renewal (30 days before expiry), encrypted key storage via
//! `opsbox-priv`, and live TLS certificate resolution for `rustls`.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, ChallengeType, HttpClient, Identifier,
    LetsEncrypt, NewAccount, NewOrder, OrderStatus, RetryPolicy,
};
use opsbox_priv::PrivClient;
use opsbox_proto::Secret;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;
use tokio_rustls::rustls::crypto::ring;
use tokio_rustls::rustls::server::{ClientHello, ResolvesServerCert};
use tokio_rustls::rustls::sign::CertifiedKey;
use tracing::{error, info, warn};
use x509_parser::prelude::*;
use zeroize::Zeroizing;

use crate::domain::validate_domain;
use crate::proto::{CertInfo, CertState};

/// Renew certificates when 30 days or less remain before `not_after`.
pub const RENEW_BEFORE_SECS: i64 = 30 * 86_400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcmeDirectory {
    Production,
    Staging,
    Custom(String),
}

impl AcmeDirectory {
    pub fn url(&self) -> &str {
        match self {
            Self::Production => LetsEncrypt::Production.url(),
            Self::Staging => LetsEncrypt::Staging.url(),
            Self::Custom(u) => u.as_str(),
        }
    }

    pub fn is_staging(&self) -> bool {
        matches!(self, Self::Staging)
    }

    fn profile_key(&self) -> &'static str {
        match self {
            Self::Production => "prod",
            Self::Staging => "staging",
            Self::Custom(_) => "custom",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AcmeError {
    #[error("{0}")]
    Invalid(String),
    #[error("helper encryption error: {0}")]
    Priv(String),
    #[error("storage error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ACME failed: {0}")]
    Acme(String),
    #[error("certificate error: {0}")]
    Cert(String),
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// Pure renewal check: returns `true` when 30 days or fewer remain before `not_after`.
pub fn needs_renewal(not_after: i64, now: i64) -> bool {
    not_after.saturating_sub(now) <= RENEW_BEFORE_SECS
}

/// Extracted X.509 certificate metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertMeta {
    pub not_before: i64,
    pub not_after: i64,
    pub issuer: String,
}

/// Parse the leaf certificate's validity window and issuer from a PEM chain.
pub fn parse_cert_metadata(cert_pem: &str) -> Result<CertMeta, AcmeError> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AcmeError::Cert(format!("invalid certificate PEM: {e}")))?;
    let leaf = certs
        .first()
        .ok_or_else(|| AcmeError::Cert("certificate chain is empty".into()))?;
    let (_, parsed) = X509Certificate::from_der(leaf.as_ref())
        .map_err(|e| AcmeError::Cert(format!("cannot parse X.509 certificate: {e}")))?;
    let validity = parsed.validity();
    let issuer = parsed
        .issuer()
        .iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok())
        .or_else(|| {
            parsed
                .issuer()
                .iter_organization()
                .next()
                .and_then(|org| org.as_str().ok())
        })
        .unwrap_or("Let's Encrypt")
        .to_owned();
    Ok(CertMeta {
        not_before: validity.not_before.timestamp(),
        not_after: validity.not_after.timestamp(),
        issuer,
    })
}

/// Dynamic SNI certificate table used by `tokio-rustls`.
#[derive(Default)]
pub struct DynamicCertResolver {
    certs: RwLock<HashMap<String, Arc<CertifiedKey>>>,
}

impl fmt::Debug for DynamicCertResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DynamicCertResolver")
            .finish_non_exhaustive()
    }
}

impl DynamicCertResolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_pem(&self, domain: &str, cert_pem: &str, key_pem: &str) -> Result<(), AcmeError> {
        let certs: Vec<CertificateDer<'static>> =
            CertificateDer::pem_slice_iter(cert_pem.as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| AcmeError::Cert(format!("invalid certificate PEM: {e}")))?;
        if certs.is_empty() {
            return Err(AcmeError::Cert("no certificates in PEM".into()));
        }
        let key_der = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())
            .map_err(|e| AcmeError::Cert(format!("invalid private key PEM: {e}")))?;
        let signing_key = ring::sign::any_supported_type(&key_der)
            .map_err(|e| AcmeError::Cert(format!("unsupported private key: {e}")))?;
        let ck = Arc::new(CertifiedKey::new(certs, signing_key));
        let mut guard = self
            .certs
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.insert(domain.to_ascii_lowercase(), ck);
        Ok(())
    }

    pub fn remove(&self, domain: &str) {
        let mut guard = self
            .certs
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.remove(&domain.to_ascii_lowercase());
    }

    pub fn has(&self, domain: &str) -> bool {
        let guard = self
            .certs
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.contains_key(&domain.to_ascii_lowercase())
    }
}

impl ResolvesServerCert for DynamicCertResolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let name = client_hello.server_name()?.to_ascii_lowercase();
        let guard = self
            .certs
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.get(&name).cloned()
    }
}

/// Active HTTP-01 challenge responses keyed by ACME token.
#[derive(Clone, Default)]
pub struct ChallengeMap {
    inner: Arc<RwLock<HashMap<String, String>>>,
}

impl ChallengeMap {
    pub fn insert(&self, token: String, key_auth: String) {
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.insert(token, key_auth);
    }

    pub fn get(&self, token: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.get(token).cloned()
    }

    pub fn remove(&self, token: &str) {
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.remove(token);
    }
}

struct ChallengeGuard {
    map: ChallengeMap,
    tokens: Vec<String>,
}

impl Drop for ChallengeGuard {
    fn drop(&mut self) {
        for t in &self.tokens {
            self.map.remove(t);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedCert {
    domain: String,
    cert_pem: String,
    key_sealed: String,
    not_before: i64,
    not_after: i64,
    issuer: String,
    staging: bool,
    issued_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedAccount {
    sealed: String,
}

#[derive(Debug, Clone, Default)]
struct DomainRuntime {
    consecutive_failures: u32,
    last_error: Option<String>,
    next_retry_at: i64,
    persisted: Option<PersistedCert>,
}

pub type CustomHttpFactory = Arc<dyn Fn() -> Box<dyn HttpClient> + Send + Sync>;

pub struct AcmeManager {
    state_dir: PathBuf,
    directory: AcmeDirectory,
    contact_email: Option<String>,
    priv_client: PrivClient,
    resolver: Arc<DynamicCertResolver>,
    challenges: ChallengeMap,
    custom_http: Option<CustomHttpFactory>,
    issue_lock: AsyncMutex<()>,
    domains: RwLock<HashMap<String, DomainRuntime>>,
}

impl AcmeManager {
    pub fn new(
        state_dir: impl Into<PathBuf>,
        directory: AcmeDirectory,
        contact_email: Option<String>,
        priv_client: PrivClient,
        resolver: Arc<DynamicCertResolver>,
        challenges: ChallengeMap,
    ) -> Self {
        Self {
            state_dir: state_dir.into(),
            directory,
            contact_email,
            priv_client,
            resolver,
            challenges,
            custom_http: None,
            issue_lock: AsyncMutex::new(()),
            domains: RwLock::new(HashMap::new()),
        }
    }

    #[must_use]
    pub fn with_custom_http(mut self, factory: CustomHttpFactory) -> Self {
        self.custom_http = Some(factory);
        self
    }

    fn certs_dir(&self) -> PathBuf {
        self.state_dir.join("certs")
    }

    fn cert_path(&self, domain: &str) -> Result<PathBuf, AcmeError> {
        validate_domain(domain).map_err(|e| AcmeError::Invalid(e.0))?;
        Ok(self.certs_dir().join(format!("{domain}.json")))
    }

    fn account_path(&self) -> PathBuf {
        self.state_dir.join(format!(
            "acme-account-{}.json",
            self.directory.profile_key()
        ))
    }

    /// Load all persisted encrypted certificates from disk and unlock them via `priv`.
    /// Returns the number of certificates loaded; logs warnings if `priv` is temporarily
    /// unreachable so the proxy can retry later.
    pub async fn load_persisted(&self) -> Result<usize, AcmeError> {
        let dir = self.certs_dir();
        if !dir.exists() {
            return Ok(0);
        }
        let mut loaded = 0usize;
        let entries = fs::read_dir(&dir)?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(rec) = serde_json::from_str::<PersistedCert>(&raw) else {
                warn!(path = %path.display(), "ignoring malformed persisted certificate record");
                continue;
            };
            if validate_domain(&rec.domain).is_err() {
                continue;
            }
            let ctx = format!("proxy:cert:{}", rec.domain);
            match self
                .priv_client
                .secret_open("proxy-boot", None, &ctx, &rec.key_sealed)
                .await
            {
                Ok(key_plain) => {
                    if let Err(e) =
                        self.resolver
                            .insert_pem(&rec.domain, &rec.cert_pem, key_plain.expose())
                    {
                        warn!(domain = %rec.domain, error = %e, "could not load persisted cert into TLS");
                    } else {
                        loaded += 1;
                    }
                    let mut guard = self
                        .domains
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    guard.insert(
                        rec.domain.clone(),
                        DomainRuntime {
                            persisted: Some(rec),
                            ..DomainRuntime::default()
                        },
                    );
                }
                Err(e) => {
                    warn!(domain = %rec.domain, error = %e, "could not decrypt persisted TLS key via priv");
                    let mut guard = self
                        .domains
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    guard.insert(
                        rec.domain.clone(),
                        DomainRuntime {
                            last_error: Some(format!("Cannot unlock certificate key: {e}")),
                            persisted: Some(rec),
                            ..DomainRuntime::default()
                        },
                    );
                }
            }
        }
        Ok(loaded)
    }

    pub fn cert_info(&self, domain: &str) -> CertInfo {
        let now = now_secs();
        let guard = self
            .domains
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match guard.get(&domain.to_ascii_lowercase()) {
            Some(rt) => match &rt.persisted {
                Some(p) => {
                    let days = (p.not_after - now) / 86_400;
                    let status = if (!self.resolver.has(&p.domain) && rt.last_error.is_some())
                        || p.not_after <= now
                    {
                        CertState::Error
                    } else if needs_renewal(p.not_after, now) {
                        CertState::Expiring
                    } else {
                        CertState::Valid
                    };
                    CertInfo {
                        domain: p.domain.clone(),
                        status,
                        not_before: Some(p.not_before),
                        not_after: Some(p.not_after),
                        days_remaining: Some(days),
                        issuer: Some(p.issuer.clone()),
                        staging: p.staging,
                        last_error: rt.last_error.clone(),
                        issued_at: Some(p.issued_at),
                    }
                }
                None => CertInfo {
                    domain: domain.to_owned(),
                    status: if rt.last_error.is_some() {
                        CertState::Error
                    } else {
                        CertState::None
                    },
                    not_before: None,
                    not_after: None,
                    days_remaining: None,
                    issuer: None,
                    staging: self.directory.is_staging(),
                    last_error: rt.last_error.clone(),
                    issued_at: None,
                },
            },
            None => CertInfo {
                domain: domain.to_owned(),
                status: CertState::None,
                not_before: None,
                not_after: None,
                days_remaining: None,
                issuer: None,
                staging: self.directory.is_staging(),
                last_error: None,
                issued_at: None,
            },
        }
    }

    pub fn list_certs(&self) -> Vec<CertInfo> {
        let names: Vec<String> = {
            let guard = self
                .domains
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.keys().cloned().collect()
        };
        let mut out: Vec<CertInfo> = names.iter().map(|d| self.cert_info(d)).collect();
        out.sort_by(|a, b| a.domain.cmp(&b.domain));
        out
    }

    pub async fn remove_cert(&self, domain: &str) -> Result<(), AcmeError> {
        let domain = domain.to_ascii_lowercase();
        let path = self.cert_path(&domain)?;
        self.resolver.remove(&domain);
        if path.exists() {
            fs::remove_file(&path)?;
        }
        let mut guard = self
            .domains
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.remove(&domain);
        Ok(())
    }

    async fn load_or_create_account(&self, rid: &str) -> Result<Account, AcmeError> {
        let profile = self.directory.profile_key();
        let ctx = format!("proxy:acme_account:{profile}");
        let path = self.account_path();

        let builder = || -> Result<instant_acme::AccountBuilder, AcmeError> {
            if let Some(ref factory) = self.custom_http {
                Ok(Account::builder_with_http(factory()))
            } else {
                Account::builder().map_err(|e| AcmeError::Acme(e.to_string()))
            }
        };

        if path.exists() {
            let raw = fs::read_to_string(&path)?;
            if let Ok(persisted) = serde_json::from_str::<PersistedAccount>(&raw) {
                let plain = self
                    .priv_client
                    .secret_open(rid, None, &ctx, &persisted.sealed)
                    .await
                    .map_err(|e| AcmeError::Priv(e.to_string()))?;
                let creds: AccountCredentials =
                    serde_json::from_str(plain.expose()).map_err(|e| {
                        AcmeError::Acme(format!("invalid ACME account credentials: {e}"))
                    })?;
                return builder()?
                    .from_credentials(creds)
                    .await
                    .map_err(|e| AcmeError::Acme(e.to_string()));
            }
        }

        let contacts: Vec<String> = self
            .contact_email
            .as_deref()
            .filter(|e| !e.trim().is_empty())
            .map(|e| format!("mailto:{}", e.trim()))
            .into_iter()
            .collect();
        let contact_refs: Vec<&str> = contacts.iter().map(String::as_str).collect();

        let (account, creds) = builder()?
            .create(
                &NewAccount {
                    contact: &contact_refs,
                    terms_of_service_agreed: true,
                    only_return_existing: false,
                },
                self.directory.url().to_owned(),
                None,
            )
            .await
            .map_err(|e| AcmeError::Acme(e.to_string()))?;

        let creds_json = Zeroizing::new(
            serde_json::to_string(&creds).map_err(|e| AcmeError::Acme(e.to_string()))?,
        );
        let sealed = self
            .priv_client
            .secret_seal(rid, None, &ctx, Secret::new(creds_json.as_str()))
            .await
            .map_err(|e| AcmeError::Priv(e.to_string()))?;
        write_private_json(&path, &PersistedAccount { sealed })?;
        Ok(account)
    }

    /// Issue (or renew) an ACME HTTP-01 certificate for `domain`, encrypt its private key
    /// at rest through `priv`, save it atomically, and load it into `DynamicCertResolver`.
    pub async fn issue_cert(&self, rid: &str, domain: &str) -> Result<CertInfo, AcmeError> {
        let domain = domain.to_ascii_lowercase();
        validate_domain(&domain).map_err(|e| AcmeError::Invalid(e.0))?;

        let _guard = self.issue_lock.lock().await;
        let res = self.issue_cert_inner(rid, &domain).await;
        let now = now_secs();

        match res {
            Ok(persisted) => {
                let mut map = self
                    .domains
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                map.insert(
                    domain.clone(),
                    DomainRuntime {
                        consecutive_failures: 0,
                        last_error: None,
                        next_retry_at: 0,
                        persisted: Some(persisted),
                    },
                );
                drop(map);
                Ok(self.cert_info(&domain))
            }
            Err(e) => {
                let msg = e.to_string();
                let mut map = self
                    .domains
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let rt = map.entry(domain.clone()).or_default();
                rt.consecutive_failures = rt.consecutive_failures.saturating_add(1);
                rt.last_error = Some(msg.clone());
                let backoff_secs = match rt.consecutive_failures {
                    1 => 5 * 60,
                    2 => 15 * 60,
                    3 => 3600,
                    _ => 6 * 3600,
                };
                rt.next_retry_at = now + backoff_secs;
                if rt.consecutive_failures >= 3 {
                    error!(
                        domain = %domain,
                        failures = rt.consecutive_failures,
                        error = %msg,
                        "ACME certificate issuance/renewal failed 3 or more times"
                    );
                } else {
                    warn!(domain = %domain, error = %msg, "ACME certificate issuance failed");
                }
                Err(e)
            }
        }
    }

    async fn issue_cert_inner(&self, rid: &str, domain: &str) -> Result<PersistedCert, AcmeError> {
        let account = self.load_or_create_account(rid).await?;
        let identifiers = [Identifier::Dns(domain.to_owned())];
        let mut order = account
            .new_order(&NewOrder::new(&identifiers))
            .await
            .map_err(|e| AcmeError::Acme(format!("order creation failed: {e}")))?;

        let mut tokens_guard = ChallengeGuard {
            map: self.challenges.clone(),
            tokens: Vec::new(),
        };

        {
            let mut authzs = order.authorizations();
            while let Some(res) = authzs.next().await {
                let mut authz = res.map_err(|e| AcmeError::Acme(e.to_string()))?;
                if authz.status == AuthorizationStatus::Valid {
                    continue;
                }
                if authz.status != AuthorizationStatus::Pending {
                    return Err(AcmeError::Acme(format!(
                        "authorization is in unexpected state {:?}",
                        authz.status
                    )));
                }
                let mut challenge = authz.challenge(ChallengeType::Http01).ok_or_else(|| {
                    AcmeError::Acme("CA did not offer an HTTP-01 challenge".into())
                })?;
                let token = challenge.token.clone();
                let key_auth = challenge.key_authorization().as_str().to_owned();
                self.challenges.insert(token.clone(), key_auth);
                tokens_guard.tokens.push(token);
                challenge.set_ready().await.map_err(|e| {
                    AcmeError::Acme(format!("failed to submit HTTP-01 challenge: {e}"))
                })?;
            }
        }

        let retry = RetryPolicy::default()
            .initial_delay(Duration::from_millis(250))
            .timeout(Duration::from_secs(90));
        let status = order
            .poll_ready(&retry)
            .await
            .map_err(|e| AcmeError::Acme(format!("challenge validation failed: {e}")))?;
        drop(tokens_guard);

        if status != OrderStatus::Ready {
            return Err(AcmeError::Acme(format!(
                "order did not become ready (status: {status:?})"
            )));
        }

        let key_pem = Zeroizing::new(
            order
                .finalize()
                .await
                .map_err(|e| AcmeError::Acme(format!("order finalize failed: {e}")))?,
        );
        let cert_pem = order
            .poll_certificate(&retry)
            .await
            .map_err(|e| AcmeError::Acme(format!("certificate download failed: {e}")))?;

        let meta = parse_cert_metadata(&cert_pem)?;
        let ctx = format!("proxy:cert:{domain}");
        let key_sealed = self
            .priv_client
            .secret_seal(rid, None, &ctx, Secret::new(key_pem.as_str()))
            .await
            .map_err(|e| AcmeError::Priv(e.to_string()))?;

        let persisted = PersistedCert {
            domain: domain.to_owned(),
            cert_pem: cert_pem.clone(),
            key_sealed,
            not_before: meta.not_before,
            not_after: meta.not_after,
            issuer: meta.issuer,
            staging: self.directory.is_staging(),
            issued_at: now_secs(),
        };
        let path = self.cert_path(domain)?;
        write_private_json(&path, &persisted)?;
        self.resolver
            .insert_pem(domain, &persisted.cert_pem, key_pem.as_str())?;
        info!(domain = %domain, not_after = persisted.not_after, "ACME certificate active");
        Ok(persisted)
    }

    /// Renew every managed certificate that is within 30 days of expiry (or missing from
    /// memory after a retry), respecting backoff.
    pub async fn renew_due_certs(&self) {
        let now = now_secs();
        let due: Vec<String> = {
            let guard = self
                .domains
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard
                .iter()
                .filter_map(|(domain, rt)| {
                    let p = rt.persisted.as_ref()?;
                    if needs_renewal(p.not_after, now) && now >= rt.next_retry_at {
                        Some(domain.clone())
                    } else {
                        None
                    }
                })
                .collect()
        };
        for domain in due {
            let rid = format!("renew-{domain}-{now}");
            if let Err(e) = self.issue_cert(&rid, &domain).await {
                warn!(domain = %domain, error = %e, "scheduled renewal failed");
            }
        }
    }
}

fn write_private_json<T: Serialize>(path: &Path, value: &T) -> Result<(), AcmeError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
    }
    let tmp = path.with_extension("json.tmp");
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|e| AcmeError::Io(std::io::Error::other(e)))?;
    fs::write(&tmp, bytes)?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renewal_window_is_30_days_before_expiry() {
        let now = 1_700_000_000;
        assert!(!needs_renewal(now + 31 * 86_400, now));
        assert!(needs_renewal(now + 30 * 86_400, now));
        assert!(needs_renewal(now + 10 * 86_400, now));
        assert!(needs_renewal(now - 1, now));
    }

    #[test]
    fn parses_x509_metadata_from_rcgen_cert() {
        let mut params =
            rcgen::CertificateParams::new(vec!["panel.example.com".to_owned()]).unwrap();
        let mut dn = rcgen::DistinguishedName::new();
        dn.push(rcgen::DnType::CommonName, "Test CA");
        params.distinguished_name = dn;
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = params.self_signed(&key).unwrap();
        let meta = parse_cert_metadata(&cert.pem()).unwrap();
        assert_eq!(meta.issuer, "Test CA");
        assert!(meta.not_after > meta.not_before);
    }
}
