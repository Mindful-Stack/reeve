//! The planning context: initiatives, epics, stories, required decisions, comments; readiness and the delivery rules.
mod domain;
mod handler;
mod ports;
mod repo;
mod repo_sprint;
pub use domain::{
    decision::{CloseError, Closed, RecordRef, RequiredDecision},
    delivery::{MoveEffect, MoveRefused, MoveRequest, validate_move},
    items::{Criterion, DiscoveryMark, Epic, Initiative, ItemError, Story},
    progress::progress,
    pull_request::{PrChange, PullRequest},
    queue::{BlockedStory, QueueEntry, QueueSummary},
    readiness::{AncestorFact, DecisionFact, ReadinessInput, compute_readiness},
    sprint::{Goal, GoalLink, Sprint, StoryTimes, Summary, goal_dto, sprint_dto, sprint_view_dto},
};
pub use handler::*;
pub use ports::{
    ActivityRecord, ActivitySink, GoalFilter, PlanningRepo, RepoError, SearchRow, StoryCounts,
    StoryFilter,
};
pub use reeve_protocol::ItemType;
pub use repo::make_planning_repo;
pub fn migrator() -> sqlx::migrate::Migrator {
    sqlx::migrate!("./migrations")
}
