use reeve_protocol::{CheckResult, DeliveryState, Key, Kind, Readiness, Reason};

#[derive(Debug, Clone)]
pub struct DecisionFact {
    pub id: i64,
    pub title: String,
    pub check: CheckResult,
    pub closed: bool,
    /// Whether a record reference is set (an unchecked decision with one is `DecisionUnchecked`).
    pub has_record: bool,
    /// The reference's repository, when one is set.
    pub record_repo: Option<String>,
    /// Whether that repository is configured, so the scheduled check reads it; the handler sets it from the settings
    /// (the repo does not know them). A record elsewhere is `DecisionRepoNotConfigured` until it is configured.
    pub repo_configured: bool,
}
#[derive(Debug, Clone)]
pub struct AncestorFact {
    pub key: Key,
    pub discovery_complete: bool,
    pub decisions: Vec<DecisionFact>,
}

/// One story that blocks the story being judged, with the two facts kb/ADR-0027 R5 reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockerFact {
    pub key: Key,
    pub delivery: DeliveryState,
    /// Normalised email, as stored.
    pub owner: Option<String>,
}

/// Whether `blocker` keeps a dependent owned by `dependent_owner` from being ready (kb/ADR-0027 R5): todo blocks
/// everyone; in progress or in review blocks unless both owners are set and equal; testing and done block nobody.
/// Each edge is judged alone, so a cycle of started stories with one owner does not block itself.
pub fn blocks(blocker: &BlockerFact, dependent_owner: Option<&str>) -> bool {
    match blocker.delivery {
        DeliveryState::Todo => true,
        DeliveryState::InProgress | DeliveryState::InReview => {
            !matches!((blocker.owner.as_deref(), dependent_owner), (Some(b), Some(d)) if b == d)
        }
        DeliveryState::Testing | DeliveryState::Done => false,
    }
}
/// Everything readiness needs, loaded by the repo in one query batch.
#[derive(Debug, Clone)]
pub struct ReadinessInput {
    /// Readiness deliberately ignores the kind: any kind without an epic is untriaged (kb/ADR-0027 R4).
    pub kind: Kind,
    pub has_epic: bool,
    pub criteria_count: usize,
    pub ancestors: Vec<AncestorFact>,
    pub own_decisions: Vec<DecisionFact>,
    /// The story's own owner, or the mover's when the gate judges an unowned story for a move (kb/ADR-0027 R6).
    pub owner: Option<String>,
    /// Every story that blocks this one, whatever its state; `blocks` decides which count. In key-number order.
    pub blockers: Vec<BlockerFact>,
}

fn decision_reason(d: &DecisionFact) -> Option<Reason> {
    if d.closed {
        return None;
    }
    Some(match &d.check {
        CheckResult::Accepted { .. } => return None,
        CheckResult::Unchecked if d.has_record && !d.repo_configured => {
            Reason::DecisionRepoNotConfigured {
                decision_id: d.id,
                title: d.title.clone(),
            }
        }
        CheckResult::Unchecked if d.has_record => Reason::DecisionUnchecked {
            decision_id: d.id,
            title: d.title.clone(),
        },
        CheckResult::Unchecked => Reason::DecisionNoRecord {
            decision_id: d.id,
            title: d.title.clone(),
        },
        CheckResult::ProposedPrOpen => Reason::DecisionProposedPrOpen {
            decision_id: d.id,
            title: d.title.clone(),
        },
        CheckResult::NotFound => Reason::DecisionRecordNotFound {
            decision_id: d.id,
            title: d.title.clone(),
        },
        CheckResult::NotAccepted {
            status,
            superseded_by,
            ..
        } => Reason::DecisionNotAccepted {
            decision_id: d.id,
            title: d.title.clone(),
            status: status.clone(),
            superseded_by: superseded_by.clone(),
        },
    })
}

/// The one check result that satisfies a required decision (kb/ADR-0027 R3). The check endpoint lets only an admin
/// store it; a test pins it against `decision_reason`, so the two cannot drift.
pub(crate) fn satisfies(check: &CheckResult) -> bool {
    matches!(check, CheckResult::Accepted { .. })
}

