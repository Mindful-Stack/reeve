use reeve_protocol::Scope;
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
    /// The id of the token this request authenticated with. A web session is one token; an approval value is
    /// bound to the token that read the request (kb/ADR-0029 R4).
    pub token_id: i64,
}

/// Who is calling the MCP endpoint (kb/ADR-0029 R6). A type of its own, without a token kind: code that takes a
/// `Principal` cannot be handed an MCP caller, and the MCP endpoint cannot be handed an API one. It carries the
/// connection the token belongs to, the client that connection was approved for, and what it may do (R5, R7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpPrincipal {
    pub email: String,
    pub role: Role,
    pub connection_id: i64,
    pub client_id: String,
    pub scope: Scope,
}

/// The text stored in `connections.scope`.
pub(crate) fn scope_text(scope: Scope) -> &'static str {
    match scope {
        Scope::Read => "read",
        Scope::Write => "write",
    }
}

/// The one parser for the stored scope; `None` for text no `Scope` writes.
pub(crate) fn scope_from(s: &str) -> Option<Scope> {
    match s {
        "read" => Some(Scope::Read),
        "write" => Some(Scope::Write),
        _ => None,
    }
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

/// Where a token works (kb/ADR-0029 R5, R6): the API or the MCP endpoint. Not a third token kind (kb/ADR-0015 R7):
/// an MCP token is an interactive token whose audience is the MCP endpoint. A caller outside this crate chooses an
/// audience only by calling `issue_token` or `issue_mcp_token`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    /// The API, the event stream, attachments and the session cookie.
    Api,
    /// The MCP endpoint only.
    Mcp,
}

impl Audience {
    /// The text stored in `tokens.audience`.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Audience::Api => "api",
            Audience::Mcp => "mcp",
        }
    }
}

/// The one parser for the stored audience; `None` for text no `Audience` writes.
pub(crate) fn audience_from(s: &str) -> Option<Audience> {
    match s {
        "api" => Some(Audience::Api),
        "mcp" => Some(Audience::Mcp),
        _ => None,
    }
}

pub fn hash_secret(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}
pub fn new_secret() -> String {
    random_secret("reeve_")
}

/// `prefix` and 24 random bytes in hex. A code and a refresh token carry prefixes of their own, so one is never
/// mistaken for a token in a log or a paste.
pub(crate) fn random_secret(prefix: &str) -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("{prefix}{}", hex::encode(bytes))
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

    #[test]
    fn audience_text_round_trips_and_unknown_text_is_refused() {
        for a in [Audience::Api, Audience::Mcp] {
            assert_eq!(audience_from(a.as_str()), Some(a));
        }
        for text in ["web", "", "API", " mcp"] {
            assert_eq!(audience_from(text), None, "{text:?}");
        }
    }
}
