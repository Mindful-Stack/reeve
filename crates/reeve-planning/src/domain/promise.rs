//! Promises (kb/ADR-0025): a date by which a story or an epic will be done. Pure: dates and facts in, state out.
//! Nothing here reads the clock or the database; `today` is the handler's `c.now` as a UTC date.
use super::sprint::Sprint;
use chrono::{DateTime, NaiveDate, Utc};
use reeve_protocol::{DeliveryState, Key, NeededIn, Resolution, SprintState};

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
    #[error("{0} has no promise to change: give a date")]
    Neither(Key),
}

/// Which kind of item a promise action names (R1): a story or an epic, never an initiative. What the port is asked
/// for before the row is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromiseItem {
    Story,
    Epic,
}

impl PromiseItem {
    /// R1: None for an initiative, which is never promised.
    pub fn of(item: reeve_protocol::ItemType) -> Option<PromiseItem> {
        match item {
            reeve_protocol::ItemType::Story => Some(PromiseItem::Story),
            reeve_protocol::ItemType::Epic => Some(PromiseItem::Epic),
            reeve_protocol::ItemType::Initiative => None,
        }
    }
}

/// Who may be promised (R1, R2), as read under the row lock: a story with its delivery state, or an epic, which has
/// none. `change` refuses a done story.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Promisable {
    Story(DeliveryState),
    Epic,
}

/// Today and the two planned ends, built once per snapshot by `Clock::at`. `today` is the request's UTC date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    pub today: NaiveDate,
    pub active_ends: Option<NaiveDate>,
    pub draft_ends: Option<NaiveDate>,
}

