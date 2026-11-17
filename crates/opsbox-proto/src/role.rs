//! Panel account roles (docs/BACKEND_SPEC.md section 6.2).

use serde::{Deserialize, Serialize};

/// What an account may do. `Admin` can do everything; `Deployer` manages apps and
/// reads logs but gets no terminal, no system changes and no files outside app
/// directories; `Viewer` is read-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Viewer,
    Deployer,
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Deployer => "deployer",
            Self::Admin => "admin",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "viewer" => Some(Self::Viewer),
            "deployer" => Some(Self::Deployer),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }

    /// Does this role satisfy a handler that needs at least `needed`?
    pub fn allows(self, needed: Self) -> bool {
        self >= needed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_is_viewer_deployer_admin() {
        assert!(Role::Admin.allows(Role::Admin) && Role::Admin.allows(Role::Viewer));
        assert!(Role::Deployer.allows(Role::Deployer) && Role::Deployer.allows(Role::Viewer));
        assert!(!Role::Deployer.allows(Role::Admin));
        assert!(Role::Viewer.allows(Role::Viewer));
        assert!(!Role::Viewer.allows(Role::Deployer) && !Role::Viewer.allows(Role::Admin));
    }

    #[test]
    fn names_round_trip() {
        for r in [Role::Viewer, Role::Deployer, Role::Admin] {
            assert_eq!(Role::parse(r.as_str()), Some(r));
        }
        assert_eq!(Role::parse("root"), None);
        assert_eq!(Role::parse("Admin"), None);
    }
}
