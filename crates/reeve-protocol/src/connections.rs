//! MCP connections on the wire (kb/ADR-0029): what an app may do, the approval page's request and answers, and
//! the connections list.
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
