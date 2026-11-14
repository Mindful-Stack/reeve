//! Users, tokens and the principal every request runs as.
mod access;
mod access_handler;
mod access_repo;
mod client_fetch;
mod clients;
mod domain;
#[cfg(feature = "static-oidc")]
mod fake_client_host;
#[cfg(feature = "static-oidc")]
mod fake_issuer;
mod handler;
mod oidc;
mod ports;
mod repo;
pub use access::{
    AccessGrant, AccessKind, AccessPolicy, AccessRow, AccessSource, AccessView, parse_domain,
    parse_email,
};
pub use access_handler::{allow_entry, grant_admin, remove_entry, revoke_admin};
pub use access_repo::{AccessStore, make_access_store};
pub use client_fetch::make_client_fetcher;
#[cfg(feature = "static-oidc")]
pub use client_fetch::make_loopback_client_fetcher;
pub use clients::{
    ClientDocumentFetcher, ClientPolicy, ClientRefusal, ValidatedClient, resolve_client,
};
pub use domain::{
    Audience, IdentityConfig, McpPrincipal, Principal, Role, TokenKind, hash_secret,
    normalise_email,
};
#[cfg(feature = "static-oidc")]
pub use fake_client_host::{FakeAnswer, FakeClientHost};
#[cfg(feature = "static-oidc")]
pub use fake_issuer::{FakeIssuer, Grant, IdClaims, Signer};
pub use handler::{
    AuthError, IssuedToken, SESSION_LABEL, VerifiedIdentity, authenticate_api, authenticate_mcp,
    disable_user, enable_user, issue_mcp_token, issue_token, list_tokens, list_users,
    list_users_in, revoke_all_for, revoke_token, sign_in,
};
pub use oidc::{GoogleOidcConfig, OidcClient, OidcError, make_google_oidc};
#[cfg(feature = "static-oidc")]
pub use oidc::{GoogleOidcTestOptions, TestClock, make_google_oidc_for_tests, make_static_oidc};
pub use ports::{IdentityActivity, IdentityEvent};
pub use reeve_protocol::Via;
pub use repo::{TokenRow, TokenStore, TokenSummary, UserRow, make_token_store};
pub fn migrator() -> sqlx::migrate::Migrator {
    sqlx::migrate!("./migrations")
}
