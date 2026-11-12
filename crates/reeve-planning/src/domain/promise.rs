//! Promises (kb/ADR-0025): a date by which a story or an epic will be done. Pure: dates and facts in, state out.
//! Nothing here reads the clock or the database; `today` is the handler's `c.now` as a UTC date.
use chrono::{DateTime, NaiveDate, Utc};
use reeve_protocol::{DeliveryState, Key, NeededIn, Resolution};

pub const MAX_RECIPIENT: usize = 200;

/// A stored promise (R1). `to` is trimmed and never blank. `at` is when the current date was promised (R3's boundary).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Promise {
    pub on: NaiveDate,
    pub by: String,
    pub at: DateTime<Utc>,
    pub to: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    ThisSprint,
    NextSprint,
}

/// What the person asked for: a date, a sprint shortcut or neither (recipient only), and the recipient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub on: Option<NaiveDate>,
    pub shortcut: Option<Shortcut>,
    /// None: keep the current recipient; Some(""): clear it.
    pub to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromiseError {
    /// The story is done (R2).
    Done(Key),
    /// A new date before today (R2).
    Past(NaiveDate),
    RecipientTooLong,
    /// A shortcut whose sprint does not exist (R6).
    NoSprint(Shortcut),
    /// A shortcut whose planned end is before today (R6).
    SprintEndPassed(Shortcut, NaiveDate),
    /// Both a date and a shortcut.
    Both,
    /// Neither, and no current promise whose recipient could change.
    Neither,
}

/// R2: a done story (dropped included: dropped is a resolution of done) cannot be promised.
pub fn may_promise(key: &Key, delivery: DeliveryState) -> Result<(), PromiseError> {
    if delivery == DeliveryState::Done {
        return Err(PromiseError::Done(key.clone()));
    }
    Ok(())
}

/// Today and the two planned ends, read once per snapshot.
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
    Ok(to)
}

/// R2 and the change rules: the promise to store, given the current one (if any).
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

