use reeve_protocol::{CheckResult, Key, Kind, Readiness, Reason};

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
/// Everything readiness needs, loaded by the repo in one query batch.
#[derive(Debug, Clone)]
pub struct ReadinessInput {
    pub kind: Kind,
    pub has_epic: bool,
    pub criteria_count: usize,
    pub ancestors: Vec<AncestorFact>,
    pub own_decisions: Vec<DecisionFact>,
    pub open_blockers: Vec<Key>,
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

/// The one check result that satisfies a required decision (kb/ADR-0003 R3). The check endpoint lets only an admin
/// store it; a test pins it against `decision_reason`, so the two cannot drift.
pub(crate) fn satisfies(check: &CheckResult) -> bool {
    matches!(check, CheckResult::Accepted { .. })
}

/// Reasons are ordered: untriaged; then ancestors in the order given (nearest first,
/// each with its discovery reason then its decisions); then own decisions; then
/// no-criteria; then blockers. Duplicates are not removed, so the repo must supply
/// unique ancestors and blockers.
pub fn compute_readiness(i: &ReadinessInput) -> Readiness {
    let mut reasons = Vec::new();
    if i.kind == Kind::Story && !i.has_epic {
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
        i.open_blockers
            .iter()
            .map(|k| Reason::BlockedBy { story: k.clone() }),
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
            open_blockers: vec![],
        }
    }
    #[test]
    fn happy_path_is_ready() {
        assert!(compute_readiness(&base()).ready);
    }
    #[test]
    fn story_kind_without_epic_is_untriaged() {
        let mut i = base();
        i.has_epic = false;
        i.ancestors.clear();
        assert_eq!(compute_readiness(&i).reasons, vec![Reason::Untriaged]);
        i.kind = Kind::Bug;
        assert!(compute_readiness(&i).ready, "bugs may stand alone");
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
                    superseded_by: Some("kb/ADR-0009".into()),
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
            matches!(&r.reasons[3], Reason::DecisionNotAccepted { decision_id: 4, status, superseded_by: Some(s), .. } if status == "superseded" && s == "kb/ADR-0009")
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
    #[test]
    fn unfinished_blockers_block() {
        let mut i = base();
        i.open_blockers = vec![key("ABC-8")];
        assert_eq!(
            compute_readiness(&i).reasons,
            vec![Reason::BlockedBy {
                story: key("ABC-8")
            }]
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
}
