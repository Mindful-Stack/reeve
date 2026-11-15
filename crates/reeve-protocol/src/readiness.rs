use crate::enums::DeliveryState;
use crate::ids::Key;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Why a story is blocked, or why a move was refused: a readiness fact, which an interactive override may get past, or a
/// delivery rule, which nothing will. `Reason::overridable` (and a refusal's `overridable`) says which.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Reason {
    Untriaged,
    NoCriteria,
    AncestorDiscoveryIncomplete {
        item: Key,
    },
    DecisionNoRecord {
        decision_id: i64,
        title: String,
    },
    DecisionUnchecked {
        decision_id: i64,
        title: String,
    },
    DecisionProposedPrOpen {
        decision_id: i64,
        title: String,
    },
    DecisionRecordNotFound {
        decision_id: i64,
        title: String,
    },
    /// The record is in a repository the server does not check yet; it is checked once the repository is configured.
    DecisionRepoNotConfigured {
        decision_id: i64,
        title: String,
    },
    DecisionNotAccepted {
        decision_id: i64,
        title: String,
        status: String,
        superseded_by: Option<String>,
    },
    /// A blocking story that is unfinished for this story (kb/ADR-0027 R5), with what decides when it unblocks: its
    /// delivery (merged, it blocks nobody) and its owner (who can build on it, or whom to ask).
    BlockedBy {
        story: Key,
        delivery: DeliveryState,
        owner: Option<String>,
    },
    ResolutionRequired,
    CompletedNeedsActiveOrigin,
    OverrideNeedsReason,
    AutomationCannotOverride,
    /// A sprint is already active; close it first.
    SprintActive,
    /// Closing needs a verdict on every goal.
    VerdictMissing {
        goals: Vec<i64>,
    },
    SprintNotActive,
    SprintNotDraft,
    SprintClosed,
    /// Only an interactive user may do this (kb/ADR-0014 R5).
    InteractiveOnly,
}

