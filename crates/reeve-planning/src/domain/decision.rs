use chrono::{DateTime, NaiveDate, Utc};
use reeve_protocol::{CheckResult, ClosureKind, ItemType, Key, Reversibility, TokenKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordRef {
    pub repo: String,
    pub path: String,
    pub record_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Closed {
    pub at: DateTime<Utc>,
    pub by: String,
    pub reason: String,
    /// What the close said; `None` on closes made before kb/ADR-0013.
    pub kind: Option<ClosureKind>,
    /// The kind of token that closed it; `None` on closes made before kb/ADR-0013.
    pub via: Option<TokenKind>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequiredDecision {
    pub id: i64,
    pub on: Key,
    /// The level of the item it is on (the table's `on_type`).
    pub on_type: ItemType,
    pub title: String,
    pub decider: String,
    pub due: Option<NaiveDate>,
    /// Unset behaves as two-way (kb/ADR-0013 R1).
    pub reversibility: Option<Reversibility>,
    pub record: Option<RecordRef>,
    pub pr_url: Option<String>,
    pub check: CheckResult,
    pub checked_at: Option<DateTime<Utc>>,
    pub closed: Option<Closed>,
    pub created_at: DateTime<Utc>,
    pub version: i64,
}

impl RequiredDecision {
    pub fn is_open(&self) -> bool {
        self.closed.is_none()
    }
    pub fn is_satisfied(&self) -> bool {
        super::readiness::satisfies(&self.check)
    }
    /// Setting or changing the reference resets the check (kb/ADR-0004 R3).
    /// Replaces the PR url outright so a re-point never carries a stale link.
    pub fn with_record(mut self, r: RecordRef, pr_url: Option<String>) -> Self {
        self.record = Some(r);
        self.pr_url = pr_url;
        self.check = CheckResult::Unchecked;
        self.checked_at = None;
        self
    }
}

/// Why a close is refused (kb/ADR-0004 R2, kb/ADR-0013 R3). Each is a validation error at the API.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CloseError {
    #[error("closing needs a reason")]
    NoReason,
    #[error("already closed")]
    AlreadyClosed,
    #[error(
        "a one-way decision is decided by its accepted record; withdraw it if it is no longer needed"
    )]
    OneWayDecided,
}

impl RequiredDecision {
    /// Closes the decision as decided or withdrawn, with a trimmed reason (kb/ADR-0013 R3). Unset reversibility
    /// behaves as two-way (R1). Leaves the decision untouched when it refuses.
    pub fn close(
        &mut self,
        kind: ClosureKind,
        reason: &str,
        by: &str,
        via: TokenKind,
        at: DateTime<Utc>,
    ) -> Result<(), CloseError> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(CloseError::NoReason);
        }
        if self.closed.is_some() {
            return Err(CloseError::AlreadyClosed);
        }
        if kind == ClosureKind::Decided && self.reversibility == Some(Reversibility::OneWay) {
            return Err(CloseError::OneWayDecided);
        }
        self.closed = Some(Closed {
            at,
            by: by.into(),
            reason: reason.into(),
            kind: Some(kind),
            via: Some(via),
        });
        Ok(())
    }
}

/// Whether two check results say the same thing about the record: the same kind and, for accepted, the same
/// reversibility the record reports; for not accepted, the same `status`, `superseded_by` and reversibility (kb/ADR-0013
/// R2: a change in the reported value alone is a new outcome). `commit` is left out, because it is the default branch's
/// head and moves on every push to the repository. A result equal to the stored one only moves `checked_at` (no
/// activity entry, no version bump).
pub(crate) fn same_outcome(a: &CheckResult, b: &CheckResult) -> bool {
    use CheckResult::*;
    match (a, b) {
        (Unchecked, Unchecked) | (ProposedPrOpen, ProposedPrOpen) | (NotFound, NotFound) => true,
        (
            Accepted {
                reversibility: r1, ..
            },
            Accepted {
                reversibility: r2, ..
            },
        ) => r1 == r2,
        (
            NotAccepted {
                status: s1,
                superseded_by: b1,
                reversibility: r1,
                ..
            },
            NotAccepted {
                status: s2,
                superseded_by: b2,
                reversibility: r2,
                ..
            },
        ) => s1 == s2 && b1 == b2 && r1 == r2,
        (Unchecked | Accepted { .. } | NotAccepted { .. } | ProposedPrOpen | NotFound, _) => false,
    }
}