impl Clock {
    /// The one assembly of a clock: today is the UTC date of `now`, and the active and the draft sprint's planned ends
    /// come from `sprints` (closed sprints are ignored, a missing one gives None). Callers pass the sprints read on
    /// their snapshot: every sprint, or only the active and the draft.
    pub fn at(now: DateTime<Utc>, sprints: &[Sprint]) -> Clock {
        let ends = |st: SprintState| sprints.iter().find(|s| s.state == st).map(|s| s.ends_on);
        Clock {
            today: now.date_naive(),
            active_ends: ends(SprintState::Active),
            draft_ends: ends(SprintState::Draft),
        }
    }
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

/// R2 and the change rules: the promise to store on `key`, given what it is and its current promise (if any), or None
/// when nothing changes (the same date and the same recipient: a retry or a double submit writes and logs nothing, and
/// the promise keeps its `by`).
/// - A done story cannot be promised (R2; dropped included: dropped is a resolution of done).
/// - A date or a shortcut, not both. Neither: change only the recipient of the current promise (refused without one).
/// - A new date before today is refused; keeping the current date is not (an overdue promise's recipient can change).
/// - Keeping the date keeps `at` (R3's boundary does not move) and records the editor in `by`; a new date sets both.
/// - `to` None keeps the current recipient; blank clears it.
///
/// `at` is the request's timestamp; the dates are judged against `clock.today` only. The caller's contract is that
/// the clock was built for that same request (`Clock::at(at, ..)`), so `clock.today == at.date_naive()`.
pub fn change(
    key: &Key,
    item: Promisable,
    current: Option<&Promise>,
    req: Request,
    by: &str,
    at: DateTime<Utc>,
    clock: &Clock,
) -> Result<Option<Promise>, PromiseError> {
    if item == Promisable::Story(DeliveryState::Done) {
        return Err(PromiseError::Done(key.clone()));
    }
    let on = match (req.on, req.shortcut) {
        (Some(_), Some(_)) => return Err(PromiseError::Both),
        (Some(d), None) => d,
        (None, Some(s)) => shortcut(s, clock)?,
        (None, None) => {
            current
                .ok_or_else(|| PromiseError::Neither(key.clone()))?
                .on
        }
    };
    let kept = current.filter(|c| c.on == on);
    if kept.is_none() && on < clock.today {
        return Err(PromiseError::Past(on));
    }
    let to = match req.to {
        None => current.and_then(|c| c.to.clone()),
        Some(t) => clean_to(t)?,
    };
    if kept.is_some_and(|c| c.to == to) {
        return Ok(None);
    }
    Ok(Some(Promise {
        on,
        by: by.to_string(),
        at: kept.map_or(at, |c| c.at),
        to,
    }))
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

/// Whose promise sets a story's effective date: the story's own, or the epic's, named by its key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeldBy {
    Own,
    Epic(Key),
}

/// The promises a story shows (R3: both remain visible), arranged by which one holds. A holder without its promise
/// cannot be built.
#[derive(Debug, Clone, PartialEq)]
enum Held {
    /// The story's own promise sets the date; `epic` is the inherited one when the story also has it (a later date, or
    /// the same: the story's own wins a tie).
    Own {
        own: Promise,
        epic: Option<(Key, Promise)>,
    },
    /// The inherited epic promise sets the date; `own` is the story's own when it has one (a later date).
    Epic {
        epic: (Key, Promise),
        own: Option<Promise>,
    },
}

/// R3: the earlier date holds; the story's own wins a tie, so its recipient is the one shown. `epic` is the promise
/// the story inherits (already filtered by `inherits`).
fn effective(own: Option<Promise>, epic: Option<(Key, Promise)>) -> Option<Held> {
    match (own, epic) {
        (Some(o), Some(e)) if e.1.on < o.on => Some(Held::Epic {
            epic: e,
            own: Some(o),
        }),
        (Some(o), e) => Some(Held::Own { own: o, epic: e }),
        (None, Some(e)) => Some(Held::Epic { epic: e, own: None }),
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
fn previous_end(this: (i64, NaiveDate), others: &[(i64, NaiveDate)]) -> Option<NaiveDate> {
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

/// R8: a sprint's promise range, `(after, upto)`: after the previous sprint's planned end (exclusive; None for the
/// first sprint) and up to this sprint's own planned end (inclusive). The previous sprint is chosen by
/// `previous_end` among the other non-draft sprints in `all`. A draft has no range (None): its promises are not yet
/// a sprint's. `all` is every sprint, read on the caller's snapshot; `this` may or may not be among them.
pub fn sprint_range(this: &Sprint, all: &[Sprint]) -> Option<(Option<NaiveDate>, NaiveDate)> {
    if this.state == SprintState::Draft {
        return None;
    }
    let others: Vec<(i64, NaiveDate)> = all
        .iter()
        .filter(|o| o.id != this.id && o.state != SprintState::Draft)
        .map(|o| (o.id, o.ends_on))
        .collect();
    Some((previous_end((this.id, this.ends_on), &others), this.ends_on))
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
            Filter::ThisSprint => sp.window() == NeededIn::ThisSprint,
            Filter::NextSprint => sp.window() == NeededIn::NextSprint,
            Filter::Overdue => sp.state == Some(State::Overdue),
            Filter::Any => matches!(sp.state, Some(State::Open | State::Overdue)),
        }
    }
}

/// Everything a reader shows about one story's promise, computed once (R3-R5). The fields are private:
/// `story_promise` is the only constructor, so the holder always comes with its promise.
#[derive(Debug, Clone, PartialEq)]
pub struct StoryPromise {
    held: Held,
    state: Option<State>,
    /// Only for `Open`.
    due: Option<Due>,
    days: Option<u32>,
    /// The UTC day of `done_at`, when the story is done with one.
    done_on: Option<NaiveDate>,
}

impl StoryPromise {
    /// The story's own promise, whether or not it holds.
    pub fn own(&self) -> Option<&Promise> {
        match &self.held {
            Held::Own { own, .. } => Some(own),
            Held::Epic { own, .. } => own.as_ref(),
        }
    }
    /// The epic's key and promise, only when the story inherits it (R3), whether or not it holds.
    pub fn epic(&self) -> Option<(&Key, &Promise)> {
        match &self.held {
            Held::Own { epic, .. } => epic.as_ref().map(|(k, p)| (k, p)),
            Held::Epic { epic, .. } => Some((&epic.0, &epic.1)),
        }
    }
    /// Whose promise sets the effective date.
    pub fn held_by(&self) -> HeldBy {
        match &self.held {
            Held::Own { .. } => HeldBy::Own,
            Held::Epic { epic, .. } => HeldBy::Epic(epic.0.clone()),
        }
    }
    /// The promise that sets the effective date.
    pub fn holding(&self) -> &Promise {
        match &self.held {
            Held::Own { own, .. } => own,
            Held::Epic { epic, .. } => &epic.1,
        }
    }
    /// The effective date (R3).
    pub fn on(&self) -> NaiveDate {
        self.holding().on
    }
    /// None only for a done story without `done_at`.
    pub fn state(&self) -> Option<State> {
        self.state
    }
    /// The due period, only while open.
    pub fn due(&self) -> Option<Due> {
        self.due
    }
    /// Days overdue or late.
    pub fn days(&self) -> Option<u32> {
        self.days
    }
    /// The UTC day of `done_at`, when the story is done with one.
    pub fn done_on(&self) -> Option<NaiveDate> {
        self.done_on
    }
    /// R7: the decision window this promise asks for.
    pub fn window(&self) -> NeededIn {
        window(self.state, self.due)
    }
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
        .filter(|(_, p)| inherits(p.at, f.delivery, f.done_at))
        .map(|(k, p)| (k.clone(), p.clone()));
    let held = effective(f.own.cloned(), epic)?;
    let on = match &held {
        Held::Own { own, .. } => own.on,
        Held::Epic { epic, .. } => epic.1.on,
    };
    let done = f.done_at.map(done_on);
    let st = state(on, f.delivery, f.resolution, done, clock.today);
    let d = (st == Some(State::Open)).then(|| due(on, clock));
    Some(StoryPromise {
        held,
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

/// One row of a sprint's promise list (R8). The fields are private: `SprintPromise::listed` is the only constructor, so
/// a row always has a state and always falls in its sprint.
#[derive(Debug, Clone, PartialEq)]
pub struct SprintPromise {
    key: Key,
    title: String,
    on: NaiveDate,
    held_by: HeldBy,
    to: Option<String>,
    state: State,
    done_on: Option<NaiveDate>,
    days: Option<u32>,
}

impl SprintPromise {
    /// R8: the row a story's promise gives the sprint planned to end `ends` whose previous sprint was planned to end
    /// `previous_ends`; None when the effective date is outside that range or the promise has no state (a done story
    /// without `done_at`).
    pub fn listed(
        key: Key,
        title: String,
        sp: StoryPromise,
        previous_ends: Option<NaiveDate>,
        ends: NaiveDate,
    ) -> Option<SprintPromise> {
        let state = sp.state?;
        if !in_sprint(sp.on(), previous_ends, ends) {
            return None;
        }
        Some(SprintPromise {
            key,
            title,
            on: sp.on(),
            held_by: sp.held_by(),
            to: sp.holding().to.clone(),
            state,
            done_on: sp.done_on,
            days: sp.days,
        })
    }
    pub fn key(&self) -> &Key {
        &self.key
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    /// The effective date (R3).
    pub fn on(&self) -> NaiveDate {
        self.on
    }
    pub fn held_by(&self) -> &HeldBy {
        &self.held_by
    }
    /// The recipient of the promise that sets `on`.
    pub fn to(&self) -> Option<&str> {
        self.to.as_deref()
    }
    pub fn state(&self) -> State {
        self.state
    }
    /// The UTC day the story was done, when it was.
    pub fn done_on(&self) -> Option<NaiveDate> {
        self.done_on
    }
    /// Days overdue or late.
    pub fn days(&self) -> Option<u32> {
        self.days
    }
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
    fn sprint(id: i64, state: SprintState, ends_on: NaiveDate) -> Sprint {
        Sprint {
            id,
            state,
            starts_on: d(2026, 10, 1),
            ends_on,
            opened_at: None,
            closed_at: None,
            version: 1,
        }
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
        let ek = key("ABC-2");
        let p = |on| promise(on, "2026-11-01T00:00:00Z", None);
        let epic = |on| Some((ek.clone(), p(on)));
        let rows = [
            (
                "epic earlier",
                Some(b),
                epic(a),
                Some((a, HeldBy::Epic(ek.clone()))),
            ),
            ("epic later", Some(a), epic(b), Some((a, HeldBy::Own))),
            ("equal", Some(a), epic(a), Some((a, HeldBy::Own))),
            ("own only", Some(a), None, Some((a, HeldBy::Own))),
            (
                "epic only",
                None,
                epic(a),
                Some((a, HeldBy::Epic(ek.clone()))),
            ),
            ("neither", None, None, None),
        ];
        for (label, own, epic, want) in rows {
            let got = effective(own.map(p), epic).map(|held| {
                let sp = StoryPromise {
                    held,
                    state: None,
                    due: None,
                    days: None,
                    done_on: None,
                };
                (sp.on(), sp.held_by())
            });
            assert_eq!(got, want, "{label}");
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

    /// R2: of the promisable items only a done story is refused, whatever the request; an epic has no delivery.
    #[test]
    fn change_refuses_only_done_stories() {
        let k = key("ABC-7");
        let now = t("2026-11-20T09:00:00Z");
        let c = clock(d(2026, 11, 20), None, None);
        let on = d(2026, 11, 25);
        let rows = [
            ("todo", Promisable::Story(Todo), true),
            ("in progress", Promisable::Story(InProgress), true),
            ("in review", Promisable::Story(InReview), true),
            ("testing", Promisable::Story(Testing), true),
            ("epic", Promisable::Epic, true),
            ("done", Promisable::Story(Done), false),
        ];
        for (label, item, ok) in rows {
            let req = Request {
                on: Some(on),
                shortcut: None,
                to: None,
            };
            let got = change(&k, item, None, req, "ed@x.y", now, &c);
            let want = if ok {
                Ok(Some(Promise::from_persisted(
                    on,
                    "ed@x.y".into(),
                    now,
                    None,
                )))
            } else {
                Err(PromiseError::Done(k.clone()))
            };
            assert_eq!(got, want, "{label}");
        }
        // Done is checked before anything else: even an empty request on a done story reads as done.
        let empty = Request {
            on: None,
            shortcut: None,
            to: None,
        };
        assert_eq!(
            change(&k, Promisable::Story(Done), None, empty, "ed@x.y", now, &c),
            Err(PromiseError::Done(k.clone()))
        );
    }

    /// `Clock::at` is the one clock assembly: the active and the draft sprint's planned ends, closed sprints ignored
    /// whatever their ends, None for a missing one; today is the UTC date of `now`.
    #[test]
    fn clock_at_takes_the_active_and_draft_ends_only() {
        let now = t("2026-11-12T23:30:00Z");
        let all = vec![
            sprint(1, SprintState::Closed, d(2026, 11, 30)),
            sprint(2, SprintState::Active, d(2026, 11, 24)),
            sprint(3, SprintState::Closed, d(2026, 12, 20)),
            sprint(4, SprintState::Draft, d(2026, 12, 8)),
        ];
        assert_eq!(
            Clock::at(now, &all),
            clock(d(2026, 11, 12), Some(d(2026, 11, 24)), Some(d(2026, 12, 8)))
        );
        let closed_only = vec![
            sprint(1, SprintState::Closed, d(2026, 11, 30)),
            sprint(3, SprintState::Closed, d(2026, 12, 20)),
        ];
        assert_eq!(
            Clock::at(now, &closed_only),
            clock(d(2026, 11, 12), None, None)
        );
        assert_eq!(
            Clock::at(now, &[sprint(4, SprintState::Draft, d(2026, 12, 8))]),
            clock(d(2026, 11, 12), None, Some(d(2026, 12, 8)))
        );
    }

    /// R8's range: a draft has none; otherwise after the previous non-draft planned end (the draft never counts as the
    /// previous sprint, whatever its end) and up to this sprint's own.
    #[test]
    fn sprint_range_follows_r8_and_skips_the_draft() {
        let all = vec![
            sprint(1, SprintState::Closed, d(2026, 11, 10)),
            sprint(2, SprintState::Closed, d(2026, 11, 24)),
            sprint(3, SprintState::Active, d(2026, 12, 8)),
            // A draft planned to end before the active sprint: still not its previous sprint.
            sprint(4, SprintState::Draft, d(2026, 12, 1)),
        ];
        let rows = [
            ("first sprint", 0, Some((None, d(2026, 11, 10)))),
            (
                "closed after the first",
                1,
                Some((Some(d(2026, 11, 10)), d(2026, 11, 24))),
            ),
            (
                "active: the draft is skipped",
                2,
                Some((Some(d(2026, 11, 24)), d(2026, 12, 8))),
            ),
            ("draft: no range", 3, None),
        ];
        for (label, i, want) in rows {
            assert_eq!(sprint_range(&all[i], &all), want, "{label}");
        }
        // `this` need not be among `all`.
        assert_eq!(
            sprint_range(&all[2], &all[..2]),
            Some((Some(d(2026, 11, 24)), d(2026, 12, 8)))
        );
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
        let k = key("ABC-7");
        #[allow(clippy::type_complexity)]
        let rows: Vec<(
            &str,
            Option<Promise>,
            Request,
            Clock,
            Result<Option<(NaiveDate, &str, DateTime<Utc>, Option<String>)>, PromiseError>,
        )> = vec![
            (
                "new promise with a date",
                None,
                req(Some(d(2026, 11, 25)), None, None),
                std,
                Ok(Some((d(2026, 11, 25), "ed@x.y", now, None))),
            ),
            (
                "new promise dated today",
                None,
                req(Some(today), None, Some("Acme")),
                std,
                Ok(Some((today, "ed@x.y", now, Some("Acme".into())))),
            ),
            (
                "new promise with neither",
                None,
                req(None, None, Some("Acme")),
                std,
                Err(PromiseError::Neither(k.clone())),
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
                Ok(Some((
                    overdue.on(),
                    "ed@x.y",
                    overdue.at(),
                    Some("Bolt".into()),
                ))),
            ),
            (
                "overdue, no date, recipient cleared",
                Some(overdue.clone()),
                req(None, None, Some("")),
                std,
                Ok(Some((overdue.on(), "ed@x.y", overdue.at(), None))),
            ),
            (
                "overdue, no date, recipient blank spaces clears",
                Some(overdue.clone()),
                req(None, None, Some("   ")),
                std,
                Ok(Some((overdue.on(), "ed@x.y", overdue.at(), None))),
            ),
            (
                "same date, to omitted: nothing changes",
                Some(future.clone()),
                req(Some(future.on()), None, None),
                std,
                Ok(None),
            ),
            (
                "same date and the same recipient: nothing changes",
                Some(future.clone()),
                req(Some(future.on()), None, Some(" Acme ")),
                std,
                Ok(None),
            ),
            (
                "neither, to omitted: nothing changes",
                Some(future.clone()),
                req(None, None, None),
                std,
                Ok(None),
            ),
            (
                "same past date, same recipient: nothing changes, not refused as past",
                Some(overdue.clone()),
                req(Some(overdue.on()), None, Some("Acme")),
                std,
                Ok(None),
            ),
            (
                "neither, new recipient: date and at kept, by is the editor",
                Some(future.clone()),
                req(None, None, Some("Bolt")),
                std,
                Ok(Some((
                    future.on(),
                    "ed@x.y",
                    future.at(),
                    Some("Bolt".into()),
                ))),
            ),
            (
                "a new later date moves at",
                Some(future.clone()),
                req(Some(d(2026, 12, 5)), None, None),
                std,
                Ok(Some((d(2026, 12, 5), "ed@x.y", now, Some("Acme".into())))),
            ),
            (
                "a new earlier future date moves at",
                Some(future.clone()),
                req(Some(d(2026, 11, 22)), None, Some("Bolt")),
                std,
                Ok(Some((d(2026, 11, 22), "ed@x.y", now, Some("Bolt".into())))),
            ),
            (
                "a new date on an overdue promise moves at",
                Some(overdue.clone()),
                req(Some(d(2026, 11, 21)), None, None),
                std,
                Ok(Some((d(2026, 11, 21), "ed@x.y", now, Some("Acme".into())))),
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
                Ok(Some((d(2026, 12, 11), "ed@x.y", now, None))),
            ),
            (
                "shortcut to the same end as the current keeps at",
                Some(promise(d(2026, 11, 27), "2026-11-03T00:00:00Z", None)),
                req(None, Some(Shortcut::ThisSprint), Some("Acme")),
                std,
                Ok(Some((
                    d(2026, 11, 27),
                    "ed@x.y",
                    t("2026-11-03T00:00:00Z"),
                    Some("Acme".into()),
                ))),
            ),
            (
                "shortcut to the same end, nothing else: nothing changes",
                Some(promise(d(2026, 11, 27), "2026-11-03T00:00:00Z", None)),
                req(None, Some(Shortcut::ThisSprint), None),
                std,
                Ok(None),
            ),
            (
                "200 characters of two bytes",
                None,
                req(Some(d(2026, 11, 25)), None, Some(&long_ok)),
                std,
                Ok(Some((
                    d(2026, 11, 25),
                    "ed@x.y",
                    now,
                    Some(long_ok.clone()),
                ))),
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
                Ok(Some((
                    d(2026, 11, 25),
                    "ed@x.y",
                    now,
                    Some(long_ok.clone()),
                ))),
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
                Ok(Some((d(2026, 11, 25), "ed@x.y", now, Some("Acme".into())))),
            ),
        ];
        for (label, current, request, c, want) in rows {
            let got = change(
                &k,
                Promisable::Story(Todo),
                current.as_ref(),
                request,
                "ed@x.y",
                now,
                &c,
            );
            let want = want
                .map(|w| w.map(|(on, by, at, to)| Promise::from_persisted(on, by.into(), at, to)));
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
            held: Held::Own {
                own: promise(d(2026, 11, 20), "2026-11-01T00:00:00Z", None),
                epic: None,
            },
            state,
            due,
            days: None,
            done_on: None,
        }
    }

    /// The filter's truth table is `web/src/lib/promise.fixture.json`'s `filter` rows, which the web's
    /// `promisedMatches` reads too (the board filters on the client, the backlog asks the server), so the two cannot
    /// drift. States and periods are spelled as on the wire.
    #[test]
    fn matches_the_fixture_filter_rows() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../../web/src/lib/promise.fixture.json"))
                .unwrap();
        let rows = fixture["filter"].as_array().unwrap();
        assert!(!rows.is_empty());
        let state = |s: &str| match s {
            "open" => State::Open,
            "overdue" => State::Overdue,
            "kept" => State::Kept,
            "late" => State::Late,
            "dropped" => State::Dropped,
            other => panic!("unknown state {other}"),
        };
        let due = |s: &str| match s {
            "this_sprint" => Due::ThisSprint,
            "next_sprint" => Due::NextSprint,
            "later" => Due::Later,
            other => panic!("unknown due {other}"),
        };
        for r in rows {
            let label = r["label"].as_str().unwrap();
            let p = sp(r["state"].as_str().map(state), r["due"].as_str().map(due));
            let matches = r["matches"].as_object().unwrap();
            let mut names: Vec<&str> = matches.keys().map(String::as_str).collect();
            names.sort_unstable();
            assert_eq!(
                names,
                ["any", "next_sprint", "overdue", "this_sprint"],
                "{label}: one column per filter"
            );
            for (f, name) in [
                (Filter::ThisSprint, "this_sprint"),
                (Filter::NextSprint, "next_sprint"),
                (Filter::Overdue, "overdue"),
                (Filter::Any, "any"),
            ] {
                assert_eq!(f.matches(&p), matches[name], "{label} / {name}");
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
            assert_eq!((got.state(), got.due()), (st, dd), "{label}");
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
            (got.on(), got.held_by(), got.state(), got.due(), got.days()),
            (
                own.on(),
                HeldBy::Own,
                Some(State::Open),
                Some(Due::ThisSprint),
                None
            ),
            "own only"
        );
        assert_eq!(
            (got.own(), got.epic()),
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
            (got.on(), got.held_by(), got.state(), got.due()),
            (
                epic_late.on(),
                HeldBy::Epic(ek.clone()),
                Some(State::Open),
                Some(Due::NextSprint)
            ),
            "inherited only"
        );
        assert_eq!(
            got.epic(),
            Some((&ek, &epic_late)),
            "inherited only carries the epic"
        );
        assert!(got.own().is_none(), "inherited only has no own");

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
            (got.on(), got.held_by()),
            (epic_early.on(), HeldBy::Epic(ek.clone())),
            "both, epic earlier"
        );
        assert!(
            got.own().is_some() && got.epic().is_some(),
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
            (got.on(), got.held_by()),
            (own.on(), HeldBy::Own),
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
            (got.held_by(), got.state(), got.epic().is_none()),
            (HeldBy::Own, Some(State::Kept), true),
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
            (got.state(), got.days(), got.due()),
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
            (got.state(), got.days(), got.due()),
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
            (got.held_by(), got.state(), got.due(), got.days()),
            (
                HeldBy::Epic(ek.clone()),
                Some(State::Overdue),
                None,
                Some(1)
            ),
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
            (got.state(), got.days(), got.done_on()),
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
            (got.state(), got.days()),
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
            (got.state(), got.due(), got.days()),
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
                PromiseError::Neither(key("ABC-7")),
                "ABC-7 has no promise to change: give a date",
            ),
        ];
        for (label, e, want) in rows {
            assert_eq!(e.to_string(), want, "{label}");
        }
    }
    #[test]
    fn listed_keeps_a_stated_row_in_its_sprint_and_names_the_holder() {
        let ek = key("ABC-7");
        let own = promise(d(2026, 11, 22), "2026-11-01T09:00:00Z", Some("Acme"));
        let epic = promise(d(2026, 11, 18), "2026-11-01T09:00:00Z", Some("Beta"));
        let later_epic = promise(d(2026, 11, 25), "2026-11-01T09:00:00Z", Some("Beta"));
        let own_at_end = promise(d(2026, 11, 24), "2026-11-01T09:00:00Z", Some("Acme"));
        let today = clock(d(2026, 11, 26), None, None);
        let facts = |delivery, done_at: Option<&str>, own, epic| StoryFacts {
            delivery,
            resolution: (delivery == Done).then_some(Resolution::Completed),
            done_at: done_at.map(t),
            own,
            epic,
        };
        let sp = |f: StoryFacts<'_>| story_promise(&f, &today).unwrap();
        let prev = Some(d(2026, 11, 10));
        let ends = d(2026, 11, 24);
        type Want<'a> = Option<(NaiveDate, HeldBy, Option<&'a str>, State)>;
        let rows: Vec<(&str, StoryPromise, Option<NaiveDate>, Want<'_>)> = vec![
            (
                "epic holds: its key and recipient",
                sp(facts(Todo, None, Some(&own), Some((&ek, &epic)))),
                prev,
                Some((
                    d(2026, 11, 18),
                    HeldBy::Epic(ek.clone()),
                    Some("Beta"),
                    State::Overdue,
                )),
            ),
            (
                "own holds beside a later epic: the own recipient",
                sp(facts(Todo, None, Some(&own), Some((&ek, &later_epic)))),
                prev,
                Some((d(2026, 11, 22), HeldBy::Own, Some("Acme"), State::Overdue)),
            ),
            (
                "on this end: inclusive",
                sp(facts(
                    Done,
                    Some("2026-11-21T10:00:00Z"),
                    Some(&own_at_end),
                    None,
                )),
                prev,
                Some((d(2026, 11, 24), HeldBy::Own, Some("Acme"), State::Kept)),
            ),
            (
                "on the previous end: not this sprint's",
                sp(facts(Todo, None, Some(&own), None)),
                Some(d(2026, 11, 22)),
                None,
            ),
            (
                "after this end: not this sprint's",
                sp(facts(Todo, None, None, Some((&ek, &later_epic)))),
                prev,
                None,
            ),
            (
                "done without done_at: no state, no row",
                sp(facts(Done, None, Some(&own), None)),
                prev,
                None,
            ),
        ];
        for (label, sp, previous, want) in rows {
            let got = SprintPromise::listed(key("ABC-9"), "T".into(), sp, previous, ends)
                .map(|r| (r.on, r.held_by, r.to, r.state));
            let want = want.map(|(on, h, to, st)| (on, h, to.map(str::to_string), st));
            assert_eq!(got, want, "{label}");
        }
    }
}
