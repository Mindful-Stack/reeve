use chrono::{DateTime, NaiveDate, Utc};
use reeve_protocol::{CheckResult, ClosureKind, ItemType, Key, Reversibility, TokenKind};

/// Why a decision refuses a change or an input (kb/ADR-0004, kb/ADR-0013 R3). `StaleReference` is a version conflict at
/// the API; every other variant is a validation error carrying its message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecisionError {
    #[error("closing needs a reason")]
    NoReason,
    #[error("already closed")]
    AlreadyClosed,
    #[error(
        "a one-way decision is decided by its accepted record; withdraw it if it is no longer needed"
    )]
    OneWayDecided,
    #[error("decision is closed")]
    Closed,
    #[error("decision has no record reference")]
    NoRecord,
    /// A check result for a reference other than the current one; the handler answers it as a version conflict.
    #[error("version conflict: current is {current}")]
    StaleReference { current: i64 },
    #[error("repo, path and record_id are required")]
    LinkFieldsRequired,
    #[error("title and decider must not be blank")]
    BlankEditField,
    #[error("{0}")]
    InvalidReference(String),
    /// Today's `pr_url` message, verbatim.
    #[error("{0}")]
    InvalidPrUrl(String),
}

/// A reference to a decision record: a repository, a path inside it and the record's id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordRef {
    repo: String,
    path: String,
    record_id: String,
}

impl RecordRef {
    /// Trims the three fields and refuses a blank one. A reference names a repository and a path inside it: relative,
    /// without a `..` component, and not starting with `-`, so a checker never reads outside the repository or hands
    /// git an option. The repository need not be configured yet: the reference is stored, readiness says the record is
    /// in an unchecked repository, and the scheduled check, which reads only configured repositories (kb/ADR-0016 R2),
    /// picks it up once the repository is configured. Its name is one plain directory name, as a
    /// checker uses it. The path is stored in normal form, without empty and `.` components (`./a//b` is `a/b`): git
    /// names tree entries that way, and the checker compares the stored path with that name. A path that is nothing but
    /// such components names no file.
    pub fn new(repo: &str, path: &str, record_id: &str) -> Result<Self, DecisionError> {
        let (repo, path, record_id) = (repo.trim(), path.trim(), record_id.trim());
        if repo.is_empty() || path.is_empty() || record_id.is_empty() {
            return Err(DecisionError::LinkFieldsRequired);
        }
        let invalid = |m: String| Err(DecisionError::InvalidReference(m));
        if repo.starts_with(['-', '.'])
            || repo.contains(['/', '\\'])
            || repo.chars().any(char::is_whitespace)
        {
            return invalid(format!("repo `{repo}` is not a single repository name"));
        }
        if path.starts_with('/') || path.starts_with('\\') {
            return invalid(format!("path `{path}` must be relative to the repository"));
        }
        if path.split(['/', '\\']).any(|c| c == "..") {
            return invalid(format!("path `{path}` must not contain a `..` component"));
        }
        let normal = path
            .split('/')
            .filter(|c| !c.is_empty() && *c != ".")
            .collect::<Vec<_>>()
            .join("/");
        if normal.is_empty() {
            return invalid(format!("path `{path}` names no file"));
        }
        // Checked on the normal form, which is what git is handed: `./-x.md` is `-x.md`.
        if normal.starts_with('-') {
            return invalid(format!("path `{path}` must not start with `-`"));
        }
        Ok(Self::from_persisted(repo, normal, record_id))
    }
    /// Rebuilds a stored reference without validation (for the adapter and tests).
    pub fn from_persisted(
        repo: impl Into<String>,
        path: impl Into<String>,
        record_id: impl Into<String>,
    ) -> Self {
        Self {
            repo: repo.into(),
            path: path.into(),
            record_id: record_id.into(),
        }
    }
    pub fn repo(&self) -> &str {
        &self.repo
    }
    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn record_id(&self) -> &str {
        &self.record_id
    }
}

/// A decision's pull-request url: an absolute `http(s)://` URL with a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrUrl(String);

