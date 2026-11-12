//! Promise wire types (kb/ADR-0025): a stored promise and what the server derives from it on read.
use crate::ids::Key;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A stored promise (kb/ADR-0025 R1): the date, who set it, when, and an optional recipient.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PromiseFactDto {
    pub on: NaiveDate,
    pub by: String,
    pub at: DateTime<Utc>,
    pub to: Option<String>,
}

/// A story's promise state, derived on read (R4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PromiseState {
    Open,
    Overdue,
    Kept,
    Late,
    Dropped,
    /// A state added after this client was built; shown without a state word.
    #[serde(other)]
    Unknown,
}

/// An open promise's due period against the sprints' planned ends, derived on read (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PromiseDue {
    ThisSprint,
    NextSprint,
    Later,
    /// A period added after this client was built; shown without a period.
    #[serde(other)]
    Unknown,
}

/// Whose promise sets a story's effective date: its own, or its epic's (R3; the story's own wins a tie).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PromiseHolder {
    Own,
    Epic,
    /// A holder added after this client was built.
    #[serde(other)]
    Unknown,
}

/// The epic whose promise a story inherits, and that promise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct EpicPromiseRefDto {
    pub key: Key,
    pub fact: PromiseFactDto,
}

/// A story's promise as derived on read (R3–R5): both promises shown, `on` the effective date, `holder` whose it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct StoryPromiseDto {
    pub own: Option<PromiseFactDto>,
    /// Only when the story inherits its epic's promise (R3).
    pub epic: Option<EpicPromiseRefDto>,
    pub on: NaiveDate,
    pub holder: PromiseHolder,
    /// None only for a done story without a completion time.
    pub state: Option<PromiseState>,
    /// Only for `open`.
    pub due: Option<PromiseDue>,
    /// Days overdue or late, computed by the server.
    pub days: Option<u32>,
}

/// An epic's own promise: no state (R4). `due` while the date is not past; `past` once it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct EpicPromiseDto {
    pub fact: PromiseFactDto,
    pub due: Option<PromiseDue>,
    pub past: bool,
}

/// A sprint whose planned end a promise takes (R6): the active sprint or the draft (next) one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PromiseShortcut {
    ThisSprint,
    NextSprint,
}

/// `POST /stories/{key}/promise`, `POST /epics/{key}/promise`. A date or a shortcut, not both; neither changes only
/// the recipient of the current promise. `to` left out keeps the recipient; an empty string clears it.
/// Any other field is refused rather than ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetPromise {
    #[serde(default)]
    pub on: Option<NaiveDate>,
    #[serde(default)]
    pub shortcut: Option<PromiseShortcut>,
    /// The recipient. Left out or `null`: keep the current one. `""` (or only spaces): clear it.
    #[serde(default)]
    pub to: Option<String>,
}

/// The story list's `promised` filter (kb/ADR-0025 R5, kb/ADR-0021): `this_sprint` keeps overdue promises and open
/// ones due this sprint, `next_sprint` open ones due next sprint, `overdue` only overdue ones, `any` every open or
/// overdue promise. Kept, late and dropped promises never match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PromisedFilter {
    ThisSprint,
    NextSprint,
    Overdue,
    Any,
}

/// One row of a sprint's promise list (R8): a story whose effective date falls after the previous sprint's planned
/// end and up to this sprint's, derived on read. `to` is the recipient of the promise that sets the date.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct SprintPromiseDto {
    pub key: Key,
    pub title: String,
    /// The effective date (R3).
    pub on: NaiveDate,
    pub to: Option<String>,
    pub holder: PromiseHolder,
    /// The key of the epic whose promise this is, when `holder` is `epic`.
    pub epic_key: Option<Key>,
    pub state: PromiseState,
    /// The UTC day the story was done, when it was.
    pub done_on: Option<NaiveDate>,
    /// Days overdue or late, computed by the server.
    pub days: Option<u32>,
}
