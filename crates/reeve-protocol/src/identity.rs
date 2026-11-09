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

/// What the client needs from instance configuration (kb/ADR-0016 R2): never hardcoded in the client.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccessKind {
    Domain,
    Email,
}

/// Where an entry comes from: the instance configuration (locked) or the database (editable by admins).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccessSourceDto {
    Config,
    Database,
}

/// One allowed domain, allowed email or admin, from one source (kb/ADR-0015 R2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct AccessEntryDto {
    pub value: String,
    pub source: AccessSourceDto,
    /// A database row that duplicates a configuration entry: listed separately and removable.
    pub also_in_config: bool,
    /// False only on an admin grant whose email nothing allows: it grants nothing until the email is allowed again.
    pub allowed: bool,
    pub added_by: Option<String>,
    pub added_at: Option<DateTime<Utc>>,
}

/// Who may sign in and who is admin: configuration rows first, then database rows, each by value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct AccessDto {
    pub domains: Vec<AccessEntryDto>,
    pub emails: Vec<AccessEntryDto>,
    pub admins: Vec<AccessEntryDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct AllowEntryRequest {
    pub kind: AccessKind,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct GrantAdminRequest {
    pub email: String,
}
