use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    Interactive,
    Automation,
    System,
}

/// One row of the activity feed, as served on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ActivityEntryDto {
    pub id: i64,
    pub at: DateTime<Utc>,
    pub actor: String,
    pub via: Via,
    pub entity_type: String,
    pub entity_key: String,
    pub action: String,
    pub payload: serde_json::Value,
}

/// The caller, as `GET /me` reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Me {
    pub email: String,
    pub role: crate::enums::Role,
    pub token_kind: crate::enums::TokenKind,
}

/// One row of `GET /stories`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct StoryWithReadiness {
    pub story: crate::items::StoryDto,
    pub readiness: crate::readiness::Readiness,
}
