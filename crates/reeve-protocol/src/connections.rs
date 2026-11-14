//! MCP connections on the wire (kb/ADR-0029): what an app may do, and the approval page's request and answers.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// What a connected app may do (kb/ADR-0029 R7). `write` includes `read`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Read,
    Write,
}

/// A pending request to connect an app, as the approval page shows it (kb/ADR-0029 R4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ConnectionRequestDto {
    /// The host of the app's client id: what the person is shown as the app's identity.
    pub client_host: String,
    /// The name the app gives itself: a secondary label, shown as text.
    pub client_name: String,
    /// The host the person is sent back to.
    pub redirect_host: String,
    /// The signed-in person who would approve.
    pub email: String,
    pub requested_at: DateTime<Utc>,
    /// The one-time value an answer must carry. A later read of the same request replaces it.
    pub approval: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ApproveConnection {
    pub approval: String,
    pub scope: Scope,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct RefuseConnection {
    pub approval: String,
}

/// Where the page sends the browser once the request is answered: the app's redirect address with the answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ConnectionAnswerDto {
    pub redirect_to: String,
}

/// Why a connection was revoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RevokedReason {
    /// By its owner or an admin.
    Revoked,
    /// A refresh token that had been used was presented again (kb/ADR-0029 R5).
    RefreshTokenReused,
    UserDisabled,
    /// The instance was restored from a backup, which ends every connection.
    Restored,
}

/// An app a person connected (kb/ADR-0029 R9): listed once, whatever tokens it has had. Never a secret or a hash.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ConnectionDto {
    pub id: i64,
    pub owner: String,
    pub client_id: String,
    /// The host of the client id: the app's identity as people are shown it. Empty when the client id is not an
    /// address (the developer command's).
    pub client_host: String,
    /// The name the app gave itself when it was approved.
    pub client_name: String,
    pub scope: Scope,
    pub approved_at: DateTime<Utc>,
    /// Thirty days after the approval. A connection at or past this has ended.
    pub ends_at: DateTime<Utc>,
    /// When the app last refreshed its token: the nearest thing to "last used" that is recorded.
    pub refreshed_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoked_reason: Option<RevokedReason>,
}
