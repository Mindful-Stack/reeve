//! The planning context: initiatives, epics, stories, required decisions, comments; readiness and the delivery rules.
mod domain;
mod handler;
mod ports;
mod repo;
pub use domain::{
    attachment::{
        Attachment, AttachmentOwner, CHUNK, ImageType, MAX_BYTES, UploadError, chunks,
        clean_filename, strip_metadata,
    },
    decision::{
        CheckOutcome, Closed, DecisionEdit, DecisionError, FieldChange, PrUrl, RecordRef,
        RequiredDecision,
    },
    items::{
        Criterion, DiscoveryMark, Epic, Initiative, ItemError, Move, MoveRefused, Moved, Story,
    },
    progress::progress,
    promise,
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
    ActivityRecord, ActivitySink, AttachmentRepo, ClaimOutcome, CommentRepo, DecisionRepo,
    DoneWindow, EpicRepo, GoalFilter, InitiativeRepo, KeyRepo, PlanningPorts, PlanningReads,
    PromiseFacts, PromiseRepo, PromiseTarget, PromisedStoryRow, PullRequestRepo, RepoError,
    SearchRow, SprintRepo, StoryCounts, StoryFilter, StoryPromiseRow, StoryRepo, StoryText,
    UrgencyRepo,
};
pub use reeve_protocol::ItemType;
pub use repo::make_planning_ports;
pub fn migrator() -> sqlx::migrate::Migrator {
    sqlx::migrate!("./migrations")
}
