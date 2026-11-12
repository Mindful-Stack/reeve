//! Wording. `short` matches the web client's `SHORT` table (`web/src/lib/board.ts`) word for word, so a card reads the
//! same in both clients; `full` is the CLI's `render::reason`. Every function matches exhaustively, so a new reason
//! variant fails the build here.
use reeve_protocol::{DeliveryState, Kind, Readiness, Reason};

/// The delivery states in board order: H/L move along this array.
pub const STATES: [DeliveryState; 5] = [
    DeliveryState::Todo,
    DeliveryState::InProgress,
    DeliveryState::InReview,
    DeliveryState::Testing,
    DeliveryState::Done,
];

/// Reasons in their short wording, comma-separated: how a refusal reads in the status line.
pub fn shorts(rs: &[Reason]) -> String {
    rs.iter().map(short).collect::<Vec<_>>().join(", ")
}

pub fn short(r: &Reason) -> String {
    match r {
        Reason::Untriaged => "untriaged".into(),
        Reason::NoCriteria => "no criteria".into(),
        Reason::AncestorDiscoveryIncomplete { item } => format!("discovery open: {item}"),
        Reason::DecisionNoRecord { .. } => "decision: no record".into(),
        Reason::DecisionUnchecked { .. } => "decision unchecked".into(),
        Reason::DecisionProposedPrOpen { .. } => "decision: proposed (PR linked)".into(),
        Reason::DecisionRecordNotFound { .. } => "decision: record not found".into(),
        Reason::DecisionRepoNotConfigured { .. } => {
            "decision: record in an unchecked repository".into()
        }
        Reason::DecisionNotAccepted {
            status,
            superseded_by,
            ..
        } => match superseded_by {
            Some(s) => format!("decision: superseded by {s}, re-point"),
            None => format!("decision: {status}"),
        },
        Reason::BlockedBy { story, .. } => format!("blocked by {story}"),
        Reason::ResolutionRequired => "resolution required".into(),
        Reason::CompletedNeedsActiveOrigin => "completed needs active work".into(),
        Reason::OverrideNeedsReason => "override needs a reason".into(),
        Reason::AutomationCannotOverride => "automation cannot override".into(),
        Reason::SprintActive => "sprint active".into(),
        Reason::VerdictMissing { .. } => "verdict missing".into(),
        Reason::SprintNotActive => "sprint not active".into(),
        Reason::SprintNotDraft => "sprint not draft".into(),
        Reason::SprintClosed => "sprint closed".into(),
        Reason::InteractiveOnly => "interactive only".into(),
    }
}

pub fn full(r: &Reason) -> String {
    crate::render::reason(r)
}

pub fn readiness_short(r: &Readiness) -> String {
    match r.reasons.first() {
        Some(first) if !r.ready => short(first),
        _ => "ready".into(),
    }
}

pub fn state(s: DeliveryState) -> &'static str {
    match s {
        DeliveryState::Todo => "todo",
        DeliveryState::InProgress => "in progress",
        DeliveryState::InReview => "in review",
        DeliveryState::Testing => "testing",
        DeliveryState::Done => "done",
    }
}

pub fn kind(k: Kind) -> &'static str {
    match k {
        Kind::Story => "story",
        Kind::Bug => "bug",
        Kind::Chore => "chore",
    }
}

/// The position of a state in `STATES`.
pub fn index(s: DeliveryState) -> usize {
    STATES
        .iter()
        .position(|x| *x == s)
        .expect("every state is in STATES")
}

/// The single copy lives in crate::urgency.
pub(crate) use crate::urgency::local_part;

#[cfg(test)]
mod tests {
    use super::*;
    use reeve_protocol::{Key, Readiness, Reason};