impl PrUrl {
    /// The url is rendered as a link, so it must be an absolute `http(s)://` URL with a host: a `javascript:` or
    /// `data:` url stored here would run in the web client when clicked. Keeps the trimmed url.
    pub fn parse(raw: &str) -> Result<Self, DecisionError> {
        let t = raw.trim();
        let ok = url::Url::parse(t)
            .map(|u| {
                matches!(u.scheme(), "http" | "https")
                    && u.host_str().is_some_and(|h| !h.is_empty())
            })
            .unwrap_or(false);
        if !ok {
            return Err(DecisionError::InvalidPrUrl(format!(
                "pr_url `{t}` must be an absolute http(s) URL"
            )));
        }
        Ok(Self(t.to_string()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The planner-side fields an edit sets: `None` is "not sent", `Some(None)` clears due or reversibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionEdit {
    title: Option<String>,
    decider: Option<String>,
    due: Option<Option<NaiveDate>>,
    reversibility: Option<Option<Reversibility>>,
}

impl DecisionEdit {
    pub fn new(
        title: Option<String>,
        decider: Option<String>,
        due: Option<Option<NaiveDate>>,
        reversibility: Option<Option<Reversibility>>,
    ) -> Result<Self, DecisionError> {
        let title = title.map(|t| t.trim().to_string());
        let decider = decider.map(|d| d.trim().to_string());
        if title.as_deref() == Some("") || decider.as_deref() == Some("") {
            return Err(DecisionError::BlankEditField);
        }
        Ok(Self {
            title,
            decider,
            due,
            reversibility,
        })
    }
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
    id: i64,
    on: Key,
    /// The level of the item it is on (the table's `on_type`).
    on_type: ItemType,
    title: String,
    decider: String,
    due: Option<NaiveDate>,
    /// Unset behaves as two-way (kb/ADR-0013 R1).
    reversibility: Option<Reversibility>,
    record: Option<RecordRef>,
    pr_url: Option<String>,
    check: CheckResult,
    checked_at: Option<DateTime<Utc>>,
    closed: Option<Closed>,
    created_at: DateTime<Utc>,
    version: i64,
}

/// One planner-side field an edit changed, with its old and new value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldChange {
    Title {
        from: String,
        to: String,
    },
    Decider {
        from: String,
        to: String,
    },
    Due {
        from: Option<NaiveDate>,
        to: Option<NaiveDate>,
    },
    Reversibility {
        from: Option<Reversibility>,
        to: Option<Reversibility>,
    },
}

impl FieldChange {
    /// The field's name as the API and the activity log spell it.
    pub fn name(&self) -> &'static str {
        match self {
            FieldChange::Title { .. } => "title",
            FieldChange::Decider { .. } => "decider",
            FieldChange::Due { .. } => "due",
            FieldChange::Reversibility { .. } => "reversibility",
        }
    }
}

/// What a check result did to the decision: nothing (it says the same as the stored one), or a new check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckOutcome {
    Unchanged,
    Changed,
}

impl RequiredDecision {
    /// Rebuilds a stored decision without validation (for the adapter and tests).
    #[allow(clippy::too_many_arguments)]
    pub fn from_persisted(
        id: i64,
        on: Key,
        on_type: ItemType,
        title: String,
        decider: String,
        due: Option<NaiveDate>,
        reversibility: Option<Reversibility>,
        record: Option<RecordRef>,
        pr_url: Option<String>,
        check: CheckResult,
        checked_at: Option<DateTime<Utc>>,
        closed: Option<Closed>,
        created_at: DateTime<Utc>,
        version: i64,
    ) -> Self {
        Self {
            id,
            on,
            on_type,
            title,
            decider,
            due,
            reversibility,
            record,
            pr_url,
            check,
            checked_at,
            closed,
            created_at,
            version,
        }
    }
    pub fn id(&self) -> i64 {
        self.id
    }
    pub fn on(&self) -> &Key {
        &self.on
    }
    /// The level of the item it is on (the table's `on_type`).
    pub fn on_type(&self) -> ItemType {
        self.on_type
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn decider(&self) -> &str {
        &self.decider
    }
    pub fn due(&self) -> Option<NaiveDate> {
        self.due
    }
    /// Unset behaves as two-way (kb/ADR-0013 R1).
    pub fn reversibility(&self) -> Option<Reversibility> {
        self.reversibility
    }
    pub fn record(&self) -> Option<&RecordRef> {
        self.record.as_ref()
    }
    pub fn pr_url(&self) -> Option<&str> {
        self.pr_url.as_deref()
    }
    pub fn check(&self) -> &CheckResult {
        &self.check
    }
    pub fn checked_at(&self) -> Option<DateTime<Utc>> {
        self.checked_at
    }
    pub fn closed(&self) -> Option<&Closed> {
        self.closed.as_ref()
    }
    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }
    pub fn version(&self) -> i64 {
        self.version
    }
    pub fn is_open(&self) -> bool {
        self.closed.is_none()
    }
    pub fn is_satisfied(&self) -> bool {
        super::readiness::satisfies(&self.check)
    }
    /// Links the decision to a record (kb/ADR-0004 R3): setting or changing the reference resets the check to
    /// unchecked. Replaces the PR url outright so a re-point never carries a stale link. Refuses a closed decision and
    /// then leaves it untouched.
    pub fn link(&mut self, r: RecordRef, pr_url: Option<PrUrl>) -> Result<(), DecisionError> {
        if self.closed.is_some() {
            return Err(DecisionError::Closed);
        }
        self.record = Some(r);
        self.pr_url = pr_url.map(|p| p.0);
        self.check = CheckResult::Unchecked;
        self.checked_at = None;
        Ok(())
    }

