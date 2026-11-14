//! MCP connections on the wire (kb/ADR-0029): what an app may do, the approval page's request and answers, and
//! the connections list.
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// What a connected app may do (kb/ADR-0029 R7). `write` includes `read`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Read,
    Write,
}
