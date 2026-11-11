//! The planning context: initiatives, epics, stories, required decisions, comments; readiness and the delivery rules.
mod domain;
mod handler;
mod ports;
mod repo;
pub use domain::{
    decision::{
        CheckOutcome, Closed, DecisionEdit, DecisionError, FieldChange, PrUrl, RecordRef,
        RequiredDecision,
    },
    items::{
        Criterion, DiscoveryMark, Epic, Initiative, ItemError, Move, MoveRefused, Moved, Story,
    },
    progress::progress,
    pull_request::{PrChange, PullRequest},
    queue::{LineageItem, QueueEntry, QueueStory, QueueSummary},
    readiness::{AncestorFact, DecisionFact, ReadinessInput, compute_readiness},
    sprint::{
        DemoStory, Goal, GoalLink, GoalProgress, Sprint, SprintCounts, StoryTimes, Summary,
        summarise,
    },
    urgency::{Lineage, MarkFact, MarkState, Urgency, UrgentMark, expires_at, urgency},
};
pub use handler::*;
pub use ports::{
    ActivityRecord, ActivitySink, CommentRepo, DecisionRepo, DoneWindow, EpicRepo, GoalFilter,
    InitiativeRepo, KeyRepo, PlanningPorts, PlanningReads, PullRequestRepo, RepoError, SearchRow,
    SprintRepo, StoryCounts, StoryFilter, StoryRepo, StoryText, UrgencyRepo,
};
pub use reeve_protocol::ItemType;
pub use repo::make_planning_ports;
pub fn migrator() -> sqlx::migrate::Migrator {
    sqlx::migrate!("./migrations")
}
