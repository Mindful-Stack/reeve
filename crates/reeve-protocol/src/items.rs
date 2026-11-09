use crate::{enums::*, ids::Key};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DiscoveryMarkDto {
    pub at: DateTime<Utc>,
    pub by: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct InitiativeDto {
    pub key: Key,
    pub title: String,
    pub brief: String,
    pub horizon: String,
    pub target_month: Option<String>,
    pub owner: Option<String>,
    pub discovery_complete: Option<DiscoveryMarkDto>,
    /// completed / not-dropped; None when nothing counts yet
    pub progress: Option<f32>,
    pub external_ref: Option<String>,
    pub version: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct EpicDto {
    pub key: Key,
    pub title: String,
    pub initiative: Option<Key>,
    pub brief: String,
    pub discovery_complete: Option<DiscoveryMarkDto>,
    pub external_ref: Option<String>,
    pub version: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct CriterionDto {
    pub text: String,
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct StoryDto {
    pub key: Key,
    pub title: String,
    pub kind: Kind,
    pub epic: Option<Key>,
    pub description: String,
    pub criteria: Vec<CriterionDto>,
    pub owner: Option<String>,
    pub labels: Vec<String>,
    pub rank: i32,
    pub blocked_by: Vec<Key>,
    pub delivery: DeliveryState,
    pub resolution: Option<Resolution>,
    pub external_ref: Option<String>,
    pub version: i64,
    /// Pull requests linked to this story (possibly none); informational, they never move it.
    pub prs: Vec<PullRequestDto>,
    /// The active sprint's goals this story serves, through itself, its epic or its epic's initiative (kb/ADR-0014
    /// R3), computed on read; empty with no active sprint. The server always sends it; JSON without it (an older
    /// server) reads as no goals.
    #[serde(default)]
    #[schema(required = true)]
    pub goals: Vec<crate::sprints::GoalRefDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PullRequestDto {
    pub repo: String,
    pub number: i64,
    pub url: String,
    pub title: String,
    pub status: PrStatus,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct RecordRefDto {
    pub repo: String,
    pub path: String,
    pub record_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct RequiredDecisionDto {
    pub id: i64,
    pub on: Key,
    /// The level of the item `on` names; one key sequence (kb/ADR-0002 R5) means the key alone does not say.
    pub on_type: ItemType,
    pub title: String,
    pub decider: String,
    pub due: Option<chrono::NaiveDate>,
    /// One-way or two-way; unset behaves as two-way (kb/ADR-0013 R1).
    #[serde(default)]
    pub reversibility: Option<Reversibility>,
    pub record: Option<RecordRefDto>,
    pub pr_url: Option<String>,
    pub check: CheckResult,
    pub checked_at: Option<DateTime<Utc>>,
    pub closed: Option<ClosedDto>,
    pub created_at: DateTime<Utc>,
    pub version: i64,
}

/// A todo story that cannot start because of a decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct BlockedStoryDto {
    pub key: Key,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DecisionQueueEntryDto {
    pub decision: RequiredDecisionDto,
    pub group: QueueGroup,
    /// The todo stories whose readiness fails on this decision; empty outside the blocking group.
    pub blocks: Vec<BlockedStoryDto>,
}

/// Counts over open, unsatisfied decisions only; `blocked_stories` counts each story once.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DecisionQueueSummaryDto {
    pub open: u32,
    pub blocking: u32,
    pub blocked_stories: u32,
    pub overdue: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DecisionQueueDto {
    /// In page order: groups in `QueueGroup` declaration order, then server order within each group. Clients must not
    /// re-sort.
    pub entries: Vec<DecisionQueueEntryDto>,
    pub summary: DecisionQueueSummaryDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ClosedDto {
    pub at: DateTime<Utc>,
    pub by: String,
    pub reason: String,
    /// Decided or withdrawn (kb/ADR-0013 R3); absent on closes made before it.
    #[serde(default)]
    pub kind: Option<ClosureKind>,
    /// The kind of token that closed it; absent on closes made before kb/ADR-0013.
    #[serde(default)]
    pub via: Option<TokenKind>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct CommentDto {
    pub id: i64,
    pub on_story: Option<Key>,
    pub on_decision: Option<i64>,
    /// Set when the comment is on an initiative; absent from servers before it.
    #[serde(default)]
    pub on_initiative: Option<Key>,
    /// Set when the comment is on an epic; absent from servers before it.
    #[serde(default)]
    pub on_epic: Option<Key>,
    pub author: String,
    pub body: String,
    pub at: DateTime<Utc>,
    pub external_ref: Option<String>,
}

/// One item `GET /search` found: its level (and a story's kind) and which of its fields matched: `key` (the
/// query named its key), `title`, `brief` (an initiative's or an epic's), `description` (a story's) or `external_ref`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SearchHit {
    pub key: Key,
    pub item: ItemType,
    pub kind: Option<Kind>,
    pub title: String,
    pub matched: Vec<String>,
    /// The query is this item's key or, ignoring case, its whole external reference: the item it names, which a
    /// client shows above the rest.
    pub exact: bool,
}
