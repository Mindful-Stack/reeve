use crate::promise::{EpicPromiseDto, StoryPromiseDto};
use crate::urgency::{ItemRefDto, UrgentMarkDto};
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
    /// The urgent mark stored on this item, active or expired (kb/ADR-0017). Servers always send it.
    #[serde(default)]
    #[schema(required = true)]
    pub urgent_mark: Option<UrgentMarkDto>,
    /// The active marks that make this item urgent: its own first, then key-number order. Empty: not urgent.
    #[serde(default)]
    #[schema(required = true)]
    pub urgent_sources: Vec<ItemRefDto>,
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
    /// The urgent mark stored on this item, active or expired (kb/ADR-0017). Servers always send it.
    #[serde(default)]
    #[schema(required = true)]
    pub urgent_mark: Option<UrgentMarkDto>,
    /// The active marks that make this item urgent: its own first, then key-number order. Empty: not urgent.
    #[serde(default)]
    #[schema(required = true)]
    pub urgent_sources: Vec<ItemRefDto>,
    /// The epic's own promise, classified on read (kb/ADR-0025 R4–R5). Servers always send it; JSON without it (an
    /// older server) reads as none.
    #[serde(default)]
    #[schema(required = true)]
    pub promise: Option<EpicPromiseDto>,
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
    /// The urgent mark stored on this item, active or expired (kb/ADR-0017). Servers always send it.
    #[serde(default)]
    #[schema(required = true)]
    pub urgent_mark: Option<UrgentMarkDto>,
    /// The active marks that make this item urgent: its own first, then key-number order. Empty: not urgent.
    #[serde(default)]
    #[schema(required = true)]
    pub urgent_sources: Vec<ItemRefDto>,
    /// The story's promise, own and inherited, derived on read (kb/ADR-0025 R3–R5). Servers always send it; JSON
    /// without it (an older server) reads as none.
    #[serde(default)]
    #[schema(required = true)]
    pub promise: Option<StoryPromiseDto>,
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
    /// Where to read the record in a browser, `<repo url>/blob/HEAD/<path>`; the server derives it from the repository's
    /// `https://` URL in `[[repos]]` and leaves it out when there is none (never stored, kb/ADR-0004 R1). Absent
    /// in JSON from an older server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browse_url: Option<String>,
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

fn default_kind() -> Kind {
    Kind::Story
}

fn default_delivery() -> DeliveryState {
    DeliveryState::Todo
}

/// A story a decision affects: held by it (todo, in `blocks`) or started despite it (in `under_way`). The type name is
/// historical (it was `BlockedStoryDto` and held only `key` and `title`); it is JSON-compatible with that shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct AffectedStoryDto {
    pub key: Key,
    pub title: String,
    /// Servers always send it; JSON without it (an older server) reads as a story.
    #[serde(default = "default_kind")]
    #[schema(required = true)]
    pub kind: Kind,
    /// Servers always send it; JSON without it (an older server, whose blocks were all todo) reads as todo.
    #[serde(default = "default_delivery")]
    #[schema(required = true)]
    pub delivery: DeliveryState,
    /// Servers always send it; JSON without it reads as no owner.
    #[serde(default)]
    #[schema(required = true)]
    pub owner: Option<String>,
    /// Urgent on the queue's snapshot (kb/ADR-0024 R5). This story flag is shown only: the queue never places or
    /// counts by it. Servers always send it; JSON without it reads as false.
    #[serde(default)]
    #[schema(required = true)]
    pub urgent: bool,
    /// The story's own window: the earlier of its goals' and its promise's (kb/ADR-0025 R7). Servers always send it; JSON without it reads as `none`.
    #[serde(default)]
    #[schema(required = true)]
    pub needed_in: NeededIn,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DecisionQueueEntryDto {
    pub decision: RequiredDecisionDto,
    pub group: QueueGroup,
    /// The page lane. Servers always send it; JSON without it (an older server) reads as `open`.
    #[serde(default)]
    #[schema(required = true)]
    pub lane: QueueLane,
    /// When the decision is needed. Servers always send it; JSON without it reads as `none`.
    #[serde(default)]
    #[schema(required = true)]
    pub needed_in: NeededIn,
    /// Why it is needed this sprint or next, or, in Now (standalone, `beyond` under the first bucket), the Now rule that
    /// placed it; absent otherwise. A decision nested under a discovery row keeps its own (never a Now rule).
    #[serde(default)]
    #[schema(required = true)]
    pub needed_in_reason: Option<NeededInReason>,
    /// What the decision needs next; absent when closed or satisfied.
    #[serde(default)]
    #[schema(required = true)]
    pub next_step: Option<NextStep>,
    /// The todo stories whose readiness fails on this decision (held); empty outside the live lanes.
    pub blocks: Vec<AffectedStoryDto>,
    /// Started stories the decision affects (in progress, in review or testing). Servers always send it; JSON without
    /// it reads as none.
    #[serde(default)]
    #[schema(required = true)]
    pub under_way: Vec<AffectedStoryDto>,
    /// The discovery row this decision is nested under (its `on`, when that item has a row); it then sits in the row's
    /// lane, directly after the row's `at`. Servers always send it; JSON without it reads as none.
    #[serde(default)]
    #[schema(required = true)]
    pub under: Option<Key>,
}