/// R4. A done story without `done_at` has no state, dropped or not; otherwise `done_at` counts by its UTC day.
pub fn state(
    on: NaiveDate,
    delivery: DeliveryState,
    resolution: Option<Resolution>,
    done_at: Option<DateTime<Utc>>,
    today: NaiveDate,
) -> Option<State> {
    if delivery != DeliveryState::Done {
        return Some(if today > on {
            State::Overdue
        } else {
            State::Open
        });
    }
    let done_on = done_at?.date_naive();
    if resolution == Some(Resolution::Dropped) {
        return Some(State::Dropped);
    }
    Some(if done_on <= on {
        State::Kept
    } else {
        State::Late
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    ThisSprint,
    NextSprint,
    Later,
}

/// R5, for open promises (and an epic's promise that is not past). Bounds inclusive; a missing sprint never matches.
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
pub fn days(
    st: State,
    on: NaiveDate,
    done_at: Option<DateTime<Utc>>,
    today: NaiveDate,
) -> Option<u32> {
    let diff = |a: NaiveDate| u32::try_from((a - on).num_days()).ok();
    match st {
        State::Overdue => diff(today),
        State::Late => done_at.and_then(|d| diff(d.date_naive())),
        _ => None,
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

/// R7: the queue lane an unfinished story's promise asks for; only this sprint and next sprint.
pub fn lane(st: Option<State>, d: Option<Due>) -> NeededIn {
    match (st, d) {
        (Some(State::Overdue), _) | (Some(State::Open), Some(Due::ThisSprint)) => {
            NeededIn::ThisSprint
        }
        (Some(State::Open), Some(Due::NextSprint)) => NeededIn::NextSprint,
        _ => NeededIn::None,
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

pub fn matches(f: Filter, sp: &StoryPromise) -> bool {
    match f {
        Filter::ThisSprint => lane(sp.state, sp.due) == NeededIn::ThisSprint,
        Filter::NextSprint => lane(sp.state, sp.due) == NeededIn::NextSprint,
        Filter::Overdue => sp.state == Some(State::Overdue),
        Filter::Any => matches!(sp.state, Some(State::Open | State::Overdue)),
    }
}

/// Everything a reader shows about one story's promise, computed once (R3–R5).
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
}

pub struct StoryFacts<'a> {
    pub delivery: DeliveryState,
    pub resolution: Option<Resolution>,
    pub done_at: Option<DateTime<Utc>>,
    pub own: Option<&'a Promise>,
    pub epic: Option<(&'a Key, &'a Promise)>,
}

pub fn story_promise(f: &StoryFacts<'_>, clock: &Clock) -> Option<StoryPromise> {
    let epic = f
        .epic
        .filter(|(_, p)| inherits(p.at, f.delivery, f.done_at));
    let (on, holder) = effective(f.own.map(|p| p.on), epic.map(|(_, p)| p.on))?;
    let st = state(on, f.delivery, f.resolution, f.done_at, clock.today);
    let d = (st == Some(State::Open)).then(|| due(on, clock));
    Some(StoryPromise {
        own: f.own.cloned(),
        epic: epic.map(|(k, p)| (k.clone(), p.clone())),
        on,
        holder,
        state: st,
        due: d,
        days: st.and_then(|s| days(s, on, f.done_at, clock.today)),
    })
}

/// An epic's own promise: no state (R4); its due period while not past, and whether it is past.
pub fn epic_due(p: &Promise, clock: &Clock) -> (Option<Due>, bool) {
    let past = clock.today > p.on;
    ((!past).then(|| due(p.on, clock)), past)
}

/// One row of a sprint's promise list (R8), built by the handler from the reads and `story_promise`.
#[derive(Debug, Clone, PartialEq)]
pub struct SprintPromise {
    pub key: Key,
    pub title: String,
    pub promise: StoryPromise,
    pub done_on: Option<NaiveDate>,
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
        Promise {
            on,
            by: "old@x.y".into(),
            at: t(at),
            to: to.map(str::to_string),
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
            let got = state(on, delivery, res, done_at.map(t), today);
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
                req(Some(overdue.on), None, Some("Bolt")),
                std,
                Ok((overdue.on, "ed@x.y", overdue.at, Some("Bolt".into()))),
            ),
            (
                "overdue, no date, recipient cleared",
                Some(overdue.clone()),
                req(None, None, Some("")),
                std,
                Ok((overdue.on, "ed@x.y", overdue.at, None)),
            ),
            (
                "overdue, no date, recipient blank spaces clears",
                Some(overdue.clone()),
                req(None, None, Some("   ")),
                std,
                Ok((overdue.on, "ed@x.y", overdue.at, None)),
            ),
            (
                "same date, to omitted keeps the recipient",
                Some(future.clone()),
                req(Some(future.on), None, None),
                std,
                Ok((future.on, "ed@x.y", future.at, Some("Acme".into()))),
            ),
            (
                "neither, to omitted keeps everything",
                Some(future.clone()),
                req(None, None, None),
                std,
                Ok((future.on, "ed@x.y", future.at, Some("Acme".into()))),
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
                "recipient is trimmed",
                None,
                req(Some(d(2026, 11, 25)), None, Some("  Acme ")),
                std,
                Ok((d(2026, 11, 25), "ed@x.y", now, Some("Acme".into()))),
            ),
        ];
        for (label, current, request, c, want) in rows {
            let got = change(current.as_ref(), request, "ed@x.y", now, &c);
            let want = want.map(|(on, by, at, to)| Promise {
                on,
                by: by.into(),
                at,
                to,
            });
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

    #[test]
    fn previous_end_picks_the_nearest_lower_planned_end() {
        let (s0, s1, s2) = (
            (1, d(2026, 11, 12)),
            (2, d(2026, 11, 30)),
            (3, d(2026, 11, 26)),
        );
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
                "closed sprint ending after the active one: earlier-ending sprint is its previous",
                s1,
                vec![s2, s0],
                Some(d(2026, 11, 26)),
            ),
            (
                "the active sprint's previous is the one before it",
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
                "S1 closed 12 Nov but ends 30 Nov, S2 active ends 26 Nov",
                vec![
                    (0, d(2026, 11, 12)),
                    (1, d(2026, 11, 30)),
                    (2, d(2026, 11, 26)),
                ],
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
        for (label, sprints) in setups {
            for day in 1..=30 {
                let on = d(2026, 11, day);
                let hits: Vec<i64> = sprints
                    .iter()
                    .filter(|(id, ends)| {
                        let others: Vec<_> =
                            sprints.iter().copied().filter(|(o, _)| o != id).collect();
                        in_sprint(on, previous_end((*id, *ends), &others), *ends)
                    })
                    .map(|(id, _)| *id)
                    .collect();
                assert_eq!(hits.len(), 1, "{label}: {on} matched {hits:?}");
            }
        }
        // Which sprint takes it, not just how many.
        let plain = [
            (1, d(2026, 11, 10)),
            (2, d(2026, 11, 24)),
            (3, d(2026, 11, 30)),
        ];
        let owner = |on: NaiveDate| {
            plain
                .iter()
                .find(|(id, ends)| {
                    let others: Vec<_> = plain.iter().copied().filter(|(o, _)| o != id).collect();
                    in_sprint(on, previous_end((*id, *ends), &others), *ends)
                })
                .map(|(id, _)| *id)
        };
        assert_eq!(
            owner(d(2026, 10, 1)),
            Some(1),
            "everything before the first end"
        );
        assert_eq!(owner(d(2026, 11, 10)), Some(1), "first end inclusive");
        assert_eq!(owner(d(2026, 11, 11)), Some(2), "day after the first end");
        assert_eq!(owner(d(2026, 11, 24)), Some(2), "second end inclusive");
        assert_eq!(owner(d(2026, 11, 30)), Some(3), "last end inclusive");
        assert_eq!(owner(d(2026, 12, 1)), None, "after the last end");
        // The S1/S2 case: 12-26 Nov belongs to S2 (id 2) although S1 (id 1) closed on 12 Nov.
        let odd = [
            (0, d(2026, 11, 12)),
            (1, d(2026, 11, 30)),
            (2, d(2026, 11, 26)),
        ];
        let owner = |on: NaiveDate| {
            odd.iter()
                .find(|(id, ends)| {
                    let others: Vec<_> = odd.iter().copied().filter(|(o, _)| o != id).collect();
                    in_sprint(on, previous_end((*id, *ends), &others), *ends)
                })
                .map(|(id, _)| *id)
        };
        assert_eq!(
            owner(d(2026, 11, 20)),
            Some(2),
            "mid-window date goes to the sprint ending 26 Nov"
        );
        assert_eq!(
            owner(d(2026, 11, 28)),
            Some(1),
            "late date goes to the sprint ending 30 Nov"
        );
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
            assert_eq!(days(st, on, done_at.map(t), today), want, "{label}");
        }
    }

    #[test]
    fn lane_gives_only_this_and_next_sprint() {
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
            assert_eq!(lane(st, dd), want, "{label}");
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
                assert_eq!(matches(f, &sp(st, dd)), w, "{label} / {name}");
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
                own.on,
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
                epic_late.on,
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
            (epic_early.on, Holder::Epic),
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
            (own.on, Holder::Own),
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
            ("on today", d(2026, 11, 20), (Some(Due::ThisSprint), false)),
            ("yesterday", d(2026, 11, 19), (None, true)),
            (
                "next sprint",
                d(2026, 12, 1),
                (Some(Due::NextSprint), false),
            ),
            ("later", d(2027, 1, 1), (Some(Due::Later), false)),
        ];
        for (label, on, want) in rows {
            assert_eq!(
                epic_due(&promise(on, "2026-11-01T00:00:00Z", None), &c),
                want,
                "{label}"
            );
        }
    }
}