impl From<&RequiredDecision> for reeve_protocol::RequiredDecisionDto {
    fn from(d: &RequiredDecision) -> Self {
        Self {
            id: d.id,
            on: d.on.clone(),
            on_type: d.on_type,
            title: d.title.clone(),
            decider: d.decider.clone(),
            due: d.due,
            reversibility: d.reversibility,
            record: d.record.as_ref().map(|r| reeve_protocol::RecordRefDto {
                repo: r.repo.clone(),
                path: r.path.clone(),
                record_id: r.record_id.clone(),
            }),
            pr_url: d.pr_url.clone(),
            check: d.check.clone(),
            checked_at: d.checked_at,
            closed: d.closed.as_ref().map(|c| reeve_protocol::ClosedDto {
                at: c.at,
                by: c.by.clone(),
                reason: c.reason.clone(),
                kind: c.kind,
                via: c.via,
            }),
            created_at: d.created_at,
            version: d.version,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// R2: a change in the record's reversibility alone is a new outcome, for accepted and not-accepted alike; a new
    /// commit alone is still not. Catches the old comparison that treats all accepted results as equal.
    #[test]
    fn same_outcome_sees_a_reversibility_change() {
        let acc = |rv| CheckResult::Accepted {
            commit: "a".into(),
            reversibility: rv,
        };
        let na = |rv| CheckResult::NotAccepted {
            status: "proposed".into(),
            superseded_by: None,
            commit: "a".into(),
            reversibility: rv,
        };
        assert!(!same_outcome(&acc(None), &acc(Some(Reversibility::OneWay))));
        assert!(!same_outcome(
            &na(Some(Reversibility::TwoWay)),
            &na(Some(Reversibility::OneWay))
        ));
        assert!(same_outcome(
            &acc(Some(Reversibility::OneWay)),
            &CheckResult::Accepted {
                commit: "b".into(),
                reversibility: Some(Reversibility::OneWay)
            }
        ));
    }

    fn decision() -> RequiredDecision {
        RequiredDecision {
            id: 1,
            on: "ABC-1".parse().unwrap(),
            on_type: ItemType::Initiative,
            title: "t".into(),
            decider: "a@b".into(),
            due: None,
            reversibility: None,
            record: None,
            pr_url: None,
            check: CheckResult::Accepted {
                commit: "x".into(),
                reversibility: None,
            },
            checked_at: Some(Utc::now()),
            closed: None,
            created_at: Utc::now(),
            version: 1,
        }
    }

    #[test]
    fn relinking_resets_the_check() {
        let d = decision();
        let d2 = d.with_record(
            RecordRef {
                repo: "kb".into(),
                path: "knowledge/adrs/0031-x.md".into(),
                record_id: "kb/ADR-0031".into(),
            },
            None,
        );
        assert!(d2.pr_url.is_none());
        assert_eq!(d2.check, CheckResult::Unchecked);
        assert!(d2.checked_at.is_none());
        assert!(!d2.is_satisfied());
    }

    /// R3 on the aggregate: one-way cannot be decided but can be withdrawn; two-way and unset can be decided. The
    /// existing rules keep their order: a blank reason first, then already closed. Catches a rule that checks the wrong
    /// kind, treats unset as one-way, or mutates the decision before refusing.
    #[test]
    fn close_follows_reversibility_and_the_existing_rules() {
        use ClosureKind::*;
        let at = Utc.with_ymd_and_hms(2026, 11, 3, 9, 0, 0).unwrap();
        let close = |r: Option<Reversibility>, kind, reason: &str| {
            let mut d = RequiredDecision {
                reversibility: r,
                ..decision()
            };
            let out = d.close(kind, reason, "dan@example.com", TokenKind::Automation, at);
            (out, d.closed)
        };
        let (out, closed) = close(Some(Reversibility::OneWay), Decided, "flat fee");
        assert_eq!((out, closed), (Err(CloseError::OneWayDecided), None));
        let (out, closed) = close(Some(Reversibility::OneWay), Withdrawn, "  dropped  ");
        assert_eq!(out, Ok(()));
        assert_eq!(
            closed,
            Some(Closed {
                at,
                by: "dan@example.com".into(),
                reason: "dropped".into(),
                kind: Some(Withdrawn),
                via: Some(TokenKind::Automation)
            })
        );
        assert_eq!(
            close(Some(Reversibility::TwoWay), Decided, "Postgres").0,
            Ok(())
        );
        assert_eq!(close(None, Decided, "Postgres").0, Ok(()));
        assert_eq!(
            close(Some(Reversibility::OneWay), Decided, "  ").0,
            Err(CloseError::NoReason)
        );

        // Already closed is checked before the one-way rule, and a refusal keeps the first close.
        let mut d = RequiredDecision {
            reversibility: Some(Reversibility::OneWay),
            ..decision()
        };
        d.close(
            Withdrawn,
            "first",
            "dan@example.com",
            TokenKind::Interactive,
            at,
        )
        .unwrap();
        assert_eq!(
            d.close(
                Decided,
                "second",
                "dan@example.com",
                TokenKind::Interactive,
                at
            ),
            Err(CloseError::AlreadyClosed)
        );
        assert_eq!(d.closed.as_ref().unwrap().reason, "first");
    }
}
