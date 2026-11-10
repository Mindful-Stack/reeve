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
use reeve_protocol::{DeliveryState, ItemType, Key, Kind};
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

#[derive(Debug, Clone, Default)]
pub struct StoryFilter {
    pub epic: Option<Key>,
    pub owner: Option<String>,
    pub delivery: Option<DeliveryState>,
    /// Items of any kind without an epic that are not done (kb/ADR-0018 R4).
    pub untriaged: bool,
    /// Stories whose delivery is not done (kb/ADR-0018 R5), of any state otherwise; combines with every other filter.
    pub unfinished: bool,
    pub label: Option<String>,
    /// Goal membership, applied after the stories are read (`goal=<id>|any|none`); combines with every other filter.
    pub goal: Option<GoalFilter>,
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
mod comments;
mod decisions;
mod epics;
mod initiatives;
mod keys;
mod pull_requests;
mod reads;
mod sprints;
mod stories;
mod urgency;

pub use self::{
    comments::CommentRepo, decisions::DecisionRepo, epics::EpicRepo, initiatives::InitiativeRepo,
    keys::KeyRepo, pull_requests::PullRequestRepo, reads::PlanningReads, sprints::SprintRepo,
    stories::StoryRepo, urgency::UrgencyRepo,
};

/// Every planning port, each a view of one adapter (`frameworks/rust/repo-port-adapter`, "Single-impl,
/// two-trait-views"), built by `make_planning_ports`.
#[derive(Clone)]
pub struct PlanningPorts {
    pub keys: Arc<dyn KeyRepo>,
    pub initiatives: Arc<dyn InitiativeRepo>,
    pub epics: Arc<dyn EpicRepo>,
    pub stories: Arc<dyn StoryRepo>,
    pub decisions: Arc<dyn DecisionRepo>,
    pub comments: Arc<dyn CommentRepo>,
    pub pull_requests: Arc<dyn PullRequestRepo>,
    pub sprints: Arc<dyn SprintRepo>,
    pub urgency: Arc<dyn UrgencyRepo>,
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
