use crate::domain::{
    decision::*,
    items::*,
    pull_request::*,
    readiness::ReadinessInput,
    sprint::{Goal, GoalLink, Sprint, StoryTimes},
    urgency::{Lineage, MarkFact, UrgentMark},
};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use reeve_protocol::{DeliveryState, ItemType, Key, Kind, Resolution, StorySort};
use sqlx::{Postgres, Transaction};
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("not found")]
    NotFound,
    #[error("version conflict: current is {current}")]
    Conflict { current: i64 },
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// One item a text search matched, with the fields the match is judged on (`PlanningReads::search_items`).
#[derive(Debug, Clone)]
pub struct SearchRow {
    pub item: ItemType,
    pub key: Key,
    pub kind: Option<Kind>,
    pub title: String,
    /// An initiative's or an epic's brief, a story's description.
    pub text: String,
    pub external_ref: Option<String>,
}

/// What the story list keeps; every set field narrows it. `StoryRepo::list_stories` reads every field except `goal`
/// and `urgent`, which the handler applies after the read (on values computed in the same snapshot, kb/ADR-0021 R4).
#[derive(Debug, Clone, Default)]
pub struct StoryFilter {
    pub epic: Option<Key>,
    pub owner: Option<String>,
    pub delivery: Option<DeliveryState>,
    /// Items of any kind without an epic that are not done (kb/ADR-0018 R4).
    pub untriaged: bool,
    pub label: Option<String>,
    /// Goal membership, applied after the stories are read (`goal=<id>|any|none`); combines with every other filter.
    pub goal: Option<GoalFilter>,
    /// A case-insensitive substring of the title, description or external reference, or exactly the key it names.
    pub text: Option<StoryText>,
    pub kind: Option<Kind>,
    /// Stories whose epic is one of these; `Some(vec![])` lists nothing (an initiative with no epics, or a reference
    /// that named nothing).
    pub epics: Option<Vec<Key>>,
    /// Only stories shown as urgent (non-empty `urgent_sources`), applied by the handler after the read.
    pub urgent: bool,
    /// `Relevance` needs `text`; the handler refuses it without.
    pub sort: StorySort,
}

/// Text the story list matches, and the key it names (derived by the handler from the text, never by a caller).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryText {
    pub text: String,
    pub key: Option<Key>,
}

/// Which done stories the story list keeps; the handler computes the cutoff, so the clock stays out of the repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DoneWindow {
    #[default]
    All,
    /// Done stories that entered done at or after this instant (one with no `done_at` is not kept).
    Since(DateTime<Utc>),
    /// No done stories: every story whose delivery is not done, of any state otherwise (wire `done=none`; the
    /// decision queue's read, kb/ADR-0018 R5).
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalFilter {
    Id(i64),
    Any,
    None,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct StoryCounts {
    pub completed: u32,
    pub dropped: u32,
    pub total: u32,
}

// DecisionRepo, CommentRepo and SprintRepo each carry `#[allow(clippy::too_many_arguments)]` by design:
// some methods take the transaction, the key, the version and every field of the item, which is more than seven.
mod attachments;
mod comments;
mod decisions;
mod epics;
mod initiatives;
mod keys;
mod promise;
mod pull_requests;
mod reads;
mod sprints;
mod stories;
mod urgency;

pub use self::{
    attachments::{AttachmentRepo, ClaimOutcome},
    comments::CommentRepo,
    decisions::DecisionRepo,
    epics::EpicRepo,
    initiatives::InitiativeRepo,
    keys::KeyRepo,
    promise::{PromiseFacts, PromiseRepo, PromiseTarget, PromisedStoryRow, StoryPromiseRow},
    pull_requests::PullRequestRepo,
    reads::PlanningReads,
    sprints::SprintRepo,
    stories::StoryRepo,
    urgency::UrgencyRepo,
};

/// Every planning port, each a view of one adapter (`frameworks/rust/repo-port-adapter`, "Single-impl,
/// two-trait-views"), built by `make_planning_ports`.
#[derive(Clone)]
pub struct PlanningPorts {
    pub keys: Arc<dyn KeyRepo>,
    pub attachments: Arc<dyn AttachmentRepo>,
    pub initiatives: Arc<dyn InitiativeRepo>,
    pub epics: Arc<dyn EpicRepo>,
    pub stories: Arc<dyn StoryRepo>,
    pub decisions: Arc<dyn DecisionRepo>,
    pub comments: Arc<dyn CommentRepo>,
    pub pull_requests: Arc<dyn PullRequestRepo>,
    pub sprints: Arc<dyn SprintRepo>,
    pub urgency: Arc<dyn UrgencyRepo>,
    pub promises: Arc<dyn PromiseRepo>,
    pub reads: Arc<dyn PlanningReads>,
}

/// What the planning context needs from the activity log: defined here, implemented by reeve-activity.
#[derive(Debug, Clone)]
pub struct ActivityRecord {
    pub at: DateTime<Utc>,
    pub actor: String,
    pub via: reeve_protocol::Via,
    pub entity_type: &'static str,
    pub entity_key: String,
    pub action: &'static str,
    pub payload: serde_json::Value,
}

#[async_trait]
pub trait ActivitySink: Send + Sync {
    async fn record(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        rec: ActivityRecord,
    ) -> Result<(), RepoError>;
}
