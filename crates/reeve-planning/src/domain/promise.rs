//! Promises (kb/ADR-0025): a date by which a story or an epic will be done. Pure: dates and facts in, state out.
//! Nothing here reads the clock or the database; `today` is the handler's `c.now` as a UTC date.
use chrono::{DateTime, NaiveDate, Utc};
use reeve_protocol::{DeliveryState, Key, NeededIn, Resolution};

pub const MAX_RECIPIENT: usize = 200;

/// A stored promise (R1). `to` is trimmed and never blank. `at` is when the current date was promised (R3's boundary).
/// The fields are private: `change` is the only constructor that validates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Promise {
    on: NaiveDate,
    by: String,
    at: DateTime<Utc>,
    to: Option<String>,
}

impl Promise {
    /// Rebuilds a promise from stored columns for the repository adapter. It does not validate: the table's checks
    /// and `change` already did.
    pub fn from_persisted(
        on: NaiveDate,
        by: String,
        at: DateTime<Utc>,
        to: Option<String>,
    ) -> Promise {
        Promise { on, by, at, to }
    }
    pub fn on(&self) -> NaiveDate {
        self.on
    }
    pub fn by(&self) -> &str {
        &self.by
    }
    pub fn at(&self) -> DateTime<Utc> {
        self.at
    }
    pub fn to(&self) -> Option<&str> {
        self.to.as_deref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    ThisSprint,
    NextSprint,
}

impl Shortcut {
    fn sprint(self) -> &'static str {
        match self {
            Shortcut::ThisSprint => "the active sprint",
            Shortcut::NextSprint => "the next sprint",
        }
    }
}

/// What the person asked for: a date, a sprint shortcut or neither (recipient only), and the recipient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub on: Option<NaiveDate>,
    pub shortcut: Option<Shortcut>,
    /// None: keep the current recipient; Some(""): clear it.
    pub to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PromiseError {
    /// The story is done (R2).
    #[error("{0} is done: a done story cannot be promised")]
    Done(Key),
    /// A new date before today (R2).
    #[error("the date {0} is past: promise today or later")]
    Past(NaiveDate),
    #[error("the recipient is longer than {max} characters", max = MAX_RECIPIENT)]
    RecipientTooLong,
    /// PostgreSQL text cannot hold a NUL; refused here rather than failing the write.
    #[error("the recipient cannot contain a NUL character")]
    RecipientHasNul,
    /// A shortcut whose sprint does not exist (R6).
    #[error("{}", match .0 {
        Shortcut::ThisSprint => "there is no active sprint: give a date",
        Shortcut::NextSprint => "there is no next sprint yet: give a date",
    })]
    NoSprint(Shortcut),
    /// A shortcut whose planned end is before today (R6).
    #[error("{} is planned to end {}, which is past: give a date", .0.sprint(), .1)]
    SprintEndPassed(Shortcut, NaiveDate),
    /// Both a date and a shortcut.
    #[error("give a date or a sprint shortcut, not both")]
    Both,
    /// Neither, and no current promise whose recipient could change.
    #[error("give a date or a sprint shortcut")]
    Neither,
}

/// R2: a done story (dropped included: dropped is a resolution of done) cannot be promised.
pub fn may_promise(key: &Key, delivery: DeliveryState) -> Result<(), PromiseError> {
    if delivery == DeliveryState::Done {
        return Err(PromiseError::Done(key.clone()));
    }
    Ok(())
}

/// Today and the two planned ends, read once per snapshot. `today` is the request's UTC date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    pub today: NaiveDate,
    pub active_ends: Option<NaiveDate>,
    pub draft_ends: Option<NaiveDate>,
}

/// R6: the planned end of the shortcut's sprint; refused when the sprint is missing or its end is before today.
pub fn shortcut(which: Shortcut, clock: &Clock) -> Result<NaiveDate, PromiseError> {
    let ends = match which {
        Shortcut::ThisSprint => clock.active_ends,
        Shortcut::NextSprint => clock.draft_ends,
    }
    .ok_or(PromiseError::NoSprint(which))?;
    if ends < clock.today {
        return Err(PromiseError::SprintEndPassed(which, ends));
    }
    Ok(ends)
}

fn clean_to(to: String) -> Result<Option<String>, PromiseError> {
    let to = Some(to.trim().to_string()).filter(|s| !s.is_empty());
    if to
        .as_ref()
        .is_some_and(|s| s.chars().count() > MAX_RECIPIENT)
    {
        return Err(PromiseError::RecipientTooLong);
    }
    if to.as_ref().is_some_and(|s| s.contains('\0')) {
        return Err(PromiseError::RecipientHasNul);
    }
    Ok(to)
}

/// R2 and the change rules: the promise to store, given the current one (if any). `clock.today` is the UTC date of
/// `at`, the request's timestamp.
/// - A date or a shortcut, not both. Neither: change only the recipient of the current promise (refused without one).
/// - A new date before today is refused; keeping the current date is not (an overdue promise's recipient can change).
/// - Keeping the date keeps `at` (R3's boundary does not move) and records the editor in `by`; a new date sets both.
/// - `to` None keeps the current recipient; blank clears it.
pub fn change(
    current: Option<&Promise>,
    req: Request,
    by: &str,
    at: DateTime<Utc>,
    clock: &Clock,
) -> Result<Promise, PromiseError> {
    let on = match (req.on, req.shortcut) {
        (Some(_), Some(_)) => return Err(PromiseError::Both),
        (Some(d), None) => d,
        (None, Some(s)) => shortcut(s, clock)?,
        (None, None) => current.ok_or(PromiseError::Neither)?.on,
    };
    let kept = current.filter(|c| c.on == on);
    if kept.is_none() && on < clock.today {
        return Err(PromiseError::Past(on));
    }
    let to = match req.to {
        None => current.and_then(|c| c.to.clone()),
        Some(t) => clean_to(t)?,
    };
    Ok(Promise {
        on,
        by: by.to_string(),
        at: kept.map_or(at, |c| c.at),
        to,
    })
}

