//! The planning context: initiatives, epics, stories, required decisions, comments; readiness and the delivery rules.
mod domain;
mod handler;
mod ports;
mod repo;
pub use domain::{
    decision::{CloseError, Closed, RecordRef, RequiredDecision},
    delivery::{MoveEffect, MoveRefused, MoveRequest, validate_move},
    items::{Criterion, DiscoveryMark, Epic, Initiative, ItemError, Story},
    progress::progress,
    pull_request::{PrChange, PullRequest},
    queue::{BlockedStory, QueueEntry, QueueSummary},
    readiness::{AncestorFact, DecisionFact, ReadinessInput, compute_readiness},
    sprint::{
        DemoStory, Goal, GoalLink, GoalProgress, Sprint, SprintCounts, StoryTimes, Summary,
        summarise,
    },
    urgency::{Lineage, MarkFact, MarkState, Urgency, UrgentMark, expires_at, urgency},
};
pub use handler::*;
pub use ports::{
    ActivityRecord, ActivitySink, CommentRepo, DecisionRepo, EpicRepo, GoalFilter, InitiativeRepo,
    KeyRepo, PlanningPorts, PlanningReads, PullRequestRepo, RepoError, SearchRow, SprintRepo,
    StoryCounts, StoryFilter, StoryRepo, UrgencyRepo,
};
pub use reeve_protocol::ItemType;
pub use repo::make_planning_ports;
pub fn migrator() -> sqlx::migrate::Migrator {
    sqlx::migrate!("./migrations")
}
