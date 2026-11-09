use crate::{ItemType, Key};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// An item named with its type: keys share one sequence (kb/ADR-0002 R5), so a key alone does not say what it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ItemRefDto {
    pub key: Key,
    pub item: ItemType,
}

/// The urgent mark stored on an item (kb/ADR-0017). `expires_at` and `active` are the server's answer; clients never
/// compute expiry. `done`: the marked item is a done story, so the mark makes nothing urgent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct UrgentMarkDto {
    pub at: DateTime<Utc>,
    pub by: String,
    pub reason: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub active: bool,
    pub done: bool,
}

/// The initiative a war room row belongs to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct WarRoomInitiativeDto {
    pub key: Key,
    pub title: String,
    pub horizon: String,
}

/// One row of the war room: a stored mark, active or expired.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct WarRoomEntryDto {
    pub item: ItemRefDto,
    pub title: String,
    pub initiative: Option<WarRoomInitiativeDto>,
    pub mark: UrgentMarkDto,
}

/// Mark (or renew) an item urgent; the reason is optional, at most 200 characters.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MarkUrgent {
    #[serde(default)]
    pub reason: Option<String>,
}

/// The war-room rows the caller saw; bulk clear and renew act on exactly these.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WarRoomItems {
    pub items: Vec<ItemRefDto>,
}
