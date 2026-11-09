pub use reeve_protocol::{Role, TokenKind};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct IdentityConfig {
    pub allowed_domains: Vec<String>,
    pub allowed_emails: Vec<String>,
    pub admins: Vec<String>,
}

impl IdentityConfig {
    pub fn is_allowed(&self, email: &str) -> bool {
        let email = normalise_email(email);
        let Some((local, domain)) = email.split_once('@') else {
            return false;
        };
        if local.is_empty() || domain.is_empty() || domain.contains('@') {
            return false;
        }
        let matches = |c: &String, target: &str| {
            let c = normalise_email(c);
            !c.is_empty() && c == target
        };
        self.allowed_emails.iter().any(|c| matches(c, &email))
            || self.allowed_domains.iter().any(|c| matches(c, domain))
    }
    pub fn role_for(&self, email: &str) -> Role {
        let email = normalise_email(email);
        if self.admins.iter().any(|a| normalise_email(a) == email) {
            Role::Admin
        } else {
            Role::Member
        }
    }
}

/// Who is calling: resolved from a token on every request (kb/ADR-0015 R5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub email: String,
    pub role: Role,
    pub token_kind: TokenKind,
}

pub use reeve_protocol::normalise_email;

/// The one parser for the stored token kind; `None` for text no `TokenKind` writes.
pub(crate) fn kind_from(s: &str) -> Option<TokenKind> {
    match s {
        "interactive" => Some(TokenKind::Interactive),
        "automation" => Some(TokenKind::Automation),
        _ => None,
    }
}

pub fn hash_secret(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}
pub fn new_secret() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("reeve_{}", hex::encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allowlist_matches_domain_or_email_case_insensitively() {
        let c = IdentityConfig {
            allowed_domains: vec!["Example.com".into()],
            allowed_emails: vec!["Guest@Other.org".into()],
            admins: vec![],
        };
        assert!(c.is_allowed("dan@example.com"));
        assert!(c.is_allowed("guest@other.org"));
        assert!(!c.is_allowed("x@other.org"));
        assert!(!c.is_allowed("example.com"));
    }
}
