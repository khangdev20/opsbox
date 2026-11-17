//! Panel accounts: passwords, two-factor sign-in, sessions, lockout, first-run setup.
//!
//! The pieces that need no I/O are plain modules ([`password`], [`tokens`],
//! [`lockout`], [`setup_token`]). [`Auth`] ties them to the store and to `priv`
//! (which holds the TOTP secrets) and is what the HTTP layer and the CLI call.

pub mod lockout;
pub mod password;
mod service;
pub mod setup_token;
pub mod tokens;

pub use service::{
    Auth, AuthConfig, AuthError, Enrolment, InviteInfo, Issued, LoginOutcome, LoginRequest,
    Principal, SessionKindView, TOKEN_PREFIX,
};
