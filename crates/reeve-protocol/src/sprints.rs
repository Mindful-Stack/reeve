//! Sprint and goal wire types (kb/ADR-0014): enums, counts, views and requests.
use crate::ids::{Key, KeyRef};
use crate::promise::SprintPromiseDto;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SprintState {
    Draft,
    Active,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    OnTrack,
    AtRisk,
    OffTrack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Met,
    PartlyMet,
    Missed,
}

/// Through which item a story serves a goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GoalVia {
    Story,
    Epic,
    Initiative,
}

/// Advice on opening or adding goals; never a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SprintWarning {
    NoGoals,
    MoreThanThreeGoals,
}

/// One goal's counts over a sprint's window (kb/ADR-0014 R2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GoalProgressDto {
    pub goal_id: i64,
    pub completed: u32,
    pub in_flight: u32,
    pub todo: u32,
    pub in_review: u32,
    /// Live: recomputed on every read from the decisions open now, even for a closed sprint. For a closed sprint, show
    /// the value at close instead, from the `counts` in the sprint's `closed` activity entry; this one is today's.
    pub blocked_decisions: u32,
}

/// The one counts type: `summarise` builds it, the sprint view serves it, and the `closed` activity entry carries it as
/// "at close" (kb/ADR-0014 R6), so the payload and `GET /sprints/{id}`'s counts are the same type, serialised the same way.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SprintCountsDto {
    pub goals: Vec<GoalProgressDto>,
    pub completed: u32,
    pub upkeep_completed: u32,
}

/// One active-sprint goal a story serves, and through which item (the nearest link wins).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct GoalRefDto {
    pub id: i64,
    pub outcome: String,
    pub via: GoalVia,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct GoalDto {
    pub id: i64,
    pub sprint_id: i64,
    pub outcome: String,
    /// "We'll know when …".
    pub check: String,
    pub owner: String,
    pub notes: String,
    pub confidence: Option<Confidence>,
    pub confidence_note: Option<String>,
    pub confidence_at: Option<DateTime<Utc>>,
    /// A draft from an agent or automation, waiting for a person to record or discard it (kb/ADR-0014 R5).
    pub suggested_confidence: Option<Confidence>,
    pub suggested_note: Option<String>,
    pub suggested_at: Option<DateTime<Utc>>,
    pub suggested_by: Option<String>,
    pub verdict: Option<Verdict>,
    pub verdict_note: Option<String>,
    pub version: i64,
    /// The items linked to this goal, in key-number order; an epic or initiative link covers their stories.
    pub links: Vec<Key>,
}

/// A goal the story serves, for story context: the goal, its sprint's dates and through which item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct GoalContextDto {
    pub goal: GoalDto,
    pub starts_on: NaiveDate,
    pub ends_on: NaiveDate,
    pub via: GoalVia,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SprintDto {
    pub id: i64,
    pub state: SprintState,
    pub starts_on: NaiveDate,
    pub ends_on: NaiveDate,
    pub opened_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub version: i64,
    pub goals: Vec<GoalDto>,
    /// Advice from the goal count, for a draft or the active sprint only; a closed sprint has none.
    pub warnings: Vec<SprintWarning>,
}

/// A story completed in the window's last seven days (never before its start), oldest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DemoStoryDto {
    pub key: Key,
    pub title: String,
}

/// A sprint with its numbers, derived on read over its window (kb/ADR-0014 R6): a story counts in the one sprint whose
/// window holds its completion; one started and not completed by the window's end is in flight, never done work
/// (kb/ADR-0014 R2). A draft has no window: its counts are zero and `demo` is empty. A closed sprint's counts at close
/// are its `closed` activity entry's `counts`, read from `GET /activity`, never from here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SprintViewDto {
    pub sprint: SprintDto,
    pub counts: SprintCountsDto,
    /// `[sprints] upkeep_line`: the team's reference percentage for the upkeep share (not a budget).
    pub upkeep_line: u8,
    pub demo: Vec<DemoStoryDto>,
    /// The sprint's promises (kb/ADR-0025 R8), derived on read: the stories whose effective date is after the previous
    /// sprint's planned end and up to this one's, by date then key. Never stored, at close or elsewhere. `GET /sprints/{id}`
    /// fills it for active and closed sprints; always empty for a draft. `GET /sprints` leaves the active sprint's list
    /// empty (and `promises_after` null) until a client renders it there: read it from `GET /sprints/{id}`.
    #[serde(default)]
    #[schema(required = true)]
    pub promises: Vec<SprintPromiseDto>,
    /// The previous sprint's planned end, which `promises` start after (kb/ADR-0025 R8): the dates listed run from the
    /// day after it to this sprint's `ends_on`. Null for the first sprint, which takes every date up to its end, for
    /// a draft, and for the active sprint in `GET /sprints`.
    #[serde(default)]
    #[schema(required = true)]
    pub promises_after: Option<NaiveDate>,
}

/// `GET /sprints`: the active sprint with its numbers, the draft, and the closed sprints newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SprintsDto {
    pub active: Option<SprintViewDto>,
    pub draft: Option<SprintDto>,
    pub closed: Vec<SprintDto>,
    /// The server's UTC date for this request, so clients never read their own clock for promises (kb/ADR-0025). The
    /// server always fills it; it is optional only so that older JSON still parses.
    #[serde(default)]
    #[schema(required = true)]
    pub today: Option<NaiveDate>,
}

/// `POST /goals`: the goal and its sprint's warnings after the add.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct CreatedGoalDto {
    pub goal: GoalDto,
    pub warnings: Vec<SprintWarning>,
}

/// Opens the draft; the end defaults to one `[sprints] length_days` after today. `leave_out` names draft goals that stay
/// in the next draft (kb/ADR-0026); empty or absent opens every goal.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OpenSprint {
    pub version: i64,
    #[serde(default)]
    pub ends_on: Option<NaiveDate>,
    #[serde(default)]
    pub leave_out: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VerdictInput {
    pub goal_id: i64,
    pub verdict: Verdict,
    #[serde(default)]
    pub note: Option<String>,
}

/// Closes the active sprint; every goal needs a verdict.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CloseSprint {
    pub version: i64,
    pub verdicts: Vec<VerdictInput>,
}

/// Adds a goal to the draft, or to the active sprint with `active`. The owner defaults to the caller.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateGoal {
    pub idempotency_key: String,
    pub outcome: String,
    pub check: String,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub active: bool,
}

/// Each field is optional; an absent one is left as it is.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct EditGoal {
    pub version: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// A person's confidence (`POST /goals/{id}/confidence`, interactive only) or a suggestion for one
/// (`POST /goals/{id}/suggestion`, any token kind).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RecordConfidence {
    pub confidence: Confidence,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SuggestConfidence {
    pub confidence: Confidence,
    #[serde(default)]
    pub note: Option<String>,
}

/// Items to link, each a key or external reference of any level.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LinkGoal {
    pub items: Vec<KeyRef>,
}