/// R3: a story gets its epic's promise unless it was done before that promise was set. A done story without
/// `done_at` (imported already done) never inherits.
pub fn inherits(
    epic_promised_at: DateTime<Utc>,
    delivery: DeliveryState,
    done_at: Option<DateTime<Utc>>,
) -> bool {
    delivery != DeliveryState::Done || done_at.is_some_and(|d| d >= epic_promised_at)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holder {
    Own,
    Epic,
}

/// R3: the earlier date; the story's own wins a tie, so its recipient is the one shown.
pub fn effective(
    own: Option<NaiveDate>,
    from_epic: Option<NaiveDate>,
) -> Option<(NaiveDate, Holder)> {
    match (own, from_epic) {
        (Some(o), Some(e)) if e < o => Some((e, Holder::Epic)),
        (Some(o), _) => Some((o, Holder::Own)),
        (None, Some(e)) => Some((e, Holder::Epic)),
        (None, None) => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Open,
    Overdue,
    Kept,
    Late,
    Dropped,
}

/// R4: the UTC day a story was done; the one place that rule lives.
pub fn done_on(done_at: DateTime<Utc>) -> NaiveDate {
    done_at.date_naive()
}

/// R4. A done story without a done day has no state, dropped or not; otherwise the done day decides.
pub fn state(
    on: NaiveDate,
    delivery: DeliveryState,
    resolution: Option<Resolution>,
    done: Option<NaiveDate>,
    today: NaiveDate,
) -> Option<State> {
    if delivery != DeliveryState::Done {
        return Some(if today > on {
            State::Overdue
        } else {
            State::Open
        });
    }
    let done = done?;
    if resolution == Some(Resolution::Dropped) {
        return Some(State::Dropped);
    }
    Some(if done <= on { State::Kept } else { State::Late })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    ThisSprint,
    NextSprint,
    Later,
}

/// R5, for open promises (and an epic's promise that is not past). Bounds inclusive; a missing sprint never matches.
/// This follows R5 as written: an active sprint that has run past its planned end still classifies by that end.
pub fn due(on: NaiveDate, clock: &Clock) -> Due {
    if clock.active_ends.is_some_and(|e| on <= e) {
        Due::ThisSprint
    } else if clock.draft_ends.is_some_and(|e| on <= e) {
        Due::NextSprint
    } else {
        Due::Later
    }
}

/// Days overdue (today - on) or days late (done day - on); None otherwise.
pub fn days(st: State, on: NaiveDate, done_on: Option<NaiveDate>, today: NaiveDate) -> Option<u32> {
    let diff = |a: NaiveDate| u32::try_from((a - on).num_days()).ok();
    match st {
        State::Overdue => diff(today),
        State::Late => done_on.and_then(diff),
        State::Open | State::Kept | State::Dropped => None,
    }
}

/// R8: the planned end a sprint's promise list starts after: the greatest planned end among the other non-draft
/// sprints below this one's, ties broken by the lower id. `others` excludes this sprint and the draft.
pub fn previous_end(this: (i64, NaiveDate), others: &[(i64, NaiveDate)]) -> Option<NaiveDate> {
    others
        .iter()
        .filter(|(id, e)| *e < this.1 || (*e == this.1 && *id < this.0))
        .map(|(_, e)| *e)
        .max()
}

/// R8: after the previous planned end (exclusive) and up to this one's (inclusive).
pub fn in_sprint(on: NaiveDate, previous_ends: Option<NaiveDate>, ends: NaiveDate) -> bool {
    previous_ends.is_none_or(|p| on > p) && on <= ends
}

/// R7: the decision window an unfinished story's promise asks for; only this sprint and next sprint.
pub fn window(st: Option<State>, d: Option<Due>) -> NeededIn {
    match (st, d) {
        (Some(State::Overdue), _) => NeededIn::ThisSprint,
        (Some(State::Open), Some(Due::ThisSprint)) => NeededIn::ThisSprint,
        (Some(State::Open), Some(Due::NextSprint)) => NeededIn::NextSprint,
        (Some(State::Open), Some(Due::Later) | None) => NeededIn::None,
        (Some(State::Kept | State::Late | State::Dropped) | None, _) => NeededIn::None,
    }
}

/// The `promised` story-list filter, classified only through `state` and `due` (R5). Unfinished stories only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    /// Overdue, or open and due this sprint.
    ThisSprint,
    NextSprint,
    Overdue,
    /// Open or overdue.
    Any,
}

impl Filter {
    /// Whether a story's promise passes this filter; the window decides the sprint filters.
    pub fn matches(self, sp: &StoryPromise) -> bool {
        match self {
            Filter::ThisSprint => window(sp.state, sp.due) == NeededIn::ThisSprint,
            Filter::NextSprint => window(sp.state, sp.due) == NeededIn::NextSprint,
            Filter::Overdue => sp.state == Some(State::Overdue),
            Filter::Any => matches!(sp.state, Some(State::Open | State::Overdue)),
        }
    }
}

/// Everything a reader shows about one story's promise, computed once (R3-R5).
#[derive(Debug, Clone, PartialEq)]
pub struct StoryPromise {
    pub own: Option<Promise>,
    /// The epic's key and promise, only when the story inherits it (R3).
    pub epic: Option<(Key, Promise)>,
    pub on: NaiveDate,
    pub holder: Holder,
    pub state: Option<State>,
    /// Only for `Open`.
    pub due: Option<Due>,
    pub days: Option<u32>,
    /// The UTC day of `done_at`, when the story is done with one.
    pub done_on: Option<NaiveDate>,
}

/// The story facts `story_promise` reads: its delivery, and the promises it owns and inherits from its epic.
#[derive(Debug, Clone, Copy)]
pub struct StoryFacts<'a> {
    pub delivery: DeliveryState,
    pub resolution: Option<Resolution>,
    pub done_at: Option<DateTime<Utc>>,
    pub own: Option<&'a Promise>,
    pub epic: Option<(&'a Key, &'a Promise)>,
}