/// Counts over open, unsatisfied decisions; `blocked_stories` counts each story once. Discovery rows are not counted
/// here: a client counts `discoveries` by lane.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DecisionQueueSummaryDto {
    pub open: u32,
    pub blocking: u32,
    pub blocked_stories: u32,
    pub overdue: u32,
    /// Decisions whose next step is `review_record`, in any lane. Servers always send it; JSON without it (an older
    /// server) reads as 0.
    #[serde(default)]
    #[schema(required = true)]
    pub review: u32,
    /// Decisions in the this-sprint lane, nested ones included; reads as 0 without it.
    #[serde(default)]
    #[schema(required = true)]
    pub this_sprint: u32,
    /// Decisions in the next-sprint lane, nested ones included; reads as 0 without it.
    #[serde(default)]
    #[schema(required = true)]
    pub next_sprint: u32,
}

/// A discovery row of the readiness queue: an initiative or epic without its discovery mark that holds unfinished
/// stories (kb/ADR-0002 R3, read off readiness, never stored: kb/ADR-0018 R1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DiscoveryEntryDto {
    pub key: Key,
    pub level: ItemType,
    pub title: String,
    /// For an epic row: its initiative, when that initiative is without its discovery mark too.
    #[schema(required = true)]
    pub also: Option<Key>,
    /// `this_sprint`, `next_sprint`, or `beyond` with a Now reason (Now, no goal yet).
    pub lane: QueueLane,
    /// The goal or promise that needs it, or the Now rule that placed it.
    #[schema(required = true)]
    pub needed_in_reason: Option<NeededInReason>,
    /// The todo stories whose discovery item this is.
    pub blocks: Vec<AffectedStoryDto>,
    /// The started ones.
    pub under_way: Vec<AffectedStoryDto>,
    /// The index in `entries` the row goes before (`entries.len()` at the end).
    pub at: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DecisionQueueDto {
    /// In page order: lanes in `QueueLane` declaration order (`beyond`: Now, then Later), server order within each
    /// lane; a nested decision (`under`) directly follows its row's `at`. Clients must not re-sort.
    pub entries: Vec<DecisionQueueEntryDto>,
    /// The discovery rows, in page order. Servers always send it; JSON without it (an older server) reads as none.
    #[serde(default)]
    #[schema(required = true)]
    pub discoveries: Vec<DiscoveryEntryDto>,
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

/// One story `GET /stories/similar` found: a possible duplicate of a title being written. Advice only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SimilarStory {
    pub key: Key,
    pub kind: Kind,
    pub title: String,
    pub epic: Option<Key>,
    pub delivery: DeliveryState,
    /// Set when `delivery` is done: completed or dropped.
    pub resolution: Option<Resolution>,
}
