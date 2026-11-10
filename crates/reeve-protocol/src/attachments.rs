//! Attachments (kb/ADR-0022, kb/ADR-0023): images an initiative, epic or story owns, or an upload not yet saved with any
//! text.
use crate::{ItemType, Key};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// The item an attachment belongs to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct AttachmentOwnerDto {
    pub item: ItemType,
    pub key: Key,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct AttachmentDto {
    pub id: Uuid,
    /// Where the image is served; the one source of its path.
    pub url: String,
    pub filename: String,
    /// The type its bytes identify: `image/png`, `image/jpeg` or `image/webp`.
    pub content_type: String,
    pub byte_size: i64,
    pub uploaded_by: String,
    pub uploaded_at: DateTime<Utc>,
    /// None while it is an upload not yet saved with any text (only its uploader sees it then).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<AttachmentOwnerDto>,
}