/// One story's promise view (R3-R5), or None when neither the story nor an inherited epic promise applies.
pub fn story_promise(f: &StoryFacts<'_>, clock: &Clock) -> Option<StoryPromise> {
    let epic = f
        .epic
        .filter(|(_, p)| inherits(p.at, f.delivery, f.done_at));
    let (on, holder) = effective(f.own.map(|p| p.on), epic.map(|(_, p)| p.on))?;
    let done = f.done_at.map(done_on);
    let st = state(on, f.delivery, f.resolution, done, clock.today);
    let d = (st == Some(State::Open)).then(|| due(on, clock));
    Some(StoryPromise {
        own: f.own.cloned(),
        epic: epic.map(|(k, p)| (k.clone(), p.clone())),
        on,
        holder,
        state: st,
        due: d,
        days: st.and_then(|s| days(s, on, done, clock.today)),
        done_on: done,
    })
}

/// An epic's own promise has no state (R4): its due period while not past, None once past.
pub fn epic_due(p: &Promise, clock: &Clock) -> Option<Due> {
    (clock.today <= p.on).then(|| due(p.on, clock))
}

/// One row of a sprint's promise list (R8), built by the handler from the reads and `story_promise`.
#[derive(Debug, Clone, PartialEq)]
pub struct SprintPromise {
    pub key: Key,
    pub title: String,
    pub promise: StoryPromise,
}