    fn key(s: &str) -> Key {
        s.parse().unwrap()
    }
    fn every_reason() -> Vec<(Reason, &'static str, bool)> {
        let t = || "T".to_string();
        vec![
            (Reason::Untriaged, "untriaged", true),
            (Reason::NoCriteria, "no criteria", true),
            (
                Reason::AncestorDiscoveryIncomplete { item: key("ABC-7") },
                "discovery open: ABC-7",
                true,
            ),
            (
                Reason::DecisionNoRecord {
                    decision_id: 3,
                    title: t(),
                },
                "decision: no record",
                true,
            ),
            (
                Reason::DecisionUnchecked {
                    decision_id: 3,
                    title: t(),
                },
                "decision unchecked",
                true,
            ),
            (
                Reason::DecisionProposedPrOpen {
                    decision_id: 3,
                    title: t(),
                },
                "decision: proposed (PR linked)",
                true,
            ),
            (
                Reason::DecisionRecordNotFound {
                    decision_id: 3,
                    title: t(),
                },
                "decision: record not found",
                true,
            ),
            (
                Reason::DecisionRepoNotConfigured {
                    decision_id: 3,
                    title: t(),
                },
                "decision: record in an unchecked repository",
                true,
            ),
            (
                Reason::DecisionNotAccepted {
                    decision_id: 3,
                    title: t(),
                    status: "rejected".into(),
                    superseded_by: None,
                },
                "decision: rejected",
                true,
            ),
            (
                Reason::DecisionNotAccepted {
                    decision_id: 3,
                    title: t(),
                    status: "superseded".into(),
                    superseded_by: Some("kb/ADR-0004".into()),
                },
                "decision: superseded by kb/ADR-0004, re-point",
                true,
            ),
            (
                Reason::BlockedBy {
                    story: key("ABC-12"),
                    delivery: DeliveryState::Todo,
                    owner: None,
                },
                "blocked by ABC-12",
                true,
            ),
            (Reason::ResolutionRequired, "resolution required", false),
            (
                Reason::CompletedNeedsActiveOrigin,
                "completed needs active work",
                false,
            ),
            (
                Reason::OverrideNeedsReason,
                "override needs a reason",
                false,
            ),
            (
                Reason::AutomationCannotOverride,
                "automation cannot override",
                false,
            ),
            (Reason::SprintActive, "sprint active", false),
            (
                Reason::VerdictMissing { goals: vec![1] },
                "verdict missing",
                false,
            ),
            (Reason::SprintNotActive, "sprint not active", false),
            (Reason::SprintNotDraft, "sprint not draft", false),
            (Reason::SprintClosed, "sprint closed", false),
            (Reason::InteractiveOnly, "interactive only", false),
        ]
    }

    #[test]
    fn short_wording_matches_the_web_client_and_overridable_the_protocol() {
        for (r, want, over) in every_reason() {
            assert_eq!(short(&r), want, "{r:?}");
            assert_eq!(r.overridable(), over, "{r:?}");
        }
    }

    /// The web's SHORT table is the source of the card wording: each fixture's label is rebuilt in the web's source
    /// form (fixture values back to template expressions) and must appear verbatim in the table.
    #[test]
    fn every_short_label_appears_in_the_web_table() {
        let web = include_str!("../../../../web/src/lib/board.ts");
        let table =
            &web[web.find("export const SHORT").unwrap()..web.find("export const FULL").unwrap()];
        let templates = [
            ("ABC-7", "${r.item}"),
            ("ABC-12", "${r.story}"),
            ("kb/ADR-0004", "${r.superseded_by}"),
        ];
        for (r, want, _) in every_reason() {
            let mut src = want.to_string();
            let mut replaced = false;
            for (value, tpl) in templates {
                if src.contains(value) {
                    src = src.replace(value, tpl);
                    replaced = true;
                }
            }
            let mut forms = vec![src];
            if let Reason::DecisionNotAccepted {
                superseded_by: None,
                ..
            } = r
            {
                forms = vec!["`decision: ${r.status}`".to_string()];
            } else if replaced {
                forms = forms.into_iter().map(|s| format!("`{s}`")).collect();
            } else {
                forms = forms.into_iter().map(|s| format!("'{s}'")).collect();
            }
            for f in forms {
                assert!(
                    table.contains(&f),
                    "{f} ({r:?}) is not in the web SHORT table"
                );
            }
        }
    }

    #[test]
    fn readiness_short_is_ready_or_the_first_reason() {
        assert_eq!(
            readiness_short(&Readiness {
                ready: true,
                reasons: vec![]
            }),
            "ready"
        );
        assert_eq!(
            readiness_short(&Readiness {
                ready: false,
                reasons: vec![Reason::NoCriteria, Reason::Untriaged]
            }),
            "no criteria"
        );
    }

    #[test]
    fn state_words_and_order() {
        assert_eq!(
            STATES.map(state),
            ["todo", "in progress", "in review", "testing", "done"]
        );
    }
}
