//! Connections (kb/ADR-0029 R5, R9): an approved, exchanged request to connect an MCP client: a person, a client,
//! a scope, and an end thirty days after the approval. Every MCP token belongs to one.
//!
//! The lock rule of every transaction, here and in `handler.rs`, that touches a person's connections, MCP tokens
//! or refresh tokens: the person's `users` row first (`TokenStore::lock_user`, or the `ON CONFLICT DO UPDATE` of
//! `set_disabled` and `ensure_user`), the activity entries last. Whoever holds the row is alone with that
//! person's connections, so nothing in between needs an order (PostgreSQL 16, 13.3.4: "acquire locks on multiple
//! objects in a consistent order").
use chrono::{DateTime, Utc};

/// How long a connection lasts from its approval (kb/ADR-0029 R5).
pub(crate) const CONNECTION_DAYS: i64 = 30;
/// The client of a connection made by `reeve-server mcp-token-issue` (a developer command of test builds), and
/// the name listings show for it.
pub(crate) const DEVELOPER_CLIENT_ID: &str = "urn:reeve:developer-command";
pub(crate) const DEVELOPER_CLIENT_NAME: &str = "Developer command";

/// The one place "live" is decided: not revoked, and not yet at its end.
pub(crate) fn is_live(
    ends_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> bool {
    revoked_at.is_none() && ends_at > now
}