#[cfg(test)]
mod tests {
    use super::*;
    use DeliveryState::{Done, InProgress, InReview, Testing, Todo};

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }
    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }
    fn clock(today: NaiveDate, active: Option<NaiveDate>, draft: Option<NaiveDate>) -> Clock {
        Clock {
            today,
            active_ends: active,
            draft_ends: draft,
        }
    }
    fn key(s: &str) -> Key {
        s.parse().unwrap()
    }
    fn promise(on: NaiveDate, at: &str, to: Option<&str>) -> Promise {
        Promise::from_persisted(on, "old@x.y".into(), t(at), to.map(str::to_string))
    }

    #[test]
    fn state_follows_the_utc_day_of_done_at() {
        use Resolution::{Completed, Dropped};
        let on = d(2026, 11, 20);
        #[allow(clippy::type_complexity)]
        let rows: Vec<(
            &str,
            DeliveryState,
            Option<Resolution>,
            Option<&str>,
            NaiveDate,
            Option<State>,
        )> = vec![
            (
                "done 23:59:59Z on the day",
                Done,
                Some(Completed),
                Some("2026-11-20T23:59:59Z"),
                d(2026, 12, 1),
                Some(State::Kept),
            ),
            (
                "done 00:00:00Z the day after",
                Done,
                Some(Completed),
                Some("2026-11-21T00:00:00Z"),
                d(2026, 12, 1),
                Some(State::Late),
            ),
            (
                "done 00:00:00Z on the day",
                Done,
                Some(Completed),
                Some("2026-11-20T00:00:00Z"),
                d(2026, 12, 1),
                Some(State::Kept),
            ),
            (
                "done the day before",
                Done,
                Some(Completed),
                Some("2026-11-19T12:00:00Z"),
                d(2026, 12, 1),
                Some(State::Kept),
            ),
            (
                "dropped with done_at on time",
                Done,
                Some(Dropped),
                Some("2026-11-19T12:00:00Z"),
                d(2026, 12, 1),
                Some(State::Dropped),
            ),
            (
                "dropped with done_at after the date",
                Done,
                Some(Dropped),
                Some("2026-11-25T12:00:00Z"),
                d(2026, 12, 1),
                Some(State::Dropped),
            ),
            (
                "dropped without done_at",
                Done,
                Some(Dropped),
                None,
                d(2026, 12, 1),
                None,
            ),
            (
                "completed without done_at",
                Done,
                Some(Completed),
                None,
                d(2026, 12, 1),
                None,
            ),
            (
                "done without resolution or done_at",
                Done,
                None,
                None,
                d(2026, 12, 1),
                None,
            ),
            (
                "todo on the day",
                Todo,
                None,
                None,
                d(2026, 11, 20),
                Some(State::Open),
            ),
            (
                "todo before the day",
                Todo,
                None,
                None,
                d(2026, 11, 1),
                Some(State::Open),
            ),
            (
                "todo the day after",
                Todo,
                None,
                None,
                d(2026, 11, 21),
                Some(State::Overdue),
            ),
            (
                "in progress the day after",
                InProgress,
                None,
                None,
                d(2026, 11, 21),
                Some(State::Overdue),
            ),
            (
                "in progress with a stale done_at and resolution, the day after",
                InProgress,
                Some(Completed),
                Some("2026-11-10T12:00:00Z"),
                d(2026, 11, 21),
                Some(State::Overdue),
            ),
            (
                "in progress with a stale done_at, on the day",
                InProgress,
                Some(Completed),
                Some("2026-11-10T12:00:00Z"),
                d(2026, 11, 20),
                Some(State::Open),
            ),
            (
                "in review on the day",
                InReview,
                None,
                None,
                d(2026, 11, 20),
                Some(State::Open),
            ),
            (
                "testing the day after",
                Testing,
                None,
                None,
                d(2026, 11, 21),
                Some(State::Overdue),
            ),
        ];
        for (label, delivery, res, done_at, today, want) in rows {
            let got = state(on, delivery, res, done_at.map(|s| done_on(t(s))), today);
            assert_eq!(got, want, "{label}");
        }
    }

    #[test]
    fn due_bounds_are_inclusive_and_missing_sprints_never_match() {
        let a = d(2026, 11, 20);
        let n = d(2026, 12, 4);
        let today = d(2026, 11, 10);
        let rows = [
            (
                "on the active end",
                a,
                clock(today, Some(a), Some(n)),
                Due::ThisSprint,
            ),
            (
                "before the active end",
                d(2026, 11, 12),
                clock(today, Some(a), Some(n)),
                Due::ThisSprint,
            ),
            (
                "a day after the active end",
                d(2026, 11, 21),
                clock(today, Some(a), Some(n)),
                Due::NextSprint,
            ),
            (
                "on the draft end",
                n,
                clock(today, Some(a), Some(n)),
                Due::NextSprint,
            ),
            (
                "after the draft end",
                d(2026, 12, 5),
                clock(today, Some(a), Some(n)),
                Due::Later,
            ),
            (
                "no active sprint, before the draft end",
                d(2026, 11, 12),
                clock(today, None, Some(n)),
                Due::NextSprint,
            ),
            (
                "no draft sprint, after the active end",
                d(2026, 11, 21),
                clock(today, Some(a), None),
                Due::Later,
            ),
            (
                "no sprints",
                d(2026, 11, 12),
                clock(today, None, None),
                Due::Later,
            ),
            // R5 as written: an active sprint that ran past its planned end still classifies by that end.
            (
                "active sprint run past its end, dated today, no draft",
                d(2026, 11, 25),
                clock(d(2026, 11, 25), Some(a), None),
                Due::Later,
            ),
            (
                "active sprint run past its end, dated on its end",
                a,
                clock(d(2026, 11, 25), Some(a), None),
                Due::ThisSprint,
            ),
        ];
        for (label, on, c, want) in rows {
            assert_eq!(due(on, &c), want, "{label}");
        }
    }

    #[test]
    fn effective_takes_the_earlier_date_and_own_wins_a_tie() {
        let (a, b) = (d(2026, 11, 20), d(2026, 11, 25));
        let rows = [
            ("epic earlier", Some(b), Some(a), Some((a, Holder::Epic))),
            ("epic later", Some(a), Some(b), Some((a, Holder::Own))),
            ("equal", Some(a), Some(a), Some((a, Holder::Own))),
            ("own only", Some(a), None, Some((a, Holder::Own))),
            ("epic only", None, Some(a), Some((a, Holder::Epic))),
            ("neither", None, None, None),
        ];
        for (label, own, epic, want) in rows {
            assert_eq!(effective(own, epic), want, "{label}");
        }
    }

    #[test]
    fn inherits_only_what_was_unfinished_when_the_epic_promised() {
        let at = t("2026-11-10T12:00:00Z");
        let rows = [
            ("todo", Todo, None, true),
            (
                "in progress with a stale done_at",
                InProgress,
                Some("2026-11-01T00:00:00Z"),
                true,
            ),
            ("done before at", Done, Some("2026-11-10T11:59:59Z"), false),
            (
                "done exactly at at",
                Done,
                Some("2026-11-10T12:00:00Z"),
                true,
            ),
            ("done after at", Done, Some("2026-11-11T00:00:00Z"), true),
            ("done with no done_at", Done, None, false),
        ];
        for (label, delivery, done_at, want) in rows {
            assert_eq!(inherits(at, delivery, done_at.map(t)), want, "{label}");
        }
    }

    #[test]
    fn may_promise_refuses_only_done_stories() {
        let k = key("ABC-7");
        let rows = [
            ("todo", Todo, Ok(())),
            ("in progress", InProgress, Ok(())),
            ("in review", InReview, Ok(())),
            ("testing", Testing, Ok(())),
            ("done", Done, Err(PromiseError::Done(k.clone()))),
        ];
        for (label, delivery, want) in rows {
            assert_eq!(may_promise(&k, delivery), want, "{label}");
        }
    }

    #[test]
    fn change_applies_the_rules() {
        let now = t("2026-11-20T09:00:00Z");
        let today = d(2026, 11, 20);
        let std = clock(today, Some(d(2026, 11, 27)), Some(d(2026, 12, 11)));
        let overdue = promise(d(2026, 11, 15), "2026-11-01T08:00:00Z", Some("Acme"));
        let future = promise(d(2026, 11, 30), "2026-11-02T08:00:00Z", Some("Acme"));
        let req = |on: Option<NaiveDate>, shortcut: Option<Shortcut>, to: Option<&str>| Request {
            on,
            shortcut,
            to: to.map(str::to_string),
        };
        let long_ok = "é".repeat(200);
        let long_bad = "é".repeat(201);
        #[allow(clippy::type_complexity)]
        let rows: Vec<(
            &str,
            Option<Promise>,
            Request,
            Clock,
            Result<(NaiveDate, &str, DateTime<Utc>, Option<String>), PromiseError>,
        )> = vec![
            (
                "new promise with a date",
                None,
                req(Some(d(2026, 11, 25)), None, None),
                std,
                Ok((d(2026, 11, 25), "ed@x.y", now, None)),
            ),
            (
                "new promise dated today",
                None,
                req(Some(today), None, Some("Acme")),
                std,
                Ok((today, "ed@x.y", now, Some("Acme".into()))),
            ),
            (
                "new promise with neither",
                None,
                req(None, None, Some("Acme")),
                std,
                Err(PromiseError::Neither),
            ),
            (
                "both a date and a shortcut",
                None,
                req(Some(d(2026, 11, 25)), Some(Shortcut::ThisSprint), None),
                std,
                Err(PromiseError::Both),
            ),
            (
                "past date on a new promise",
                None,
                req(Some(d(2026, 11, 19)), None, None),
                std,
                Err(PromiseError::Past(d(2026, 11, 19))),
            ),
            (
                "past date that differs from the current",
                Some(overdue.clone()),
                req(Some(d(2026, 11, 16)), None, None),
                std,
                Err(PromiseError::Past(d(2026, 11, 16))),
            ),
            (
                "overdue, same past date, new recipient",
                Some(overdue.clone()),
                req(Some(overdue.on()), None, Some("Bolt")),
                std,
                Ok((overdue.on(), "ed@x.y", overdue.at(), Some("Bolt".into()))),
            ),
            (
                "overdue, no date, recipient cleared",
                Some(overdue.clone()),
                req(None, None, Some("")),
                std,
                Ok((overdue.on(), "ed@x.y", overdue.at(), None)),
            ),
            (
                "overdue, no date, recipient blank spaces clears",
                Some(overdue.clone()),
                req(None, None, Some("   ")),
                std,
                Ok((overdue.on(), "ed@x.y", overdue.at(), None)),
            ),
            (
                "same date, to omitted keeps the recipient",
                Some(future.clone()),
                req(Some(future.on()), None, None),
                std,
                Ok((future.on(), "ed@x.y", future.at(), Some("Acme".into()))),
            ),
            (
                "neither, to omitted: date, at and recipient kept, by is the editor",
                Some(future.clone()),
                req(None, None, None),
                std,
                Ok((future.on(), "ed@x.y", future.at(), Some("Acme".into()))),
            ),
            (
                "a new later date moves at",
                Some(future.clone()),
                req(Some(d(2026, 12, 5)), None, None),
                std,
                Ok((d(2026, 12, 5), "ed@x.y", now, Some("Acme".into()))),
            ),
            (
                "a new earlier future date moves at",
                Some(future.clone()),
                req(Some(d(2026, 11, 22)), None, Some("Bolt")),
                std,
                Ok((d(2026, 11, 22), "ed@x.y", now, Some("Bolt".into()))),
            ),
            (
                "a new date on an overdue promise moves at",
                Some(overdue.clone()),
                req(Some(d(2026, 11, 21)), None, None),
                std,
                Ok((d(2026, 11, 21), "ed@x.y", now, Some("Acme".into()))),
            ),
            (
                "shortcut with no sprint",
                None,
                req(None, Some(Shortcut::NextSprint), None),
                clock(today, Some(d(2026, 11, 27)), None),
                Err(PromiseError::NoSprint(Shortcut::NextSprint)),
            ),
            (
                "shortcut whose end has passed",
                None,
                req(None, Some(Shortcut::ThisSprint), None),
                clock(today, Some(d(2026, 11, 19)), None),
                Err(PromiseError::SprintEndPassed(
                    Shortcut::ThisSprint,
                    d(2026, 11, 19),
                )),
            ),
            (
                "shortcut stores the planned end",
                None,
                req(None, Some(Shortcut::NextSprint), None),
                std,
                Ok((d(2026, 12, 11), "ed@x.y", now, None)),
            ),
            (
                "shortcut to the same end as the current keeps at",
                Some(promise(d(2026, 11, 27), "2026-11-03T00:00:00Z", None)),
                req(None, Some(Shortcut::ThisSprint), None),
                std,
                Ok((d(2026, 11, 27), "ed@x.y", t("2026-11-03T00:00:00Z"), None)),
            ),
            (
                "200 characters of two bytes",
                None,
                req(Some(d(2026, 11, 25)), None, Some(&long_ok)),
                std,
                Ok((d(2026, 11, 25), "ed@x.y", now, Some(long_ok.clone()))),
            ),
            (
                "201 characters of two bytes",
                None,
                req(Some(d(2026, 11, 25)), None, Some(&long_bad)),
                std,
                Err(PromiseError::RecipientTooLong),
            ),
            (
                "200 characters plus padding trims to fit",
                None,
                req(Some(d(2026, 11, 25)), None, Some(&format!("  {long_ok} "))),
                std,
                Ok((d(2026, 11, 25), "ed@x.y", now, Some(long_ok.clone()))),
            ),
            (
                "a NUL in the recipient",
                None,
                req(Some(d(2026, 11, 25)), None, Some("Ac\0me")),
                std,
                Err(PromiseError::RecipientHasNul),
            ),
            (
                "recipient is trimmed",
                None,
                req(Some(d(2026, 11, 25)), None, Some("  Acme ")),
                std,
                Ok((d(2026, 11, 25), "ed@x.y", now, Some("Acme".into()))),
            ),
        ];
        for (label, current, request, c, want) in rows {
            let got = change(current.as_ref(), request, "ed@x.y", now, &c);
            let want = want.map(|(on, by, at, to)| Promise::from_persisted(on, by.into(), at, to));
            assert_eq!(got, want, "{label}");
        }
    }

    #[test]
    fn shortcut_resolves_planned_ends() {
        let today = d(2026, 11, 20);
        let rows = [
            (
                "active sprint ends today",
                Shortcut::ThisSprint,
                clock(today, Some(today), None),
                Ok(today),
            ),
            (
                "active sprint ended yesterday",
                Shortcut::ThisSprint,
                clock(today, Some(d(2026, 11, 19)), None),
                Err(PromiseError::SprintEndPassed(
                    Shortcut::ThisSprint,
                    d(2026, 11, 19),
                )),
            ),
            (
                "no active sprint",
                Shortcut::ThisSprint,
                clock(today, None, Some(d(2026, 12, 4))),
                Err(PromiseError::NoSprint(Shortcut::ThisSprint)),
            ),
            (
                "draft present",
                Shortcut::NextSprint,
                clock(today, Some(d(2026, 11, 27)), Some(d(2026, 12, 4))),
                Ok(d(2026, 12, 4)),
            ),
            (
                "draft absent",
                Shortcut::NextSprint,
                clock(today, Some(d(2026, 11, 27)), None),
                Err(PromiseError::NoSprint(Shortcut::NextSprint)),
            ),
        ];
        for (label, which, c, want) in rows {
            assert_eq!(shortcut(which, &c), want, "{label}");
        }
    }

    /// The sprint (by id) that owns `on`, with every sprint's previous end taken from the others.
    fn owner(sprints: &[(i64, NaiveDate)], on: NaiveDate) -> Vec<i64> {
        sprints
            .iter()
            .filter(|(id, ends)| {
                let others: Vec<_> = sprints.iter().copied().filter(|(o, _)| o != id).collect();
                in_sprint(on, previous_end((*id, *ends), &others), *ends)
            })
            .map(|(id, _)| *id)
            .collect()
    }

    // Shared by both tests: id 0 ended 12 Nov, id 1 ends 30 Nov (planned long, closed early elsewhere),
    // id 2 is active and ends 26 Nov. By planned end the order is 0, 2, 1.
    const S0: (i64, (i32, u32, u32)) = (0, (2026, 11, 12));
    const S1: (i64, (i32, u32, u32)) = (1, (2026, 11, 30));
    const S2: (i64, (i32, u32, u32)) = (2, (2026, 11, 26));
    fn sprint_of(s: (i64, (i32, u32, u32))) -> (i64, NaiveDate) {
        (s.0, d(s.1.0, s.1.1, s.1.2))
    }

    #[test]
    fn previous_end_picks_the_nearest_lower_planned_end() {
        let (s0, s1, s2) = (sprint_of(S0), sprint_of(S1), sprint_of(S2));
        #[allow(clippy::type_complexity)]
        let rows: Vec<(
            &str,
            (i64, NaiveDate),
            Vec<(i64, NaiveDate)>,
            Option<NaiveDate>,
        )> = vec![
            (
                "unsorted input",
                (9, d(2026, 11, 30)),
                vec![
                    (5, d(2026, 11, 24)),
                    (3, d(2026, 11, 10)),
                    (4, d(2026, 12, 8)),
                    (2, d(2026, 11, 17)),
                ],
                Some(d(2026, 11, 24)),
            ),
            (
                "equal ends: the higher id follows the lower",
                (6, d(2026, 11, 10)),
                vec![(5, d(2026, 11, 10))],
                Some(d(2026, 11, 10)),
            ),
            (
                "equal ends: the lower id has nothing before it",
                (5, d(2026, 11, 10)),
                vec![(6, d(2026, 11, 10))],
                None,
            ),
            (
                "the sprint ending 30 Nov follows the one ending 26 Nov",
                s1,
                vec![s2, s0],
                Some(d(2026, 11, 26)),
            ),
            (
                "the sprint ending 26 Nov follows the one ending 12 Nov",
                s2,
                vec![s1, s0],
                Some(d(2026, 11, 12)),
            ),
            ("the first sprint has none", s0, vec![s1, s2], None),
            ("no others", s0, vec![], None),
            (
                "only later ends",
                (1, d(2026, 11, 1)),
                vec![(2, d(2026, 11, 2))],
                None,
            ),
        ];
        for (label, this, others, want) in rows {
            assert_eq!(previous_end(this, &others), want, "{label}");
        }
    }

    #[test]
    fn in_sprint_puts_each_date_in_exactly_one_sprint() {
        let setups: Vec<(&str, Vec<(i64, NaiveDate)>)> = vec![
            (
                "plain",
                vec![
                    (1, d(2026, 11, 10)),
                    (2, d(2026, 11, 24)),
                    (3, d(2026, 11, 30)),
                ],
            ),
            (
                "ends 12, 30 and 26 Nov",
                vec![sprint_of(S0), sprint_of(S1), sprint_of(S2)],
            ),
            (
                "equal ends",
                vec![
                    (1, d(2026, 11, 10)),
                    (2, d(2026, 11, 10)),
                    (3, d(2026, 11, 30)),
                ],
            ),
        ];
        for (label, sprints) in &setups {
            for day in 1..=30 {
                let on = d(2026, 11, day);
                let hits = owner(sprints, on);
                assert_eq!(hits.len(), 1, "{label}: {on} matched {hits:?}");
            }
        }
        let plain = &setups[0].1;
        let rows = [
            ("everything before the first end", d(2026, 10, 1), vec![1]),
            ("first end inclusive", d(2026, 11, 10), vec![1]),
            ("day after the first end", d(2026, 11, 11), vec![2]),
            ("second end inclusive", d(2026, 11, 24), vec![2]),
            ("last end inclusive", d(2026, 11, 30), vec![3]),
            ("after the last end", d(2026, 12, 1), vec![]),
        ];
        for (label, on, want) in rows {
            assert_eq!(owner(plain, on), want, "plain: {label}");
        }
        // Attribution is by planned end, not by id order or closing: 12 Nov is id 0's, 13-26 Nov are id 2's.
        let odd = &setups[1].1;
        let rows = [
            ("12 Nov is the first end", d(2026, 11, 12), vec![0]),
            ("13 Nov starts the middle sprint", d(2026, 11, 13), vec![2]),
            ("26 Nov is the middle end", d(2026, 11, 26), vec![2]),
            ("27 Nov starts the last sprint", d(2026, 11, 27), vec![1]),
        ];
        for (label, on, want) in rows {
            assert_eq!(owner(odd, on), want, "odd: {label}");
        }
    }

    #[test]
    fn days_counts_overdue_and_late_days() {
        let on = d(2026, 11, 20);
        let rows = [
            (
                "overdue three days",
                State::Overdue,
                None,
                d(2026, 11, 23),
                Some(3),
            ),
            (
                "late two days",
                State::Late,
                Some("2026-11-22T10:00:00Z"),
                d(2026, 12, 1),
                Some(2),
            ),
            (
                "late counts the UTC day",
                State::Late,
                Some("2026-11-21T00:00:00Z"),
                d(2026, 12, 1),
                Some(1),
            ),
            (
                "late without done_at",
                State::Late,
                None,
                d(2026, 12, 1),
                None,
            ),
            (
                "kept",
                State::Kept,
                Some("2026-11-19T10:00:00Z"),
                d(2026, 12, 1),
                None,
            ),
            ("open", State::Open, None, d(2026, 11, 19), None),
            (
                "dropped",
                State::Dropped,
                Some("2026-11-25T10:00:00Z"),
                d(2026, 12, 1),
                None,
            ),
            (
                "overdue with an impossible negative span",
                State::Overdue,
                None,
                d(2026, 11, 19),
                None,
            ),
        ];
        for (label, st, done_at, today, want) in rows {
            assert_eq!(
                days(st, on, done_at.map(|s| done_on(t(s))), today),
                want,
                "{label}"
            );
        }
    }

    #[test]
    fn window_gives_only_this_and_next_sprint() {
        let rows = [
            ("overdue", Some(State::Overdue), None, NeededIn::ThisSprint),
            (
                "overdue ignores a stray due",
                Some(State::Overdue),
                Some(Due::Later),
                NeededIn::ThisSprint,
            ),
            (
                "open this sprint",
                Some(State::Open),
                Some(Due::ThisSprint),
                NeededIn::ThisSprint,
            ),
            (
                "open next sprint",
                Some(State::Open),
                Some(Due::NextSprint),
                NeededIn::NextSprint,
            ),
            (
                "open later",
                Some(State::Open),
                Some(Due::Later),
                NeededIn::None,
            ),
            ("open without due", Some(State::Open), None, NeededIn::None),
            ("kept", Some(State::Kept), None, NeededIn::None),
            ("late", Some(State::Late), None, NeededIn::None),
            ("dropped", Some(State::Dropped), None, NeededIn::None),
            ("no state", None, None, NeededIn::None),
        ];
        for (label, st, dd, want) in rows {
            assert_eq!(window(st, dd), want, "{label}");
        }
    }

    fn sp(state: Option<State>, due: Option<Due>) -> StoryPromise {
        StoryPromise {
            own: None,
            epic: None,
            on: d(2026, 11, 20),
            holder: Holder::Own,
            state,
            due,
            days: None,
            done_on: None,
        }
    }

    #[test]
    fn matches_each_filter() {
        use Filter::{Any, NextSprint, Overdue, ThisSprint};
        // (label, state, due, [this, next, overdue, any])
        let rows = [
            (
                "open this sprint",
                Some(State::Open),
                Some(Due::ThisSprint),
                [true, false, false, true],
            ),
            (
                "open next sprint",
                Some(State::Open),
                Some(Due::NextSprint),
                [false, true, false, true],
            ),
            (
                "open later",
                Some(State::Open),
                Some(Due::Later),
                [false, false, false, true],
            ),
            (
                "overdue",
                Some(State::Overdue),
                None,
                [true, false, true, true],
            ),
            (
                "kept",
                Some(State::Kept),
                None,
                [false, false, false, false],
            ),
            (
                "late",
                Some(State::Late),
                None,
                [false, false, false, false],
            ),
            (
                "dropped",
                Some(State::Dropped),
                None,
                [false, false, false, false],
            ),
            ("no state", None, None, [false, false, false, false]),
        ];
        for (label, st, dd, want) in rows {
            for (f, name, w) in [
                (ThisSprint, "this_sprint", want[0]),
                (NextSprint, "next_sprint", want[1]),
                (Overdue, "overdue", want[2]),
                (Any, "any", want[3]),
            ] {
                assert_eq!(f.matches(&sp(st, dd)), w, "{label} / {name}");
            }
        }
    }

    #[test]
    fn filters_follow_r5_when_the_active_sprint_ran_past_its_end() {
        // The active sprint was planned to end 20 Nov; today is 25 Nov and it is still open.
        let c = clock(d(2026, 11, 25), Some(d(2026, 11, 20)), None);
        let own = |on| promise(on, "2026-11-01T00:00:00Z", None);
        let rows = [
            (
                "dated today: open, due later",
                d(2026, 11, 25),
                Some(State::Open),
                Some(Due::Later),
                [false, false, false, true],
            ),
            (
                "dated yesterday: overdue",
                d(2026, 11, 24),
                Some(State::Overdue),
                None,
                [true, false, true, true],
            ),
        ];
        for (label, on, st, dd, want) in rows {
            let p = own(on);
            let f = StoryFacts {
                delivery: Todo,
                resolution: None,
                done_at: None,
                own: Some(&p),
                epic: None,
            };
            let got = story_promise(&f, &c).unwrap();
            assert_eq!((got.state, got.due), (st, dd), "{label}");
            for (flt, name, w) in [
                (Filter::ThisSprint, "this_sprint", want[0]),
                (Filter::NextSprint, "next_sprint", want[1]),
                (Filter::Overdue, "overdue", want[2]),
                (Filter::Any, "any", want[3]),
            ] {
                assert_eq!(flt.matches(&got), w, "{label} / {name}");
            }
        }
    }

    #[test]
    fn story_promise_combines_the_rules() {
        let c = clock(
            d(2026, 11, 20),
            Some(d(2026, 11, 27)),
            Some(d(2026, 12, 11)),
        );
        let own = promise(d(2026, 11, 25), "2026-11-01T00:00:00Z", Some("Acme"));
        let epic_early = promise(d(2026, 11, 22), "2026-11-10T12:00:00Z", None);
        let epic_late = promise(d(2026, 12, 8), "2026-11-10T12:00:00Z", None);
        let ek = key("ABC-2");

        // own only
        let f = StoryFacts {
            delivery: Todo,
            resolution: None,
            done_at: None,
            own: Some(&own),
            epic: None,
        };
        let got = story_promise(&f, &c).unwrap();
        assert_eq!(
            (got.on, got.holder, got.state, got.due, got.days),
            (
                own.on(),
                Holder::Own,
                Some(State::Open),
                Some(Due::ThisSprint),
                None
            ),
            "own only"
        );
        assert_eq!(
            (got.own.as_ref(), got.epic.as_ref()),
            (Some(&own), None),
            "own only keeps both sides"
        );

        // inherited only
        let f = StoryFacts {
            delivery: Todo,
            resolution: None,
            done_at: None,
            own: None,
            epic: Some((&ek, &epic_late)),
        };
        let got = story_promise(&f, &c).unwrap();
        assert_eq!(
            (got.on, got.holder, got.state, got.due),
            (
                epic_late.on(),
                Holder::Epic,
                Some(State::Open),
                Some(Due::NextSprint)
            ),
            "inherited only"
        );
        assert_eq!(
            got.epic,
            Some((ek.clone(), epic_late.clone())),
            "inherited only carries the epic"
        );
        assert!(got.own.is_none(), "inherited only has no own");

        // both, epic earlier: both stay visible
        let f = StoryFacts {
            delivery: Todo,
            resolution: None,
            done_at: None,
            own: Some(&own),
            epic: Some((&ek, &epic_early)),
        };
        let got = story_promise(&f, &c).unwrap();
        assert_eq!(
            (got.on, got.holder),
            (epic_early.on(), Holder::Epic),
            "both, epic earlier"
        );
        assert!(
            got.own.is_some() && got.epic.is_some(),
            "both remain visible"
        );

        // both, epic later: own holds
        let f = StoryFacts {
            delivery: Todo,
            resolution: None,
            done_at: None,
            own: Some(&own),
            epic: Some((&ek, &epic_late)),
        };
        let got = story_promise(&f, &c).unwrap();
        assert_eq!(
            (got.on, got.holder),
            (own.on(), Holder::Own),
            "both, epic later"
        );

        // done before the epic promised, no own promise: nothing
        let f = StoryFacts {
            delivery: Done,
            resolution: Some(Resolution::Completed),
            done_at: Some(t("2026-11-09T00:00:00Z")),
            own: None,
            epic: Some((&ek, &epic_early)),
        };
        assert_eq!(story_promise(&f, &c), None, "done before the epic promised");

        // done before the epic promised, own promise: own only, epic dropped from the result
        let f = StoryFacts {
            delivery: Done,
            resolution: Some(Resolution::Completed),
            done_at: Some(t("2026-11-09T00:00:00Z")),
            own: Some(&own),
            epic: Some((&ek, &epic_early)),
        };
        let got = story_promise(&f, &c).unwrap();
        assert_eq!(
            (got.holder, got.state, got.epic.is_none()),
            (Holder::Own, Some(State::Kept), true),
            "done before the epic promised, own kept"
        );

        // done late: days, no due
        let f = StoryFacts {
            delivery: Done,
            resolution: Some(Resolution::Completed),
            done_at: Some(t("2026-11-27T10:00:00Z")),
            own: Some(&own),
            epic: None,
        };
        let got = story_promise(&f, &c).unwrap();
        assert_eq!(
            (got.state, got.days, got.due),
            (Some(State::Late), Some(2), None),
            "done late"
        );

        // overdue: days, no due
        let late_clock = clock(d(2026, 11, 28), None, None);
        let f = StoryFacts {
            delivery: InProgress,
            resolution: None,
            done_at: None,
            own: Some(&own),
            epic: None,
        };
        let got = story_promise(&f, &late_clock).unwrap();
        assert_eq!(
            (got.state, got.days, got.due),
            (Some(State::Overdue), Some(3), None),
            "overdue"
        );

        // own 25 Nov, epic 22 Nov: state, due and days follow the effective (epic) date
        let f = StoryFacts {
            delivery: InProgress,
            resolution: None,
            done_at: None,
            own: Some(&own),
            epic: Some((&ek, &epic_early)),
        };
        let got = story_promise(&f, &clock(d(2026, 11, 23), None, None)).unwrap();
        assert_eq!(
            (got.holder, got.state, got.due, got.days),
            (Holder::Epic, Some(State::Overdue), None, Some(1)),
            "effective date drives overdue"
        );
        let f = StoryFacts {
            delivery: Done,
            resolution: Some(Resolution::Completed),
            done_at: Some(t("2026-11-24T23:59:59Z")),
            own: Some(&own),
            epic: Some((&ek, &epic_early)),
        };
        let got = story_promise(&f, &c).unwrap();
        assert_eq!(
            (got.state, got.days, got.done_on),
            (Some(State::Late), Some(2), Some(d(2026, 11, 24))),
            "effective date drives late"
        );
        let f = StoryFacts {
            delivery: Done,
            resolution: Some(Resolution::Completed),
            done_at: Some(t("2026-11-22T23:59:59Z")),
            own: Some(&own),
            epic: Some((&ek, &epic_early)),
        };
        let got = story_promise(&f, &c).unwrap();
        assert_eq!(
            (got.state, got.days),
            (Some(State::Kept), None),
            "done 23:59:59Z on the epic's date is kept"
        );

        // done without done_at, own promise: present but stateless
        let f = StoryFacts {
            delivery: Done,
            resolution: Some(Resolution::Dropped),
            done_at: None,
            own: Some(&own),
            epic: None,
        };
        let got = story_promise(&f, &c).unwrap();
        assert_eq!(
            (got.state, got.due, got.days),
            (None, None, None),
            "done without done_at"
        );

        // no promise at all
        let f = StoryFacts {
            delivery: Todo,
            resolution: None,
            done_at: None,
            own: None,
            epic: None,
        };
        assert_eq!(story_promise(&f, &c), None, "no promise");
    }

    #[test]
    fn epic_due_has_a_period_until_it_is_past() {
        let c = clock(
            d(2026, 11, 20),
            Some(d(2026, 11, 27)),
            Some(d(2026, 12, 11)),
        );
        let rows = [
            ("on today", d(2026, 11, 20), Some(Due::ThisSprint)),
            ("yesterday", d(2026, 11, 19), None),
            ("next sprint", d(2026, 12, 1), Some(Due::NextSprint)),
            ("later", d(2027, 1, 1), Some(Due::Later)),
        ];
        for (label, on, want) in rows {
            assert_eq!(
                epic_due(&promise(on, "2026-11-01T00:00:00Z", None), &c),
                want,
                "{label}"
            );
        }
    }

    #[test]
    fn errors_read_as_the_message_the_person_sees() {
        let long = format!("the recipient is longer than {MAX_RECIPIENT} characters");
        let rows = [
            (
                "done",
                PromiseError::Done(key("ABC-7")),
                "ABC-7 is done: a done story cannot be promised",
            ),
            (
                "past",
                PromiseError::Past(d(2026, 11, 19)),
                "the date 2026-11-19 is past: promise today or later",
            ),
            ("long", PromiseError::RecipientTooLong, &long),
            (
                "nul",
                PromiseError::RecipientHasNul,
                "the recipient cannot contain a NUL character",
            ),
            (
                "no active",
                PromiseError::NoSprint(Shortcut::ThisSprint),
                "there is no active sprint: give a date",
            ),
            (
                "no draft",
                PromiseError::NoSprint(Shortcut::NextSprint),
                "there is no next sprint yet: give a date",
            ),
            (
                "active passed",
                PromiseError::SprintEndPassed(Shortcut::ThisSprint, d(2026, 11, 19)),
                "the active sprint is planned to end 2026-11-19, which is past: give a date",
            ),
            (
                "draft passed",
                PromiseError::SprintEndPassed(Shortcut::NextSprint, d(2026, 11, 19)),
                "the next sprint is planned to end 2026-11-19, which is past: give a date",
            ),
            (
                "both",
                PromiseError::Both,
                "give a date or a sprint shortcut, not both",
            ),
            (
                "neither",
                PromiseError::Neither,
                "give a date or a sprint shortcut",
            ),
        ];
        for (label, e, want) in rows {
            assert_eq!(e.to_string(), want, "{label}");
        }
    }
}
