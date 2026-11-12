use crate::ids::Key;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Which done stories `GET /stories` lists: every one (the default), only those that entered done within the
/// instance's `[board] done_window_days`, or none. Stories in other states are always listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DoneFilter {
    Recent,
    #[default]
    All,
    /// No done stories: todo, in progress, in review and testing.
    None,
}

/// The story list's order: `rank` (the default: delivery state, rank, key number) or `relevance` (needs `q`: the
/// story whose key or external reference equals the text first, then title matches, then the rest; within each,
/// the `rank` order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StorySort {
    #[default]
    Rank,
    Relevance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Story,
    Bug,
    Chore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    Todo,
    InProgress,
    InReview,
    Testing,
    Done,
}

impl DeliveryState {
    /// The three states in which work is happening; the gate guards entry into these.
    pub fn is_active(self) -> bool {
        matches!(self, Self::InProgress | Self::InReview | Self::Testing)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    Completed,
    Dropped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TokenKind {
    Interactive,
    Automation,
}

/// How hard a required decision is to undo (kb/ADR-0013 R1). Unset (`None`) behaves as two-way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Reversibility {
    OneWay,
    TwoWay,
}

/// What a close says (kb/ADR-0013 R3): the decision was made, or it is no longer needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClosureKind {
    Decided,
    Withdrawn,
}

/// Where a decision sits in the queue; the declaration order is the page order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueueGroup {
    Blocking,
    Open,
    Satisfied,
    Closed,
}

/// When an open, unsatisfied decision is needed, derived on read from goals, discovery and the promises of the stories it
/// affects (never stored, kb/ADR-0014 R6, kb/ADR-0025 R7). The variant order is the order of urgency, earliest first:
/// the queue takes the earliest of several windows with `min`, so the variants must not be reordered.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum NeededIn {
    ThisSprint,
    NextSprint,
    Beyond,
    #[default]
    None,
}

/// Why a decision is needed this sprint or next. Clients must accept `unknown`: a later server may send a reason
/// this build does not know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NeededInReason {
    /// An affected story serves this goal (of the active sprint for `this_sprint`, the draft one for `next_sprint`).
    Goal { goal_id: i64, outcome: String },
    /// The decision's item, or an ancestor, is in discovery under the first horizon bucket. `level` says whether `key` is an
    /// initiative or an epic, so a client can link to it. It defaults to `initiative` when absent, so a payload from before
    /// the field existed still parses; a discovery item is never a story.
    Discovery {
        key: Key,
        #[serde(default = "initiative_level")]
        #[schema(required = true)]
        level: ItemType,
    },
    /// An affected story is promised (kb/ADR-0025 R7); `key`/`level` name the item holding the promise (the story, or
    /// the epic it inherits from), `on` the effective date, and `overdue` whether that date has passed unfinished.
    Promise {
        key: Key,
        level: ItemType,
        on: chrono::NaiveDate,
        #[serde(default)]
        #[schema(required = true)]
        overdue: bool,
    },
    /// A reason added after this client was built.
    #[serde(other)]
    Unknown,
}

fn initiative_level() -> ItemType {
    ItemType::Initiative
}

/// What an open, unsatisfied decision needs next. The first that holds wins, in declaration order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NextStep {
    WriteRecord,
    ConfigureRepo {
        repo: String,
    },
    FixRecord {
        result: CheckResult,
    },
    ReviewRecord {
        pr_url: String,
    },
    AwaitCheck {
        checked_at: Option<chrono::DateTime<chrono::Utc>>,
    },
}

/// The queue lane the web groups by; declaration order is the page order. Lanes are disjoint.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum QueueLane {
    Review,
    ThisSprint,
    NextSprint,
    Beyond,
    #[default]
    Open,
    Satisfied,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    Member,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ItemType {
    Initiative,
    Epic,
    Story,
}

/// A pull request's state as GitHub last reported it; a cache, never a driver of delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrStatus {
    Draft,
    Open,
    Merged,
    Closed,
}

