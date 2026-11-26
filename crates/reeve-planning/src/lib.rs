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
    brief_template::{BriefTemplate, BriefTemplateError},
    decision::{
        CheckOutcome, Closed, DecisionEdit, DecisionError, FieldChange, PrUrl, RecordRef,
        RequiredDecision,
    },
    item_link::{ItemLink, ItemLinkError},
    items::{
        Criterion, DiscoveryMark, Epic, Initiative, ItemError, Move, MoveRefused, Moved, Story,
    },
    progress::progress,
    promise::{
        Clock as PromiseClock, HeldBy, Promisable, Promise, PromiseItem, SprintPromise, StoryFacts,
        StoryPromise, story_promise,
    },
    pull_request::{PrChange, PullRequest},
    queue::{DiscoveryRow, LineageItem, QueueEntry, QueueStory, QueueSummary},
    readiness::{AncestorFact, BlockerFact, DecisionFact, ReadinessInput, compute_readiness},
    sprint::{
        BucketGoal, BucketInitiative, DecisionGate, DemoStory, DiscoveryGate, Goal, GoalGates,
        GoalLink, GoalProgress, PlanningGoal, ReachedInitiative, Sprint, SprintCounts, StoryTimes,
        Summary, first_bucket_initiatives, goal_gates, is_first_bucket, planning_goals, summarise,
    },
    urgency::{Lineage, MarkFact, MarkState, Urgency, UrgentMark, expires_at, urgency},
    web_link::{WebLink, WebLinkError},
};
pub use handler::*;
pub use ports::{
    ActivityRecord, ActivitySink, AttachmentRepo, BriefTemplateRepo, ClaimOutcome, CommentRepo,
    DecisionRepo, DoneWindow, EpicRepo, GoalFilter, InitiativeRepo, ItemHead, ItemLinkRepo,
    KeyRepo, PlanningPorts, PlanningReads, PromiseFacts, PromiseRepo, PromiseTarget,
    PromisedStoryRow, PullRequestRepo, RepoError, SearchRow, SprintRepo, StoredBriefTemplate,
    StoredWebLink, StoryCounts, StoryFilter, StoryPromiseRow, StoryRepo, StoryText, UrgencyRepo,
    WebLinkRepo,
};
pub use reeve_protocol::ItemType;
pub use repo::make_planning_ports;
pub fn migrator() -> sqlx::migrate::Migrator {
    sqlx::migrate!("./migrations")
}