impl Reason {
    /// The required decision this reason waits on, if it is one.
    pub fn decision_id(&self) -> Option<i64> {
        match self {
            Reason::DecisionNoRecord { decision_id, .. }
            | Reason::DecisionUnchecked { decision_id, .. }
            | Reason::DecisionProposedPrOpen { decision_id, .. }
            | Reason::DecisionRecordNotFound { decision_id, .. }
            | Reason::DecisionRepoNotConfigured { decision_id, .. }
            | Reason::DecisionNotAccepted { decision_id, .. } => Some(*decision_id),
            Reason::Untriaged
            | Reason::NoCriteria
            | Reason::AncestorDiscoveryIncomplete { .. }
            | Reason::BlockedBy { .. }
            | Reason::ResolutionRequired
            | Reason::CompletedNeedsActiveOrigin
            | Reason::OverrideNeedsReason
            | Reason::AutomationCannotOverride
            | Reason::SprintActive
            | Reason::VerdictMissing { .. }
            | Reason::SprintNotActive
            | Reason::SprintNotDraft
            | Reason::SprintClosed
            | Reason::InteractiveOnly => None,
        }
    }
    /// A readiness fact (an interactive override may get past it) rather than a delivery rule (nothing will).
    pub fn overridable(&self) -> bool {
        match self {
            Reason::Untriaged
            | Reason::NoCriteria
            | Reason::AncestorDiscoveryIncomplete { .. }
            | Reason::DecisionNoRecord { .. }
            | Reason::DecisionUnchecked { .. }
            | Reason::DecisionProposedPrOpen { .. }
            | Reason::DecisionRecordNotFound { .. }
            | Reason::DecisionRepoNotConfigured { .. }
            | Reason::DecisionNotAccepted { .. }
            | Reason::BlockedBy { .. } => true,
            Reason::ResolutionRequired
            | Reason::CompletedNeedsActiveOrigin
            | Reason::OverrideNeedsReason
            | Reason::AutomationCannotOverride
            | Reason::SprintActive
            | Reason::VerdictMissing { .. }
            | Reason::SprintNotActive
            | Reason::SprintNotDraft
            | Reason::SprintClosed
            | Reason::InteractiveOnly => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Readiness {
    pub ready: bool,
    pub reasons: Vec<Reason>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every decision reason names its decision; nothing else does. Catches a new decision variant left out of the
    /// helper (the queue would silently stop counting stories it blocks).
    #[test]
    fn decision_id_is_set_exactly_on_decision_reasons() {
        let k = || -> Key { "ABC-3".parse().unwrap() };
        let t = || "T".to_string();
        for r in [
            Reason::Untriaged,
            Reason::NoCriteria,
            Reason::AncestorDiscoveryIncomplete { item: k() },
            Reason::DecisionNoRecord {
                decision_id: 1,
                title: t(),
            },
            Reason::DecisionUnchecked {
                decision_id: 1,
                title: t(),
            },
            Reason::DecisionProposedPrOpen {
                decision_id: 1,
                title: t(),
            },
            Reason::DecisionRecordNotFound {
                decision_id: 1,
                title: t(),
            },
            Reason::DecisionRepoNotConfigured {
                decision_id: 1,
                title: t(),
            },
            Reason::DecisionNotAccepted {
                decision_id: 1,
                title: t(),
                status: "rejected".into(),
                superseded_by: None,
            },
            Reason::BlockedBy {
                story: k(),
                delivery: DeliveryState::Todo,
                owner: None,
            },
            Reason::ResolutionRequired,
            Reason::CompletedNeedsActiveOrigin,
            Reason::OverrideNeedsReason,
            Reason::AutomationCannotOverride,
            Reason::SprintActive,
            Reason::VerdictMissing { goals: vec![1] },
            Reason::SprintNotActive,
            Reason::SprintNotDraft,
            Reason::SprintClosed,
            Reason::InteractiveOnly,
        ] {
            let named = matches!(
                r,
                Reason::DecisionNoRecord { .. }
                    | Reason::DecisionUnchecked { .. }
                    | Reason::DecisionProposedPrOpen { .. }
                    | Reason::DecisionRecordNotFound { .. }
                    | Reason::DecisionRepoNotConfigured { .. }
                    | Reason::DecisionNotAccepted { .. }
            );
            assert_eq!(r.decision_id().is_some(), named, "{r:?}");
        }
    }
    #[test]
    fn readiness_facts_are_overridable_and_delivery_rules_are_not() {
        let k = || -> Key { "ABC-3".parse().unwrap() };
        let t = || "T".to_string();
        let facts = [
            Reason::Untriaged,
            Reason::NoCriteria,
            Reason::AncestorDiscoveryIncomplete { item: k() },
            Reason::DecisionNoRecord {
                decision_id: 1,
                title: t(),
            },
            Reason::DecisionUnchecked {
                decision_id: 1,
                title: t(),
            },
            Reason::DecisionProposedPrOpen {
                decision_id: 1,
                title: t(),
            },
            Reason::DecisionRecordNotFound {
                decision_id: 1,
                title: t(),
            },
            Reason::DecisionNotAccepted {
                decision_id: 1,
                title: t(),
                status: "rejected".into(),
                superseded_by: None,
            },
            Reason::BlockedBy {
                story: k(),
                delivery: DeliveryState::Todo,
                owner: None,
            },
        ];
        let rules = [
            Reason::ResolutionRequired,
            Reason::CompletedNeedsActiveOrigin,
            Reason::OverrideNeedsReason,
            Reason::AutomationCannotOverride,
            Reason::SprintActive,
            Reason::VerdictMissing { goals: vec![1] },
            Reason::SprintNotActive,
            Reason::SprintNotDraft,
            Reason::SprintClosed,
            Reason::InteractiveOnly,
        ];
        for r in &facts {
            assert!(r.overridable(), "{r:?}");
        }
        for r in &rules {
            assert!(!r.overridable(), "{r:?}");
        }
    }
    #[test]
    fn reason_tags_are_snake_case() {
        let b = serde_json::to_value(Reason::BlockedBy {
            story: "ABC-3".parse().unwrap(),
            delivery: DeliveryState::InReview,
            owner: Some("dan@example.com".into()),
        })
        .unwrap();
        assert_eq!(
            b,
            serde_json::json!({"reason":"blocked_by","story":"ABC-3","delivery":"in_review","owner":"dan@example.com"})
        );
        let d = serde_json::to_value(Reason::DecisionNotAccepted {
            decision_id: 1,
            title: "t".into(),
            status: "rejected".into(),
            superseded_by: None,
        })
        .unwrap();
        assert_eq!(d["reason"], "decision_not_accepted");
        let u = serde_json::to_value(Reason::DecisionUnchecked {
            decision_id: 1,
            title: "t".into(),
        })
        .unwrap();
        assert_eq!(u["reason"], "decision_unchecked");
    }
}