/// The outcome of the last record check on a required decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckResult {
    Unchecked,
    Accepted {
        commit: String,
        /// The record's own `reversibility` (kb/ADR-0013 R2); absent from older checkers and records without it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reversibility: Option<Reversibility>,
    },
    NotAccepted {
        status: String,
        superseded_by: Option<String>,
        commit: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reversibility: Option<Reversibility>,
    },
    ProposedPrOpen,
    NotFound,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delivery_states_serialise_as_spec_words() {
        let all = [
            DeliveryState::Todo,
            DeliveryState::InProgress,
            DeliveryState::InReview,
            DeliveryState::Testing,
            DeliveryState::Done,
        ];
        let words: Vec<String> = all
            .iter()
            .map(|s| serde_json::to_string(s).unwrap())
            .collect();
        assert_eq!(
            words,
            [
                "\"todo\"",
                "\"in_progress\"",
                "\"in_review\"",
                "\"testing\"",
                "\"done\""
            ]
        );
    }
    #[test]
    fn active_states_are_the_three_middle_ones() {
        assert!(!DeliveryState::Todo.is_active());
        assert!(DeliveryState::InProgress.is_active());
        assert!(DeliveryState::InReview.is_active());
        assert!(DeliveryState::Testing.is_active());
        assert!(!DeliveryState::Done.is_active());
    }
    #[test]
    fn kinds_and_resolutions() {
        assert_eq!(serde_json::to_string(&Kind::Chore).unwrap(), "\"chore\"");
        assert_eq!(
            serde_json::to_string(&Resolution::Dropped).unwrap(),
            "\"dropped\""
        );
        assert_eq!(
            serde_json::to_string(&TokenKind::Automation).unwrap(),
            "\"automation\""
        );
    }
    #[test]
    fn remaining_enums_and_check_result_wire_shapes() {
        use serde_json::json;
        let cases = [
            (
                serde_json::to_value(CheckResult::Unchecked).unwrap(),
                json!({"kind":"unchecked"}),
            ),
            (
                serde_json::to_value(CheckResult::Accepted {
                    commit: "c1".into(),
                    reversibility: None,
                })
                .unwrap(),
                json!({"kind":"accepted","commit":"c1"}),
            ),
            (
                serde_json::to_value(CheckResult::NotAccepted {
                    status: "superseded".into(),
                    superseded_by: Some("ADR-2".into()),
                    commit: "c2".into(),
                    reversibility: None,
                })
                .unwrap(),
                json!({"kind":"not_accepted","status":"superseded","superseded_by":"ADR-2","commit":"c2"}),
            ),
            (
                serde_json::to_value(CheckResult::ProposedPrOpen).unwrap(),
                json!({"kind":"proposed_pr_open"}),
            ),
            (
                serde_json::to_value(CheckResult::NotFound).unwrap(),
                json!({"kind":"not_found"}),
            ),
            (
                serde_json::to_value(Resolution::Completed).unwrap(),
                json!("completed"),
            ),
            (
                serde_json::to_value(ItemType::Initiative).unwrap(),
                json!("initiative"),
            ),
            (serde_json::to_value(Role::Admin).unwrap(), json!("admin")),
        ];
        for (got, want) in cases {
            assert_eq!(got, want);
        }
    }

    /// A discovery reason carries the item's level, and one without it (from before the field) reads as an initiative.
    #[test]
    fn discovery_reason_level_round_trips_and_defaults() {
        use serde_json::json;
        let epic = NeededInReason::Discovery {
            key: "ABC-5".parse().unwrap(),
            level: ItemType::Epic,
        };
        let v = serde_json::to_value(&epic).unwrap();
        assert_eq!(
            v,
            json!({"kind": "discovery", "key": "ABC-5", "level": "epic"})
        );
        assert_eq!(serde_json::from_value::<NeededInReason>(v).unwrap(), epic);
        let old: NeededInReason =
            serde_json::from_value(json!({"kind": "discovery", "key": "ABC-4"})).unwrap();
        assert_eq!(
            old,
            NeededInReason::Discovery {
                key: "ABC-4".parse().unwrap(),
                level: ItemType::Initiative
            }
        );
    }
}
