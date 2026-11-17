//! GitHub integration, build detection, deploy key generation, and webhook verification (Phase 3).

pub mod compose;
pub mod deploy_key;
pub mod detect;
pub mod github;
pub mod jwt;
pub mod manifest;
pub mod pipeline;
pub mod public_repo;
pub mod templates;
pub mod webhook;

pub use deploy_key::{
    DeployKeyPair, NormalizedDeployRepo, generate_ed25519_deploy_key, normalize_repo_source,
};
pub use detect::detect_build_method;
pub use github::{
    AppInstallation, DEFAULT_API_BASE, DEFAULT_WEB_BASE, GitHubClient, GitHubConfig,
    GitHubRepoMeta, ManifestConversion, PublicGitHubRepo, format_pushed_ago,
};
pub use jwt::sign_app_jwt;
pub use manifest::{ManifestStart, build_manifest, random_hex, validate_org};
pub use pipeline::{
    CheckoutBuildPlan, looks_secret_var_key, plan_checkout_build, scrub_secrets,
    static_site_dockerfile,
};
pub use public_repo::{PublicRepoTarget, validate_public_repo_url};
pub use webhook::{WebhookEvent, parse_webhook_event, verify_github_signature};

#[derive(Debug, thiserror::Error)]
pub enum DeployError {
    #[error("{message}")]
    Validation {
        field: Option<&'static str>,
        message: String,
    },
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    InvalidKey(String),
    #[error("{0}")]
    GitHub(String),
    #[error("{0}")]
    Internal(String),
}
