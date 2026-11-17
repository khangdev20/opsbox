//! Password hashing (Argon2id) and the rules for choosing a password.

use argon2::password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash};
use argon2::{Algorithm, Argon2, Params, Version};

/// Argon2id cost. The defaults are the spec's (64 MB, 3 passes, 1 lane); tune them
/// so a sign-in stays under about 300 ms on the smallest supported machine.
#[derive(Debug, Clone, Copy)]
pub struct HashParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl Default for HashParams {
    fn default() -> Self {
        Self {
            memory_kib: 64 * 1024,
            iterations: 3,
            parallelism: 1,
        }
    }
}

pub struct Hasher {
    argon: Argon2<'static>,
}

impl Hasher {
    pub fn new(p: HashParams) -> Self {
        let params = Params::new(p.memory_kib, p.iterations, p.parallelism, None)
            .expect("argon2 parameters are valid");
        Self {
            argon: Argon2::new(Algorithm::Argon2id, Version::V0x13, params),
        }
    }

    /// A PHC string (`$argon2id$v=19$m=...`) with a fresh random salt.
    pub fn hash(&self, password: &str) -> Result<String, String> {
        self.argon
            .hash_password(password.as_bytes())
            .map(|h| h.to_string())
            .map_err(|e| e.to_string())
    }

    /// Check a password against a stored hash. The cost parameters come from the
    /// stored hash, so changing [`HashParams`] later does not lock anyone out.
    pub fn verify(&self, password: &str, stored: &str) -> bool {
        PasswordHash::new(stored).is_ok_and(|parsed| {
            self.argon
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
    }
}

pub const MIN_PASSWORD_CHARS: usize = 12;
const MAX_PASSWORD_BYTES: usize = 256;

/// Rules for a new password. Returns a sentence for the person, or `Ok`.
pub fn check_new_password(password: &str, username: &str) -> Result<(), String> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(format!(
            "Use at least {MIN_PASSWORD_CHARS} characters for the password."
        ));
    }
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(format!(
            "The password is too long (at most {MAX_PASSWORD_BYTES} bytes)."
        ));
    }
    if password.eq_ignore_ascii_case(username)
        || (username.len() >= 4 && password.to_lowercase().contains(&username.to_lowercase()))
    {
        return Err("The password must not contain the username.".into());
    }
    let first = password.chars().next();
    if password.chars().all(|c| Some(c) == first) {
        return Err("The password must not be one repeated character.".into());
    }
    Ok(())
}

/// Account names: lower-case letters, digits, `.`, `_`, `-`; start with a letter.
pub fn check_username(name: &str) -> Result<(), String> {
    let ok = (3..=32).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        });
    if ok {
        Ok(())
    } else {
        Err("Username: 3 to 32 characters, lower-case letters, digits, dots, dashes or underscores, starting with a letter.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast() -> Hasher {
        Hasher::new(HashParams {
            memory_kib: 8,
            iterations: 1,
            parallelism: 1,
        })
    }

    #[test]
    fn hashes_verify_and_are_salted() {
        let h = fast();
        let a = h.hash("correct horse battery").unwrap();
        let b = h.hash("correct horse battery").unwrap();
        assert!(a.starts_with("$argon2id$v=19$"), "{a}");
        assert_ne!(a, b, "each hash has its own salt");
        assert!(h.verify("correct horse battery", &a));
        assert!(!h.verify("correct horse batterx", &a));
        assert!(!h.verify("", &a));
    }

    #[test]
    fn garbage_hashes_never_verify() {
        let h = fast();
        for junk in ["", "plain", "$argon2id$", "$argon2id$v=19$m=1,t=1,p=1$x$y"] {
            assert!(!h.verify("anything", junk), "{junk}");
        }
    }

    #[test]
    fn old_hashes_keep_working_after_the_cost_changes() {
        let old = fast().hash("a long enough password").unwrap();
        let newer = Hasher::new(HashParams {
            memory_kib: 16,
            iterations: 2,
            parallelism: 1,
        });
        assert!(newer.verify("a long enough password", &old));
    }

    #[test]
    fn default_cost_is_the_specs() {
        let d = HashParams::default();
        assert_eq!((d.memory_kib, d.iterations, d.parallelism), (65536, 3, 1));
    }

    #[test]
    fn password_rules() {
        assert!(check_new_password("short", "alice").is_err());
        assert!(check_new_password("aaaaaaaaaaaaaaaa", "alice").is_err());
        assert!(check_new_password("my-alice-password-1", "alice").is_err());
        assert!(check_new_password("correct horse battery staple", "alice").is_ok());
        assert!(
            check_new_password("пароль-на-русском-языке", "alice").is_ok(),
            "length counts characters, not bytes"
        );
        assert!(
            check_new_password(&"x1".repeat(200), "alice").is_err(),
            "too long"
        );
    }

    #[test]
    fn username_rules() {
        for ok in ["admin", "bob.smith", "a_b-c9"] {
            assert!(check_username(ok).is_ok(), "{ok}");
        }
        for bad in [
            "ab",
            "Admin",
            "9lives",
            "has space",
            "dröp",
            &"a".repeat(33),
            "",
        ] {
            assert!(check_username(bad).is_err(), "{bad}");
        }
    }
}
