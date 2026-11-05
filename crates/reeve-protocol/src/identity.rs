use crate::{Role, TokenKind};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct TokenDto {
    pub id: i64,
    pub kind: TokenKind,
    pub label: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct IssueTokenRequest {
    pub kind: TokenKind,
    #[serde(default)]
    pub label: String,
    pub expires_in_days: Option<u32>,
}

/// The secret is shown once; the server stores only its hash.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct IssuedTokenDto {
    pub token: TokenDto,
    pub secret: String,
}

/// What the client needs from instance configuration (kb/ADR-0009 R2): never hardcoded in the client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct InstanceConfigDto {
    pub key_prefix: String,
    pub horizon_buckets: Vec<String>,
    pub repos: Vec<String>,
    pub sign_in_configured: bool,
    /// `[import] enabled`: the import endpoints answer 404 unless this is true.
    #[serde(default)]
    pub import_enabled: bool,
    /// `[board] done_window_days`: how far back `done=recent` reaches. Absent from servers before this field existed (deserialises as 0, meaning unknown).
    #[serde(default)]
    pub done_window_days: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct UserDto {
    pub email: String,
    pub display_name: String,
    pub role: Role,
    pub disabled: bool,
    pub created_at: DateTime<Utc>,
}