    /// Applies the planner-side fields an edit changes (kb/ADR-0004 R2, kb/ADR-0013 R1) and returns them in the order
    /// title, decider, due, reversibility; empty when nothing changed. Never touches the reference, PR url or check
    /// (kb/ADR-0004 R1). Refuses a closed decision and then leaves it untouched.
    pub fn edit(&mut self, e: DecisionEdit) -> Result<Vec<FieldChange>, DecisionError> {
        if self.closed.is_some() {
            return Err(DecisionError::Closed);
        }
        let mut changes = Vec::new();
        if let Some(to) = e.title
            && to != self.title
        {
            let from = std::mem::replace(&mut self.title, to.clone());
            changes.push(FieldChange::Title { from, to });
        }
        if let Some(to) = e.decider
            && to != self.decider
        {
            let from = std::mem::replace(&mut self.decider, to.clone());
            changes.push(FieldChange::Decider { from, to });
        }
        if let Some(to) = e.due
            && to != self.due
        {
            let from = std::mem::replace(&mut self.due, to);
            changes.push(FieldChange::Due { from, to });
        }
        if let Some(to) = e.reversibility
            && to != self.reversibility
        {
            let from = std::mem::replace(&mut self.reversibility, to);
            changes.push(FieldChange::Reversibility { from, to });
        }
        Ok(changes)
    }

    /// Stores a check result posted for `for_record`. Refuses, in this order, a decision with no reference, a result
    /// for a reference other than the current one (a version conflict at the API), and a closed decision; each leaves
    /// it untouched. A result that says the same as the stored one (`same_outcome`, which ignores `commit`) changes
    /// nothing and is `Unchanged`; otherwise the check and `checked_at` are set.
    pub fn record_check(
        &mut self,
        for_record: &str,
        check: CheckResult,
        at: DateTime<Utc>,
    ) -> Result<CheckOutcome, DecisionError> {
        match &self.record {
            None => return Err(DecisionError::NoRecord),
            Some(r) if r.record_id != for_record => {
                return Err(DecisionError::StaleReference {
                    current: self.version,
                });
            }
            Some(_) => {}
        }
        if self.closed.is_some() {
            return Err(DecisionError::Closed);
        }
        if same_outcome(&self.check, &check) {
            return Ok(CheckOutcome::Unchanged);
        }
        self.check = check;
        self.checked_at = Some(at);
        Ok(CheckOutcome::Changed)
    }
}

/// An open, two-way-unset decision with no record, unchecked, at version 3: "Pick", decided by dan@example.com.
#[cfg(test)]
pub(crate) fn test_decision() -> RequiredDecision {
    use chrono::TimeZone;
    RequiredDecision::from_persisted(
        1,
        "ABC-1".parse().unwrap(),
        ItemType::Initiative,
        "Pick".into(),
        "dan@example.com".into(),
        None,
        None,
        None,
        None,
        CheckResult::Unchecked,
        None,
        None,
        Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).unwrap(),
        3,
    )
}

