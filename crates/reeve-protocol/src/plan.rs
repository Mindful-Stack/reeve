//! `POST /plans` (`reeve apply`): a plan of new items, required decisions and blockers, sent as one request.
use crate::{enums::*, ids::Key};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Items refer to each other by their local `name`; a reference that is no name in the plan is an existing item's key
/// or external reference. The whole plan is validated before anything is written, and applying the same plan (`id`)
/// again creates nothing: every write is keyed `plan:<id>:<name>`. Unknown fields are refused, so a typo is an error,
/// not a silently dropped field.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyPlan {
    /// Names this plan: the same id and names are the same items on a second apply.
    pub id: String,
    #[serde(default)]
    pub initiatives: Vec<PlanInitiative>,
    #[serde(default)]
    pub epics: Vec<PlanEpic>,
    #[serde(default)]
    pub stories: Vec<PlanStory>,
    #[serde(default)]
    pub decisions: Vec<PlanDecision>,
    #[serde(default)]
    pub blocks: Vec<PlanBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanInitiative {
    pub name: String,
    pub title: String,
    pub horizon: String,
    #[serde(default)]
    pub brief: Option<String>,
    #[serde(default)]
    pub target_month: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub external_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanEpic {
    pub name: String,
    pub title: String,
    /// A plan initiative's name, or an existing initiative's key.
    #[serde(default)]
    pub initiative: Option<String>,
    #[serde(default)]
    pub brief: Option<String>,
    #[serde(default)]
    pub external_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanStory {
    pub name: String,
    pub title: String,
    /// story when absent.
    #[serde(default)]
    pub kind: Option<Kind>,
    /// A plan epic's name, or an existing epic's key.
    #[serde(default)]
    pub epic: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub criteria: Vec<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub external_ref: Option<String>,
    /// The state of the migrated ticket: todo when absent. Any state needs `external_ref`; one other than
    /// todo is created in that state and logged `imported`, as the importer does, never `override`. Done is the
    /// importer's alone (it needs a resolution).
    #[serde(default)]
    pub delivery: Option<DeliveryState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanDecision {
    pub name: String,
    /// A plan item's name, or an existing item's key.
    pub on: String,
    pub title: String,
    pub decider: String,
    #[serde(default)]
    pub due: Option<chrono::NaiveDate>,
    /// One-way or two-way; absent leaves it unset (kb/ADR-0013 R1).
    #[serde(default)]
    pub reversibility: Option<Reversibility>,
}

/// `blocked_by` blocks `story`; each is a plan story's name or an existing story's key.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanBlock {
    pub story: String,
    pub blocked_by: String,
}

/// What the plan came to: every item and decision by its local name, with `created` false when an earlier apply of
/// this plan had made it, and every blocker edge with `created` false when it was already there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PlanApplied {
    pub items: Vec<PlannedItem>,
    pub decisions: Vec<PlannedDecision>,
    pub blocks: Vec<PlannedBlock>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PlannedItem {
    pub name: String,
    pub item: ItemType,
    pub key: Key,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PlannedDecision {
    pub name: String,
    pub id: i64,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PlannedBlock {
    pub story: Key,
    pub blocked_by: Key,
    pub created: bool,
}
