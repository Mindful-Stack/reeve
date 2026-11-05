use crate::{
    ActivityEntryDto, CommentDto, EpicDto, InitiativeDto, RequiredDecisionDto, SprintDto, StoryDto,
    UserDto,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The whole instance as one consistent snapshot (`GET /api/v1/export`): every list is read inside one
/// `REPEATABLE READ` transaction. Items order by key number, decisions, comments and activity by id, users by email.
/// Users carry emails and roles only; the export never holds token hashes, secrets or configuration.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ExportDto {
    pub exported_at: DateTime<Utc>,
    pub key_prefix: String,
    pub initiatives: Vec<InitiativeDto>,
    pub epics: Vec<EpicDto>,
    pub stories: Vec<StoryDto>,
    /// Every sprint, oldest first, with its goals (confidence, verdict) and their links nested. Not in the DTOs:
    /// `created_at` of sprints and goals and who linked what and when. The exported activity carries the goal's
    /// `created` entry and each item's `goal_linked` entry (actor, time); a draft sprint's creation is not logged.
    pub sprints: Vec<SprintDto>,
    pub decisions: Vec<RequiredDecisionDto>,
    pub comments: Vec<CommentDto>,
    pub activity: Vec<ActivityEntryDto>,
    pub users: Vec<UserDto>,
}