/// Reasons are ordered: untriaged (any kind without an epic, kb/ADR-0027 R4); then ancestors
/// in the order given (nearest first, each with its discovery reason then its decisions);
/// then own decisions; then no-criteria; then blockers that are unfinished for this story's owner (kb/ADR-0027 R5). Duplicates are not removed, so the
/// repo must supply unique ancestors and blockers.
pub fn compute_readiness(i: &ReadinessInput) -> Readiness {
    let mut reasons = Vec::new();
    if !i.has_epic {
        reasons.push(Reason::Untriaged);
    }
    for a in &i.ancestors {
        if !a.discovery_complete {
            reasons.push(Reason::AncestorDiscoveryIncomplete {
                item: a.key.clone(),
            });
        }
        reasons.extend(a.decisions.iter().filter_map(decision_reason));
    }
    reasons.extend(i.own_decisions.iter().filter_map(decision_reason));
    if i.criteria_count == 0 {
        reasons.push(Reason::NoCriteria);
    }
    reasons.extend(
        i.blockers
            .iter()
            .filter(|b| blocks(b, i.owner.as_deref()))
            .map(|b| Reason::BlockedBy {
                story: b.key.clone(),
                delivery: b.delivery,
                owner: b.owner.clone(),
            }),
    );
    Readiness {
        ready: reasons.is_empty(),
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reeve_protocol::{CheckResult, Kind, Reason};

    fn key(s: &str) -> reeve_protocol::Key {
        s.parse().unwrap()
    }
    fn decision(id: i64, check: CheckResult, closed: bool, has_record: bool) -> DecisionFact {
        DecisionFact {
            id,
            title: format!("d{id}"),
            check,
            closed,
            has_record,
            record_repo: has_record.then(|| "kb".into()),
            repo_configured: true,
        }
    }
    fn base() -> ReadinessInput {
        ReadinessInput {
            kind: Kind::Story,
            has_epic: true,
            criteria_count: 1,
            ancestors: vec![
                AncestorFact {
                    key: key("ABC-2"),
                    discovery_complete: true,
                    decisions: vec![],
                },
                AncestorFact {
                    key: key("ABC-1"),
                    discovery_complete: true,
                    decisions: vec![],
                },
            ],
            own_decisions: vec![],
            owner: Some("dan@example.com".into()),
            blockers: vec![],
        }
    }
    #[test]
    fn happy_path_is_ready() {
        assert!(compute_readiness(&base()).ready);
    }
    #[test]
    fn any_kind_without_epic_is_untriaged_first() {
        for kind in [Kind::Story, Kind::Bug, Kind::Chore] {
            let mut i = base();
            i.kind = kind;
            i.has_epic = false;
            i.ancestors.clear();
            assert_eq!(
                compute_readiness(&i).reasons,
                vec![Reason::Untriaged],
                "{kind:?}"
            );
            i.criteria_count = 0;
            assert_eq!(
                compute_readiness(&i).reasons,
                vec![Reason::Untriaged, Reason::NoCriteria],
                "{kind:?}: untriaged stays first"
            );
            i.has_epic = true;
            i.criteria_count = 1;
            assert!(
                compute_readiness(&i).ready,
                "{kind:?} with an epic is not untriaged"
            );
        }
    }
    #[test]
    fn missing_criteria_blocks() {
        let mut i = base();
        i.criteria_count = 0;
        assert!(compute_readiness(&i).reasons.contains(&Reason::NoCriteria));
    }
    #[test]
    fn ancestor_without_discovery_mark_blocks_and_names_it() {
        let mut i = base();
        i.ancestors[1].discovery_complete = false;
        assert_eq!(
            compute_readiness(&i).reasons,
            vec![Reason::AncestorDiscoveryIncomplete { item: key("ABC-1") }]
        );
    }
    #[test]
    fn each_decision_state_yields_its_own_reason() {
        let mut i = base();
        i.own_decisions = vec![
            decision(1, CheckResult::Unchecked, false, false),
            decision(2, CheckResult::ProposedPrOpen, false, false),
            decision(3, CheckResult::NotFound, false, false),
            decision(
                4,
                CheckResult::NotAccepted {
                    status: "superseded".into(),
                    superseded_by: Some("kb/ADR-0016".into()),
                    commit: "abc".into(),
                    reversibility: None,
                },
                false,
                false,
            ),
            decision(
                5,
                CheckResult::Accepted {
                    commit: "abc".into(),
                    reversibility: None,
                },
                false,
                false,
            ),
            decision(6, CheckResult::Unchecked, true, false),
        ];
        let r = compute_readiness(&i);
        assert!(!r.ready);
        assert_eq!(
            r.reasons.len(),
            4,
            "accepted and closed decisions yield no reason"
        );
        assert!(matches!(
            r.reasons[0],
            Reason::DecisionNoRecord { decision_id: 1, .. }
        ));
        assert!(matches!(
            r.reasons[1],
            Reason::DecisionProposedPrOpen { decision_id: 2, .. }
        ));
        assert!(matches!(
            r.reasons[2],
            Reason::DecisionRecordNotFound { decision_id: 3, .. }
        ));
        assert!(
            matches!(&r.reasons[3], Reason::DecisionNotAccepted { decision_id: 4, status, superseded_by: Some(s), .. } if status == "superseded" && s == "kb/ADR-0016")
        );
    }
    #[test]
    fn unchecked_with_record_is_its_own_reason() {
        let mut i = base();
        i.own_decisions = vec![decision(1, CheckResult::Unchecked, false, true)];
        assert!(matches!(
            compute_readiness(&i).reasons[0],
            Reason::DecisionUnchecked { decision_id: 1, .. }
        ));
    }
    #[test]
    fn ancestor_decisions_block_too() {
        let mut i = base();
        i.ancestors[0]
            .decisions
            .push(decision(9, CheckResult::Unchecked, false, false));
        assert!(matches!(
            compute_readiness(&i).reasons[0],
            Reason::DecisionNoRecord { decision_id: 9, .. }
        ));
    }
    fn blocker(k: &str, delivery: DeliveryState, owner: Option<&str>) -> BlockerFact {
        BlockerFact {
            key: key(k),
            delivery,
            owner: owner.map(str::to_string),
        }
    }
    #[test]
    fn only_blockers_unfinished_for_this_owner_are_reasons() {
        let mut i = base(); // owned by dan
        i.blockers = vec![
            blocker("ABC-5", DeliveryState::InReview, Some("dan@example.com")),
            blocker("ABC-6", DeliveryState::InReview, Some("mia@example.com")),
            blocker("ABC-7", DeliveryState::Testing, Some("mia@example.com")),
            blocker("ABC-8", DeliveryState::Todo, None),
        ];
        assert_eq!(
            compute_readiness(&i).reasons,
            vec![
                Reason::BlockedBy {
                    story: key("ABC-6"),
                    delivery: DeliveryState::InReview,
                    owner: Some("mia@example.com".into())
                },
                Reason::BlockedBy {
                    story: key("ABC-8"),
                    delivery: DeliveryState::Todo,
                    owner: None
                },
            ]
        );
        i.owner = None;
        assert_eq!(
            compute_readiness(&i).reasons.len(),
            3,
            "an unowned dependent is blocked by every started blocker; testing still blocks nobody"
        );
    }

    /// Every check result, through an exhaustive match so a new variant fails to compile here until it is listed.
    fn every_check() -> Vec<CheckResult> {
        let all = vec![
            CheckResult::Unchecked,
            CheckResult::Accepted {
                commit: "c".into(),
                reversibility: None,
            },
            CheckResult::NotAccepted {
                status: "proposed".into(),
                superseded_by: None,
                commit: "c".into(),
                reversibility: None,
            },
            CheckResult::ProposedPrOpen,
            CheckResult::NotFound,
        ];
        for c in &all {
            match c {
                CheckResult::Unchecked
                | CheckResult::Accepted { .. }
                | CheckResult::NotAccepted { .. }
                | CheckResult::ProposedPrOpen
                | CheckResult::NotFound => {}
            }
        }
        all
    }

    /// What only an admin may store (the check endpoint's rule) is exactly what readiness lets through for an open
    /// decision with a record: a predicate that also admitted, say, proposed_pr_open, or a readiness change that made
    /// another result satisfying, would let a member satisfy a decision by assertion.
    #[test]
    fn satisfies_is_exactly_what_readiness_lets_through() {
        for check in every_check() {
            let open = decision(1, check.clone(), false, true);
            assert_eq!(
                satisfies(&check),
                decision_reason(&open).is_none(),
                "{check:?}"
            );
        }
    }

    use reeve_protocol::DeliveryState::{self, *};

    /// kb/ADR-0027 R5 as literal rows: (blocker delivery, blocker owner, dependent owner, blocks). Written from the
    /// record's sentence, not from the implementation.
    #[test]
    fn a_blocker_blocks_by_its_delivery_and_both_owners() {
        const D: Option<&str> = Some("dan@example.com");
        const M: Option<&str> = Some("mia@example.com");
        let rows: &[(DeliveryState, Option<&str>, Option<&str>, bool)] = &[
            (Todo, D, D, true),
            (Todo, D, M, true),
            (Todo, None, None, true),
            (InProgress, D, D, false),
            (InProgress, D, M, true),
            (InProgress, D, None, true),
            (InProgress, None, D, true),
            (InProgress, None, None, true),
            (InReview, D, D, false),
            (InReview, D, M, true),
            (InReview, None, None, true),
            (Testing, D, M, false),
            (Testing, None, None, false),
            (Done, D, M, false),
            (Done, None, None, false),
        ];
        for &(delivery, b_owner, d_owner, expected) in rows {
            let fact = BlockerFact {
                key: key("ABC-8"),
                delivery,
                owner: b_owner.map(str::to_string),
            };
            assert_eq!(
                blocks(&fact, d_owner),
                expected,
                "{delivery:?}, blocker {b_owner:?}, dependent {d_owner:?}"
            );
        }
    }
}
