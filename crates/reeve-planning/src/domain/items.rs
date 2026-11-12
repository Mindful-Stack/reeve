use chrono::{DateTime, Utc};
use reeve_protocol::{DeliveryState, Key, Kind, Resolution, TokenKind};

#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveryMark {
    pub at: DateTime<Utc>,
    pub by: String,
}

/// Private fields; constructed only by `new` (validated) or `from_persisted` (trusted rows).
#[derive(Debug, Clone, PartialEq)]
pub struct Initiative {
    key: Key,
    title: String,
    brief: String,
    horizon: String,
    target_month: Option<String>,
    owner: Option<String>,
    discovery: Option<DiscoveryMark>,
    external_ref: Option<String>,
    version: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Epic {
    key: Key,
    title: String,
    initiative: Option<Key>,
    brief: String,
    discovery: Option<DiscoveryMark>,
    external_ref: Option<String>,
    version: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Criterion {
    pub text: String,
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Story {
    key: Key,
    title: String,
    kind: Kind,
    epic: Option<Key>,
    description: String,
    criteria: Vec<Criterion>,
    owner: Option<String>,
    labels: Vec<String>,
    rank: i32,
    delivery: DeliveryState,
    resolution: Option<Resolution>,
    external_ref: Option<String>,
    version: i64,
}

/// A requested delivery move: the target state, the resolution when entering done, and an override's written reason.
#[derive(Debug, Clone)]
pub struct Move {
    pub to: DeliveryState,
    pub resolution: Option<Resolution>,
    pub override_reason: Option<String>,
}

/// What a move did: nothing (same state, same resolution), or a move from `from`, overriding the gate when
/// `override_reason` is set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Moved {
    Unchanged,
    Moved {
        from: DeliveryState,
        override_reason: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MoveRefused {
    #[error("story is not ready")]
    NotReady,
    #[error("automation tokens cannot override the gate")]
    AutomationCannotOverride,
    #[error("an override needs a written reason")]
    OverrideNeedsReason,
    #[error("entering done requires a resolution")]
    ResolutionRequired,
    #[error("completed is only allowed from an active state")]
    CompletedNeedsActiveOrigin,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ItemError {
    #[error("title must be 1..=200 characters")]
    Title,
    #[error("horizon {0:?} is not one of the configured buckets")]
    Horizon(String),
    #[error("a criterion must not be blank")]
    BlankCriterion,
}

pub(crate) fn check_title(t: &str) -> Result<(), ItemError> {
    let n = t.trim().chars().count();
    if (1..=200).contains(&n) {
        Ok(())
    } else {
        Err(ItemError::Title)
    }
}

/// A horizon the initiative already has is always valid (its bucket may have been unlisted since); a different one
/// must be a configured bucket.
pub fn check_horizon_change(
    current: &str,
    next: &str,
    buckets: &[String],
) -> Result<(), ItemError> {
    if next == current || buckets.iter().any(|b| b == next) {
        Ok(())
    } else {
        Err(ItemError::Horizon(next.into()))
    }
}

impl Initiative {
    pub fn new(
        key: Key,
        title: &str,
        brief: &str,
        horizon: &str,
        buckets: &[String],
        target_month: Option<String>,
        owner: Option<String>,
    ) -> Result<Self, ItemError> {
        check_title(title)?;
        if !buckets.iter().any(|b| b == horizon) {
            return Err(ItemError::Horizon(horizon.into()));
        }
        Ok(Self {
            key,
            title: title.trim().into(),
            brief: brief.into(),
            horizon: horizon.into(),
            target_month,
            owner,
            discovery: None,
            external_ref: None,
            version: 1,
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub fn from_persisted(
        key: Key,
        title: String,
        brief: String,
        horizon: String,
        target_month: Option<String>,
        owner: Option<String>,
        discovery: Option<DiscoveryMark>,
        external_ref: Option<String>,
        version: i64,
    ) -> Self {
        Self {
            key,
            title,
            brief,
            horizon,
            target_month,
            owner,
            discovery,
            external_ref,
            version,
        }
    }
    pub fn key(&self) -> &Key {
        &self.key
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn brief(&self) -> &str {
        &self.brief
    }
    pub fn horizon(&self) -> &str {
        &self.horizon
    }
    pub fn target_month(&self) -> Option<&str> {
        self.target_month.as_deref()
    }
    pub fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }
    pub fn discovery(&self) -> Option<&DiscoveryMark> {
        self.discovery.as_ref()
    }
    /// The origin a hand-created item records (a Jira key it was migrated from); validated by the handler.
    pub fn with_external_ref(mut self, external_ref: Option<String>) -> Self {
        self.external_ref = external_ref;
        self
    }
    pub fn external_ref(&self) -> Option<&str> {
        self.external_ref.as_deref()
    }
    pub fn version(&self) -> i64 {
        self.version
    }
}

impl Epic {
    pub fn new(
        key: Key,
        title: &str,
        initiative: Option<Key>,
        brief: &str,
    ) -> Result<Self, ItemError> {
        check_title(title)?;
        Ok(Self {
            key,
            title: title.trim().into(),
            initiative,
            brief: brief.into(),
            discovery: None,
            external_ref: None,
            version: 1,
        })
    }
    pub fn from_persisted(
        key: Key,
        title: String,
        initiative: Option<Key>,
        brief: String,
        discovery: Option<DiscoveryMark>,
        external_ref: Option<String>,
        version: i64,
    ) -> Self {
        Self {
            key,
            title,
            initiative,
            brief,
            discovery,
            external_ref,
            version,
        }
    }
    pub fn key(&self) -> &Key {
        &self.key
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn initiative(&self) -> Option<&Key> {
        self.initiative.as_ref()
    }
    pub fn brief(&self) -> &str {
        &self.brief
    }
    pub fn discovery(&self) -> Option<&DiscoveryMark> {
        self.discovery.as_ref()
    }
    /// The origin a hand-created item records (a Jira key it was migrated from); validated by the handler.
    pub fn with_external_ref(mut self, external_ref: Option<String>) -> Self {
        self.external_ref = external_ref;
        self
    }
    pub fn external_ref(&self) -> Option<&str> {
        self.external_ref.as_deref()
    }
    pub fn version(&self) -> i64 {
        self.version
    }
}

impl Story {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        key: Key,
        title: &str,
        kind: Kind,
        epic: Option<Key>,
        description: &str,
        criteria: Vec<String>,
        owner: Option<String>,
        labels: Vec<String>,
    ) -> Result<Self, ItemError> {
        check_title(title)?;
        if criteria.iter().any(|c| c.trim().is_empty()) {
            return Err(ItemError::BlankCriterion);
        }
        Ok(Self {
            key,
            title: title.trim().into(),
            kind,
            epic,
            description: description.into(),
            criteria: criteria
                .into_iter()
                .map(|t| Criterion {
                    text: t.trim().into(),
                    done: false,
                })
                .collect(),
            owner,
            labels,
            rank: 0,
            delivery: DeliveryState::Todo,
            resolution: None,
            external_ref: None,
            version: 1,
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub fn from_persisted(
        key: Key,
        title: String,
        kind: Kind,
        epic: Option<Key>,
        description: String,
        criteria: Vec<Criterion>,
        owner: Option<String>,
        labels: Vec<String>,
        rank: i32,
        delivery: DeliveryState,
        resolution: Option<Resolution>,
        external_ref: Option<String>,
        version: i64,
    ) -> Self {
        Self {
            key,
            title,
            kind,
            epic,
            description,
            criteria,
            owner,
            labels,
            rank,
            delivery,
            resolution,
            external_ref,
            version,
        }
    }
    pub fn key(&self) -> &Key {
        &self.key
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn kind(&self) -> Kind {
        self.kind
    }
    pub fn epic(&self) -> Option<&Key> {
        self.epic.as_ref()
    }
    pub fn description(&self) -> &str {
        &self.description
    }
    pub fn criteria(&self) -> &[Criterion] {
        &self.criteria
    }
    pub fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }
    pub fn labels(&self) -> &[String] {
        &self.labels
    }
    pub fn rank(&self) -> i32 {
        self.rank
    }
    pub fn delivery(&self) -> DeliveryState {
        self.delivery
    }
    pub fn resolution(&self) -> Option<Resolution> {
        self.resolution
    }
    /// The origin a hand-created item records (a Jira key it was migrated from); validated by the handler.
    pub fn with_external_ref(mut self, external_ref: Option<String>) -> Self {
        self.external_ref = external_ref;
        self
    }
    /// A migrated story created in the state its ticket had; the handler allows it only with a reference.
    pub fn grandfathered(mut self, delivery: DeliveryState) -> Self {
        self.delivery = delivery;
        self
    }
    pub fn external_ref(&self) -> Option<&str> {
        self.external_ref.as_deref()
    }
    pub fn version(&self) -> i64 {
        self.version
    }
    /// The delivery policy: two rules, everything else free (kb/ADR-0002 R4). Readiness is computed by the caller
    /// (kb/ADR-0018 R1); nothing about sprints or goals is an input (kb/ADR-0014 R4). On `Moved` the story carries
    /// the new delivery, the resolution (the given one when entering done, none otherwise) and, when it enters an
    /// active state without an owner, `actor_email` (normalised by the caller) as its owner (kb/ADR-0027 R6). Untouched on `Unchanged` and on refusal.
    pub fn move_to(
        &mut self,
        m: Move,
        ready: bool,
        via: TokenKind,
        actor_email: &str,
    ) -> Result<Moved, MoveRefused> {
        use DeliveryState::*;
        let from = self.delivery;
        // Done to done changes nothing unless a different resolution is named; then the done rule below applies to it,
        // so a completed story may be re-resolved as dropped, but a dropped one cannot become completed without
        // active work.
        let re_resolve = from == Done
            && m.to == Done
            && m.resolution.is_some()
            && m.resolution != self.resolution;
        if from == m.to && !re_resolve {
            return Ok(Moved::Unchanged);
        }
        let (resolution, override_reason) = if m.to == Done {
            match m.resolution {
                None => return Err(MoveRefused::ResolutionRequired),
                Some(Resolution::Completed) if !from.is_active() => {
                    return Err(MoveRefused::CompletedNeedsActiveOrigin);
                }
                Some(r) => (Some(r), None),
            }
        } else if m.to.is_active() && !from.is_active() && !ready {
            match (m.override_reason, via) {
                (None, _) => return Err(MoveRefused::NotReady),
                (Some(_), TokenKind::Automation) => {
                    return Err(MoveRefused::AutomationCannotOverride);
                }
                (Some(r), TokenKind::Interactive) if r.trim().is_empty() => {
                    return Err(MoveRefused::OverrideNeedsReason);
                }
                (Some(r), TokenKind::Interactive) => (None, Some(r.trim().to_string())),
            }
        } else {
            (None, None)
        };
        self.delivery = m.to;
        self.resolution = resolution;
        if m.to.is_active() && self.owner.is_none() {
            self.owner = Some(actor_email.to_string());
        }
        Ok(Moved::Moved {
            from,
            override_reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_validates_title_and_horizon() {
        let b = vec!["Now".to_string(), "Later".to_string()];
        assert_eq!(
            Initiative::new("ABC-1".parse().unwrap(), " ", "", "Now", &b, None, None).unwrap_err(),
            ItemError::Title
        );
        assert!(matches!(
            Initiative::new("ABC-1".parse().unwrap(), "x", "", "Soon", &b, None, None).unwrap_err(),
            ItemError::Horizon(_)
        ));
        assert_eq!(
            Story::new(
                "ABC-2".parse().unwrap(),
                "t",
                Kind::Story,
                None,
                "",
                vec!["ok".into(), " ".into()],
                None,
                vec![]
            )
            .unwrap_err(),
            ItemError::BlankCriterion
        );
    }
    #[test]
    fn a_kept_horizon_is_valid_and_a_changed_one_must_be_a_bucket() {
        let b = vec!["Soon".to_string(), "Later".to_string()];
        assert_eq!(check_horizon_change("Now", "Now", &b), Ok(()));
        assert_eq!(check_horizon_change("Now", "Soon", &b), Ok(()));
        assert_eq!(
            check_horizon_change("Now", "Someday", &b),
            Err(ItemError::Horizon("Someday".into()))
        );
        assert_eq!(check_horizon_change("Soon", "Soon", &[]), Ok(()));
    }
    #[test]
    fn title_and_criteria_are_trimmed() {
        let s = Story::new(
            "ABC-2".parse().unwrap(),
            "  t  ",
            Kind::Story,
            None,
            "",
            vec!["  ok ".into()],
            None,
            vec![],
        )
        .unwrap();
        assert_eq!(s.title(), "t");
        assert_eq!(s.criteria()[0].text, "ok");
    }
    #[test]
    fn title_length_boundary() {
        let new = |t: &str| Epic::new("ABC-2".parse().unwrap(), t, None, "");
        assert!(new(&"a".repeat(200)).is_ok());
        assert_eq!(new(&"a".repeat(201)).unwrap_err(), ItemError::Title);
    }
    #[test]
    fn horizon_match_is_exact() {
        let b = vec!["Now".to_string()];
        for h in ["now", " Now"] {
            assert!(matches!(
                Initiative::new("ABC-1".parse().unwrap(), "x", "", h, &b, None, None).unwrap_err(),
                ItemError::Horizon(_)
            ));
        }
    }

    mod moves {
        use super::super::*;
        use reeve_protocol::{DeliveryState::*, Resolution, TokenKind};

        fn story(
            delivery: DeliveryState,
            resolution: Option<Resolution>,
            owner: Option<&str>,
        ) -> Story {
            Story::from_persisted(
                "ABC-1".parse().unwrap(),
                "t".into(),
                Kind::Story,
                None,
                "".into(),
                vec![],
                owner.map(Into::into),
                vec![],
                0,
                delivery,
                resolution,
                None,
                3,
            )
        }
        fn mv(to: DeliveryState, resolution: Option<Resolution>, why: Option<&str>) -> Move {
            Move {
                to,
                resolution,
                override_reason: why.map(Into::into),
            }
        }
        /// One move of a fresh story: it starts in `from` (with `current` as its resolution), unowned; returns the
        /// outcome and the story after the call.
        #[allow(clippy::too_many_arguments)]
        fn go(
            from: DeliveryState,
            current: Option<Resolution>,
            to: DeliveryState,
            ready: bool,
            resolution: Option<Resolution>,
            why: Option<&str>,
            via: TokenKind,
        ) -> (Result<Moved, MoveRefused>, Story) {
            let mut s = story(from, current, None);
            let r = s.move_to(mv(to, resolution, why), ready, via, "dan@example.com");
            (r, s)
        }
        fn plain(from: DeliveryState) -> Result<Moved, MoveRefused> {
            Ok(Moved::Moved {
                from,
                override_reason: None,
            })
        }
        const I: TokenKind = TokenKind::Interactive;
        const A: TokenKind = TokenKind::Automation;

        #[test]
        fn ready_story_may_enter_active_from_todo_or_done() {
            for from in [Todo, Done] {
                for to in [InProgress, InReview, Testing] {
                    let (r, s) = go(from, None, to, true, None, None, I);
                    assert_eq!(r, plain(from));
                    assert_eq!((s.delivery(), s.resolution()), (to, None));
                }
            }
        }
        #[test]
        fn unready_story_needs_an_interactive_override_to_enter_active() {
            assert_eq!(
                go(Todo, None, Testing, false, None, None, I).0,
                Err(MoveRefused::NotReady)
            );
            let (r, s) = go(Todo, None, Testing, false, None, Some("urgent fix"), I);
            assert_eq!(
                r,
                Ok(Moved::Moved {
                    from: Todo,
                    override_reason: Some("urgent fix".into())
                })
            );
            assert_eq!(s.delivery(), Testing);
            assert_eq!(
                go(Todo, None, Testing, false, None, Some("urgent fix"), A).0,
                Err(MoveRefused::AutomationCannotOverride)
            );
            assert_eq!(
                go(Todo, None, InProgress, false, None, Some("   "), I).0,
                Err(MoveRefused::OverrideNeedsReason)
            );
        }
        #[test]
        fn moves_between_active_states_are_free() {
            for (a, b) in [
                (InProgress, InReview),
                (InReview, Testing),
                (Testing, InProgress),
                (InReview, InProgress),
            ] {
                assert_eq!(go(a, None, b, false, None, None, I).0, plain(a));
            }
            assert_eq!(
                go(InProgress, None, Todo, false, None, None, I).0,
                plain(InProgress),
                "release is free"
            );
        }
        #[test]
        fn done_needs_a_resolution_and_completed_needs_an_active_origin() {
            assert_eq!(
                go(Testing, None, Done, false, None, None, I).0,
                Err(MoveRefused::ResolutionRequired)
            );
            let (r, s) = go(
                Testing,
                None,
                Done,
                false,
                Some(Resolution::Completed),
                None,
                I,
            );
            assert_eq!(r, plain(Testing));
            assert_eq!(s.resolution(), Some(Resolution::Completed));
            assert_eq!(
                go(
                    Todo,
                    None,
                    Done,
                    false,
                    Some(Resolution::Completed),
                    None,
                    I
                )
                .0,
                Err(MoveRefused::CompletedNeedsActiveOrigin)
            );
            let (r, s) = go(Todo, None, Done, false, Some(Resolution::Dropped), None, I);
            assert_eq!(r, plain(Todo), "dropping from todo is the one ungated exit");
            assert_eq!(s.resolution(), Some(Resolution::Dropped));
        }
        #[test]
        fn edge_cases() {
            for (from, to) in [(Todo, InProgress), (Done, Testing)] {
                for tk in [I, A] {
                    let (r, _) = go(from, None, to, true, None, Some("why"), tk);
                    assert_eq!(r, plain(from));
                }
            }
            let dropped = Some(Resolution::Dropped);
            assert_eq!(
                go(Done, dropped, Done, false, dropped, None, I).0,
                Ok(Moved::Unchanged),
                "same resolution"
            );
            assert_eq!(
                go(Done, dropped, Done, false, None, None, I).0,
                Ok(Moved::Unchanged),
                "no resolution named"
            );
            assert_eq!(
                go(
                    Done,
                    dropped,
                    Done,
                    false,
                    Some(Resolution::Completed),
                    None,
                    I
                )
                .0,
                Err(MoveRefused::CompletedNeedsActiveOrigin),
                "dropped cannot become completed without active work"
            );
            let (r, s) = go(
                Done,
                Some(Resolution::Completed),
                Done,
                false,
                dropped,
                None,
                I,
            );
            assert_eq!(
                r,
                plain(Done),
                "a completed story may be re-resolved as dropped"
            );
            assert_eq!(s.resolution(), dropped);
            assert_eq!(go(Done, None, Todo, false, None, None, I).0, plain(Done));
            assert_eq!(
                go(Done, None, InProgress, false, None, None, I).0,
                Err(MoveRefused::NotReady)
            );
            for from in [InProgress, InReview, Testing] {
                let (r, s) = go(
                    from,
                    None,
                    Done,
                    false,
                    Some(Resolution::Completed),
                    None,
                    I,
                );
                assert_eq!(r, plain(from));
                assert_eq!(s.resolution(), Some(Resolution::Completed));
            }
            assert_eq!(
                go(Todo, None, Testing, false, None, Some("  "), A).0,
                Err(MoveRefused::AutomationCannotOverride)
            );
            assert_eq!(
                go(Todo, None, Testing, false, None, Some("  urgent  "), I).0,
                Ok(Moved::Moved {
                    from: Todo,
                    override_reason: Some("urgent".into())
                })
            );
        }
        #[test]
        fn same_state_is_a_no_op() {
            assert_eq!(
                go(Todo, None, Todo, false, None, None, I).0,
                Ok(Moved::Unchanged)
            );
        }

        /// A refusal leaves the story exactly as it was. Catches a move_to that assigns before validating.
        #[test]
        fn a_refused_move_leaves_the_story_untouched() {
            for (s, m, ready, via) in [
                (
                    story(Todo, None, None),
                    mv(InProgress, None, None),
                    false,
                    I,
                ),
                (
                    story(Todo, None, None),
                    mv(InProgress, None, Some("x")),
                    false,
                    A,
                ),
                (
                    story(Todo, None, None),
                    mv(InProgress, None, Some(" ")),
                    false,
                    I,
                ),
                (story(InProgress, None, None), mv(Done, None, None), true, I),
                (
                    story(Todo, None, None),
                    mv(Done, Some(Resolution::Completed), None),
                    true,
                    I,
                ),
            ] {
                let mut after = s.clone();
                assert!(after.move_to(m, ready, via, "dan@example.com").is_err());
                assert_eq!(after, s);
            }
        }

        /// Owner on start, four cases. Catches the rule applied on other states or over an existing owner.
        #[test]
        fn entering_active_work_from_todo_or_done_assigns_an_unowned_story_to_the_mover() {
            let mut a = story(Todo, None, None);
            a.move_to(mv(InProgress, None, None), true, I, "dan@example.com")
                .unwrap();
            assert_eq!(a.owner(), Some("dan@example.com"));
            let mut b = story(Todo, None, Some("mia@example.com"));
            b.move_to(mv(InProgress, None, None), true, I, "dan@example.com")
                .unwrap();
            assert_eq!(b.owner(), Some("mia@example.com"));
            let mut c = story(Todo, None, None);
            c.move_to(mv(Testing, None, None), true, I, "dan@example.com")
                .unwrap();
            assert_eq!(
                c.owner(),
                Some("dan@example.com"),
                "todo to testing assigns too (R6)"
            );
            let mut c2 = story(Done, Some(Resolution::Dropped), None);
            c2.move_to(mv(InReview, None, None), true, I, "dan@example.com")
                .unwrap();
            assert_eq!(
                c2.owner(),
                Some("dan@example.com"),
                "done to in review assigns too (R6)"
            );
            let mut c4 = story(Todo, None, None);
            c4.move_to(
                mv(Done, Some(Resolution::Dropped), None),
                true,
                I,
                "dan@example.com",
            )
            .unwrap();
            assert_eq!(c4.owner(), None, "dropping is not starting");
            let mut d = story(InProgress, None, None);
            let before = d.clone();
            assert_eq!(
                d.move_to(mv(InProgress, None, None), true, I, "dan@example.com")
                    .unwrap(),
                Moved::Unchanged
            );
            assert_eq!(d, before, "Unchanged leaves the whole story as it was");
            let mut e = story(Done, Some(Resolution::Completed), None);
            let before = e.clone();
            assert_eq!(
                e.move_to(mv(Done, Some(Resolution::Completed), None), true, I, "d")
                    .unwrap(),
                Moved::Unchanged
            );
            assert_eq!(e, before);
        }

        /// A successful move sets delivery and resolution and reports where it came from; leaving done clears the
        /// resolution.
        #[test]
        fn a_move_sets_delivery_and_resolution_and_reports_from() {
            let mut s = story(InProgress, None, None);
            assert_eq!(
                s.move_to(mv(Done, Some(Resolution::Completed), None), true, I, "d")
                    .unwrap(),
                Moved::Moved {
                    from: InProgress,
                    override_reason: None
                }
            );
            assert_eq!(
                (s.delivery(), s.resolution()),
                (Done, Some(Resolution::Completed))
            );
            assert_eq!(
                s.move_to(mv(InProgress, None, None), true, I, "d").unwrap(),
                Moved::Moved {
                    from: Done,
                    override_reason: None
                }
            );
            assert_eq!((s.delivery(), s.resolution()), (InProgress, None));
        }

        #[test]
        fn move_refusals_keep_their_messages() {
            assert_eq!(MoveRefused::NotReady.to_string(), "story is not ready");
            assert_eq!(
                MoveRefused::AutomationCannotOverride.to_string(),
                "automation tokens cannot override the gate"
            );
            assert_eq!(
                MoveRefused::OverrideNeedsReason.to_string(),
                "an override needs a written reason"
            );
            assert_eq!(
                MoveRefused::ResolutionRequired.to_string(),
                "entering done requires a resolution"
            );
            assert_eq!(
                MoveRefused::CompletedNeedsActiveOrigin.to_string(),
                "completed is only allowed from an active state"
            );
        }
    }
}
