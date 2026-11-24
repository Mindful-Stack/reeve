use crate::{
    ActivityEntryDto, AttachmentDto, CommentDto, EpicDto, InitiativeDto, ItemLinkType, Key,
    RequiredDecisionDto, SprintDto, StoryDto, UserDto,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The whole instance as one consistent snapshot (`GET /api/v1/export`): every list is read inside one
/// `REPEATABLE READ` transaction. Items order by key number, decisions, comments and activity by id, users by email, item links by their `from` key
/// and web links by their item's key.
/// Users carry emails and roles only; the export never holds token hashes, secrets or configuration.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ExportDto {
    pub exported_at: DateTime<Utc>,
    pub key_prefix: String,
    pub initiatives: Vec<InitiativeDto>,
    pub epics: Vec<EpicDto>,
    pub stories: Vec<StoryDto>,
    /// Every sprint, oldest first, with its goals (confidence, verdict, created_at) and their links nested. Not in the
    /// DTOs: `created_at` of sprints and who linked what and when. The exported activity carries the goal's
    /// `created` entry and each item's `goal_linked` entry (actor, time); a draft sprint's creation is not logged.
    pub sprints: Vec<SprintDto>,
    pub decisions: Vec<RequiredDecisionDto>,
    pub comments: Vec<CommentDto>,
    /// Every attachment an item owns, by upload time then id: metadata only; the bytes travel in the database dump
    /// (kb/ADR-0022 R5). Uploads not yet saved with any text are left out.
    #[serde(default)]
    pub attachments: Vec<AttachmentDto>,
    /// Every item link (kb/ADR-0031), once, as stored, ordered by the number of its `from` key, then of its `to`
    /// key, then by name (follows, duplicates, relates_to). An export written before item links existed has no
    /// such field.
    #[serde(default)]
    pub item_links: Vec<ExportItemLinkDto>,
    /// Every web link, ordered by the number of its item's key and, on one item, by when it was added.
    #[serde(default)]
    pub web_links: Vec<ExportWebLinkDto>,
    pub activity: Vec<ActivityEntryDto>,
    pub users: Vec<UserDto>,
}

/// An item link in the export: `from` follows, duplicates or relates to `to`. A `relates_to` has the lower key
/// (byte order) in `from`, whichever end it was added from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ExportItemLinkDto {
    pub from: Key,
    #[serde(rename = "type")]
    pub link_type: ItemLinkType,
    pub to: Key,
}

/// A web link in the export: a title and a URL on one item. Who added it and when is in the exported activity
/// (`web_link_added`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ExportWebLinkDto {
    pub item: Key,
    pub title: String,
    pub url: String,
}