#[cfg(test)]
impl RequiredDecision {
    pub(crate) fn with_record_ref(mut self, r: RecordRef) -> Self {
        self.record = Some(r);
        self
    }
    pub(crate) fn closed_now(mut self) -> Self {
        use chrono::TimeZone;
        self.closed = Some(Closed {
            at: Utc.with_ymd_and_hms(2026, 11, 2, 9, 0, 0).unwrap(),
            by: "dan@example.com".into(),
            reason: "moot".into(),
            kind: Some(ClosureKind::Withdrawn),
            via: Some(TokenKind::Interactive),
        });
        self
    }
    pub(crate) fn set_check_for_test(&mut self, check: CheckResult, at: Option<DateTime<Utc>>) {
        self.check = check;
        self.checked_at = at;
    }
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
    ) -> Result<(), DecisionError> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(DecisionError::NoReason);
        }
        if self.closed.is_some() {
            return Err(DecisionError::AlreadyClosed);
        }
        if kind == ClosureKind::Decided && self.reversibility == Some(Reversibility::OneWay) {
            return Err(DecisionError::OneWayDecided);
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

    #[test]
    fn relinking_resets_the_check() {
        let mut d = test_decision();
        d.set_check_for_test(
            CheckResult::Accepted {
                commit: "x".into(),
                reversibility: None,
            },
            Some(t(1)),
        );
        d.link(
            RecordRef::from_persisted("kb", "knowledge/adrs/0031-x.md", "kb/ADR-0031"),
            None,
        )
        .unwrap();
        assert!(d.pr_url().is_none());
        assert_eq!(d.check(), &CheckResult::Unchecked);
        assert!(d.checked_at().is_none());
        assert!(!d.is_satisfied());
    }

    /// R3 on the aggregate: one-way cannot be decided but can be withdrawn; two-way and unset can be decided. The
    /// existing rules keep their order: a blank reason first, then already closed. Catches a rule that checks the wrong
    /// kind, treats unset as one-way, or mutates the decision before refusing.
    #[test]
    fn close_follows_reversibility_and_the_existing_rules() {
        use ClosureKind::*;
        let at = Utc.with_ymd_and_hms(2026, 11, 3, 9, 0, 0).unwrap();
        let close = |r: Option<Reversibility>, kind, reason: &str| {
            let mut d = test_decision();
            d.reversibility = r;
            let out = d.close(kind, reason, "dan@example.com", TokenKind::Automation, at);
            (out, d.closed)
        };
        let (out, closed) = close(Some(Reversibility::OneWay), Decided, "flat fee");
        assert_eq!((out, closed), (Err(DecisionError::OneWayDecided), None));
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
            Err(DecisionError::NoReason)
        );

        // Already closed is checked before the one-way rule, and a refusal keeps the first close.
        let mut d = test_decision();
        d.reversibility = Some(Reversibility::OneWay);
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
            Err(DecisionError::AlreadyClosed)
        );
        assert_eq!(d.closed.as_ref().unwrap().reason, "first");
    }

    /// The accepted and refused references, with the exact messages link_record answers.
    #[test]
    fn a_record_reference_is_validated_with_todays_messages() {
        let ok = RecordRef::new(" kb ", " ./adrs//0001.md ", " kb/ADR-0001 ").unwrap();
        assert_eq!(
            (ok.repo(), ok.path(), ok.record_id()),
            ("kb", "adrs/0001.md", "kb/ADR-0001")
        );
        for (repo, path, rid, want) in [
            ("", "a.md", "r", "repo, path and record_id are required"),
            ("kb", " ", "r", "repo, path and record_id are required"),
            ("kb", "a.md", "", "repo, path and record_id are required"),
            (
                "a/b",
                "a.md",
                "r",
                "repo `a/b` is not a single repository name",
            ),
            (
                ".kb",
                "a.md",
                "r",
                "repo `.kb` is not a single repository name",
            ),
            (
                "-kb",
                "a.md",
                "r",
                "repo `-kb` is not a single repository name",
            ),
            (
                "k b",
                "a.md",
                "r",
                "repo `k b` is not a single repository name",
            ),
            (
                "kb",
                "/a.md",
                "r",
                "path `/a.md` must be relative to the repository",
            ),
            (
                "kb",
                "\\a.md",
                "r",
                "path `\\a.md` must be relative to the repository",
            ),
            (
                "kb",
                "x/../a.md",
                "r",
                "path `x/../a.md` must not contain a `..` component",
            ),
            ("kb", "./.", "r", "path `./.` names no file"),
            (
                "kb",
                "./-a.md",
                "r",
                "path `./-a.md` must not start with `-`",
            ),
        ] {
            assert_eq!(
                RecordRef::new(repo, path, rid).unwrap_err().to_string(),
                want,
                "{repo} {path} {rid}"
            );
        }
    }

    #[test]
    fn a_pr_url_must_be_absolute_http() {
        assert_eq!(
            PrUrl::parse(" https://github.com/o/r/pull/1 ")
                .unwrap()
                .as_str(),
            "https://github.com/o/r/pull/1"
        );
        for bad in [
            "javascript:alert(1)",
            "data:x",
            "/rel",
            "https://",
            "ftp://h/x",
        ] {
            assert_eq!(
                PrUrl::parse(bad).unwrap_err(),
                DecisionError::InvalidPrUrl(format!(
                    "pr_url `{}` must be an absolute http(s) URL",
                    bad.trim()
                ))
            );
            assert_eq!(
                PrUrl::parse(bad).unwrap_err().to_string(),
                format!("pr_url `{}` must be an absolute http(s) URL", bad.trim())
            );
        }
    }

    #[test]
    fn an_edit_refuses_a_blank_title_or_decider() {
        for (t, d) in [(Some(" "), None), (None, Some(""))] {
            assert_eq!(
                DecisionEdit::new(t.map(Into::into), d.map(Into::into), None, None)
                    .unwrap_err()
                    .to_string(),
                "title and decider must not be blank"
            );
        }
        assert!(DecisionEdit::new(Some(" T ".into()), None, Some(None), None).is_ok());
    }

    #[test]
    fn decision_errors_keep_todays_messages() {
        assert_eq!(DecisionError::Closed.to_string(), "decision is closed");
        assert_eq!(
            DecisionError::NoRecord.to_string(),
            "decision has no record reference"
        );
        assert_eq!(
            DecisionError::NoReason.to_string(),
            "closing needs a reason"
        );
        assert_eq!(DecisionError::AlreadyClosed.to_string(), "already closed");
        assert_eq!(
            DecisionError::OneWayDecided.to_string(),
            "a one-way decision is decided by its accepted record; withdraw it if it is no longer needed"
        );
        assert_eq!(
            DecisionError::StaleReference { current: 3 }.to_string(),
            "version conflict: current is 3"
        );
    }

    fn t(n: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, n, 9, 0, 0).unwrap()
    }
    fn day(n: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, n).unwrap()
    }
    fn closed_decision() -> RequiredDecision {
        test_decision().closed_now()
    }
    fn closed_linked_decision() -> RequiredDecision {
        test_decision()
            .with_record_ref(RecordRef::from_persisted("kb", "a.md", "kb/ADR-1"))
            .closed_now()
    }

    #[test]
    fn link_resets_the_check_and_refuses_a_closed_decision() {
        let mut d = test_decision();
        d.set_check_for_test(CheckResult::NotFound, Some(t(1)));
        let r = RecordRef::new("kb", "a.md", "kb/ADR-1").unwrap();
        d.link(r.clone(), Some(PrUrl::parse("https://x/p/1").unwrap()))
            .unwrap();
        assert_eq!(
            (d.record(), d.pr_url(), d.check(), d.checked_at()),
            (
                Some(&r),
                Some("https://x/p/1"),
                &CheckResult::Unchecked,
                None
            )
        );
        let mut c = closed_decision();
        let before = c.clone();
        assert_eq!(c.link(r, None).unwrap_err(), DecisionError::Closed);
        assert_eq!(c, before);
    }

    #[test]
    fn edit_returns_typed_changes_in_field_order_and_nothing_for_no_change() {
        let mut d = test_decision(); // title "Pick", decider "dan@example.com", due None, reversibility None
        let e = DecisionEdit::new(
            Some("Pick".into()),
            Some("mia@example.com".into()),
            Some(Some(day(5))),
            Some(Some(Reversibility::OneWay)),
        )
        .unwrap();
        assert_eq!(
            d.edit(e).unwrap(),
            vec![
                FieldChange::Decider {
                    from: "dan@example.com".into(),
                    to: "mia@example.com".into()
                },
                FieldChange::Due {
                    from: None,
                    to: Some(day(5))
                },
                FieldChange::Reversibility {
                    from: None,
                    to: Some(Reversibility::OneWay)
                },
            ]
        );
        assert_eq!(
            (d.title(), d.decider(), d.due(), d.reversibility()),
            (
                "Pick",
                "mia@example.com",
                Some(day(5)),
                Some(Reversibility::OneWay)
            )
        );
        let retitle = DecisionEdit::new(Some(" Choose ".into()), None, None, None).unwrap();
        assert_eq!(
            d.edit(retitle).unwrap(),
            vec![FieldChange::Title {
                from: "Pick".into(),
                to: "Choose".into()
            }]
        );
        let same =
            DecisionEdit::new(Some("Choose".into()), None, Some(Some(day(5))), None).unwrap();
        let before = d.clone();
        assert_eq!(d.edit(same).unwrap(), vec![]);
        assert_eq!(d, before);
        let mut c = closed_decision();
        let before = c.clone();
        assert_eq!(
            c.edit(DecisionEdit::new(Some("X".into()), None, None, None).unwrap())
                .unwrap_err(),
            DecisionError::Closed
        );
        assert_eq!(c, before);
        assert_eq!(
            [
                FieldChange::Title {
                    from: "".into(),
                    to: "".into()
                },
                FieldChange::Decider {
                    from: "".into(),
                    to: "".into()
                },
                FieldChange::Due {
                    from: None,
                    to: None
                },
                FieldChange::Reversibility {
                    from: None,
                    to: None
                },
            ]
            .map(|c| c.name()),
            ["title", "decider", "due", "reversibility"]
        );
    }

    #[test]
    fn record_check_refuses_in_order_and_ignores_a_new_commit_alone() {
        let mut d = test_decision();
        let before = d.clone();
        assert_eq!(
            d.record_check("kb/ADR-1", CheckResult::NotFound, t(2))
                .unwrap_err(),
            DecisionError::NoRecord
        );
        assert_eq!(d, before);
        d.link(RecordRef::new("kb", "a.md", "kb/ADR-1").unwrap(), None)
            .unwrap();
        let before = d.clone();
        assert_eq!(
            d.record_check("kb/ADR-9", CheckResult::NotFound, t(2))
                .unwrap_err(),
            DecisionError::StaleReference {
                current: d.version()
            }
        );
        assert_eq!(d, before);
        let acc = |c: &str| CheckResult::Accepted {
            commit: c.into(),
            reversibility: None,
        };
        assert_eq!(
            d.record_check("kb/ADR-1", acc("a"), t(2)).unwrap(),
            CheckOutcome::Changed
        );
        assert_eq!((d.check(), d.checked_at()), (&acc("a"), Some(t(2))));
        assert_eq!(
            d.record_check("kb/ADR-1", acc("b"), t(3)).unwrap(),
            CheckOutcome::Unchanged
        );
        assert_eq!((d.check(), d.checked_at()), (&acc("a"), Some(t(2))));

        // Today's order: no record, then a stale reference, then closed. A closed decision without a record says
        // NoRecord, and a closed one linked elsewhere says StaleReference; only a closed, matching one says Closed.
        let mut c = closed_linked_decision();
        let before = c.clone();
        assert_eq!(
            c.record_check("kb/ADR-1", acc("a"), t(2)).unwrap_err(),
            DecisionError::Closed
        );
        assert_eq!(c, before);
        let mut u = closed_decision();
        let before = u.clone();
        assert_eq!(
            u.record_check("kb/ADR-1", acc("a"), t(2)).unwrap_err(),
            DecisionError::NoRecord
        );
        assert_eq!(u, before);
        let mut m = closed_linked_decision();
        let before = m.clone();
        assert_eq!(
            m.record_check("kb/ADR-9", acc("a"), t(2)).unwrap_err(),
            DecisionError::StaleReference { current: 3 }
        );
        assert_eq!(m, before);
    }
}
