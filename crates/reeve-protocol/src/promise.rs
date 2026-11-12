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
}

/// An open promise's due period against the sprints' planned ends, derived on read (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PromiseDue {
    ThisSprint,
    NextSprint,
    Later,
}

/// Whose promise sets a story's effective date: its own, or its epic's (R3; the story's own wins a tie).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PromiseHolder {
    Own,
    Epic,
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
