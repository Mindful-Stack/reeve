//! Sprints and goals (kb/ADR-0014): pure rules and the summary computation. No I/O and no clock: every instant comes
//! from the handler's `Ctx.now`.
use super::decision::RequiredDecision;
use super::items::{Epic, Initiative, Story};
use super::readiness::{DecisionFact, decision_reason};
use chrono::{DateTime, Days, NaiveDate, Utc};
use reeve_protocol::{
    ClosureKind, Confidence, DeliveryState, GoalVia, ItemType, Key, Reason, Resolution,
    SprintState, SprintWarning, Verdict,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Sprint {
    pub id: i64,
    pub state: SprintState,
    pub starts_on: NaiveDate,
    pub ends_on: NaiveDate,
    pub opened_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub version: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Goal {
    pub id: i64,
    pub sprint_id: i64,
    pub outcome: String,
    pub check: String,
    pub owner: String,
    pub notes: String,
    pub confidence: Option<Confidence>,
    pub confidence_note: Option<String>,
    pub confidence_at: Option<DateTime<Utc>>,
    pub suggested_confidence: Option<Confidence>,
    pub suggested_note: Option<String>,
    pub suggested_at: Option<DateTime<Utc>>,
    pub suggested_by: Option<String>,
    pub verdict: Option<Verdict>,
    pub verdict_note: Option<String>,
    pub version: i64,
    /// When the goal was created (stored since `1008_sprints.sql`): the draft panel's "Waiting since".
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalLink {
    pub goal_id: i64,
    pub item_key: Key,
}

/// What `summarise` needs of one story: its place in the hierarchy and its two timestamps.
#[derive(Debug, Clone)]
pub struct StoryTimes {
    pub key: Key,
    pub title: String,
    pub epic: Option<Key>,
    pub initiative: Option<Key>,
    pub delivery: DeliveryState,
    pub resolution: Option<Resolution>,
    pub started_at: Option<DateTime<Utc>>,
    pub done_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Open,
    Close,
    AddGoal,
    EditGoal,
    RemoveGoal,
    Confidence,
    Suggest,
    Link,
    Unlink,
}

/// The one table of which write each state allows; every cell is explicit, so a new `Op` or state fails to compile.
/// Interactive-only is checked separately (it depends on the caller).
///
/// `(Active, Op::Open)` is a backstop only: opening targets the draft, and "a sprint is already active" really comes
/// from the partial unique index (23505 mapped to `sprint_active`), so handlers must not rely on this cell alone.
pub fn allow(state: SprintState, op: Op) -> Result<(), Reason> {
    use SprintState::*;
    match (state, op) {
        (Draft, Op::Open) => Ok(()),
        (Draft, Op::Close) => Err(Reason::SprintNotActive),
        (Draft, Op::AddGoal) => Ok(()),
        (Draft, Op::EditGoal) => Ok(()),
        (Draft, Op::RemoveGoal) => Ok(()),
        (Draft, Op::Confidence) => Err(Reason::SprintNotActive),
        (Draft, Op::Suggest) => Err(Reason::SprintNotActive),
        (Draft, Op::Link) => Ok(()),
        (Draft, Op::Unlink) => Ok(()),
        (Active, Op::Open) => Err(Reason::SprintActive),
        (Active, Op::Close) => Ok(()),
        (Active, Op::AddGoal) => Ok(()),
        (Active, Op::EditGoal) => Ok(()),
        (Active, Op::RemoveGoal) => Err(Reason::SprintNotDraft),
        (Active, Op::Confidence) => Ok(()),
        (Active, Op::Suggest) => Ok(()),
        (Active, Op::Link) => Ok(()),
        (Active, Op::Unlink) => Ok(()),
        (
            Closed,
            Op::Open
            | Op::Close
            | Op::AddGoal
            | Op::EditGoal
            | Op::RemoveGoal
            | Op::Confidence
            | Op::Suggest
            | Op::Link
            | Op::Unlink,
        ) => Err(Reason::SprintClosed),
    }
}

/// A new draft's dates: from the active sprint's end when there is one, else from today; one configured length long.
pub fn draft_dates(
    active_ends_on: Option<NaiveDate>,
    now: DateTime<Utc>,
    length_days: u32,
) -> (NaiveDate, NaiveDate) {
    let starts = active_ends_on.unwrap_or(now.date_naive());
    (starts, starts + Days::new(length_days.into()))
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SprintDatesError {
    #[error("ends_on {ends} must be after {starts}")]
    EndNotAfterStart { starts: NaiveDate, ends: NaiveDate },
}

/// The dates an opened sprint shows: today (UTC, from `Ctx.now`) to the given end or one configured length later.
pub fn open_dates(
    now: DateTime<Utc>,
    ends_on: Option<NaiveDate>,
    length_days: u32,
) -> Result<(NaiveDate, NaiveDate), SprintDatesError> {
    let starts = now.date_naive();
    let ends = ends_on.unwrap_or(starts + Days::new(length_days.into()));
    if ends <= starts {
        return Err(SprintDatesError::EndNotAfterStart { starts, ends });
    }
    Ok((starts, ends))
}

pub fn warnings(goal_count: usize) -> Vec<SprintWarning> {
    match goal_count {
        0 => vec![SprintWarning::NoGoals],
        1..=3 => vec![],
        _ => vec![SprintWarning::MoreThanThreeGoals],
    }
}

/// Which goals a story serves: links to it, to its epic or to its epic's initiative. The nearest link wins `via`.
pub fn serves(s: &StoryTimes, links: &[GoalLink]) -> Vec<(i64, GoalVia)> {
    let mut out: Vec<(i64, GoalVia)> = Vec::new();
    for (k, via) in [
        (Some(&s.key), GoalVia::Story),
        (s.epic.as_ref(), GoalVia::Epic),
        (s.initiative.as_ref(), GoalVia::Initiative),
    ] {
        let Some(k) = k else { continue };
        for l in links.iter().filter(|l| &l.item_key == k) {
            if !out.iter().any(|(g, _)| *g == l.goal_id) {
                out.push((l.goal_id, via));
            }
        }
    }
    out
}

/// One goal's gates: what its work waits on, derived on read and never stored (kb/ADR-0014 R6, kb/ADR-0018 R1).
#[derive(Debug, Clone, PartialEq)]
pub struct GoalGates {
    pub goal_id: i64,
    /// Each link's initiative (an initiative itself, an epic's, a story's epic's), once, in link order.
    pub initiatives: Vec<Key>,
    /// The goal's initiatives, then its epics, each by key number, that lack the discovery mark (kb/ADR-0002 R3).
    pub discoveries: Vec<DiscoveryGate>,
    /// The goal's decisions that are open and not satisfied, by decision id, each with its readiness reason.
    pub decisions: Vec<DecisionGate>,
    /// Every initiative and epic among the goal's items; the marked ones are this less `discoveries`.
    pub discovery_total: u32,
    /// Every decision on the goal's items and stories, withdrawn ones excluded (kb/ADR-0013 R3); the decided ones are
    /// this less `decisions`.
    pub decisions_total: u32,
}

/// An initiative or epic among a goal's items without its discovery mark.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveryGate {
    pub key: Key,
    pub item: ItemType,
    pub title: String,
}

/// An open, unsatisfied decision among a goal's decisions: the item it is on and the reason `decision_reason` gives.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionGate {
    pub on: Key,
    pub reason: Reason,
}

/// Each goal's gates, in `goals` order. A goal's items are a linked initiative and its epics, a linked epic and its
/// initiative, and a linked story's epic and that epic's initiative; a linked story itself is one of the goal's
/// stories, not an item. Its stories are the ones `serves` gives it, done ones included but not those resolved as
/// dropped (never delivered, as `summarise` skips them): ancestors widen the items, never the stories. A discovery gate
/// is an initiative or epic among the items without its mark, whatever its horizon, as readiness treats every
/// ancestor. The goal's decisions are those on its items and stories, withdrawn ones excluded; a gate is one for which
/// `decision_reason` gives a reason. A link to an item that no longer exists adds nothing. Nothing here reads urgency
/// (kb/ADR-0024 R7).
pub fn goal_gates(
    goals: &[i64],
    links: &[GoalLink],
    items: (&[Initiative], &[Epic], &[Story]),
    decisions: &[(RequiredDecision, DecisionFact)],
) -> Vec<GoalGates> {
    use std::collections::{HashMap, HashSet};
    let (initiatives, epics, stories) = items;
    let initiative: HashMap<&Key, &Initiative> = initiatives.iter().map(|i| (i.key(), i)).collect();
    let epic: HashMap<&Key, &Epic> = epics.iter().map(|e| (e.key(), e)).collect();
    let story: HashMap<&Key, &Story> = stories.iter().map(|s| (s.key(), s)).collect();
    // The stories each goal serves, from `serves`, dropped ones left out.
    let mut served: HashMap<i64, Vec<&Key>> = HashMap::new();
    for s in stories {
        if s.resolution() == Some(Resolution::Dropped) {
            continue; // never delivered, as `summarise` skips it
        }
        let times = StoryTimes {
            key: s.key().clone(),
            title: String::new(),
            epic: s.epic().cloned(),
            initiative: s
                .epic()
                .and_then(|e| epic.get(e).copied())
                .and_then(Epic::initiative)
                .cloned(),
            delivery: s.delivery(),
            resolution: s.resolution(),
            started_at: None,
            done_at: None,
        };
        for (g, _) in serves(&times, links) {
            served.entry(g).or_default().push(s.key());
        }
    }
    goals
        .iter()
        .map(|&goal_id| {
            let mut goal_items: HashSet<&Key> = HashSet::new();
            let mut goal_initiatives: Vec<Key> = Vec::new();
            for l in links.iter().filter(|l| l.goal_id == goal_id) {
                let k = &l.item_key;
                let parent: Option<&Key> = if initiative.contains_key(k) {
                    goal_items.extend(
                        epics
                            .iter()
                            .filter(|e| e.initiative() == Some(k))
                            .map(Epic::key),
                    );
                    Some(k)
                } else if let Some(e) = epic.get(k).copied() {
                    e.initiative()
                } else if let Some(s) = story.get(k).copied() {
                    goal_items.extend(s.epic());
                    s.epic()
                        .and_then(|e| epic.get(e).copied())
                        .and_then(Epic::initiative)
                } else {
                    continue; // removed since it was linked
                };
                if !story.contains_key(k) {
                    goal_items.insert(k); // a linked story is one of the goal's stories, not an item
                }
                if let Some(i) = parent {
                    goal_items.insert(i);
                    if !goal_initiatives.contains(i) {
                        goal_initiatives.push(i.clone());
                    }
                }
            }
            let mut sorted: Vec<&Key> = goal_items.iter().copied().collect();
            sorted.sort_by_key(|k| (k.number(), k.to_string()));
            let mut discoveries = Vec::new();
            let mut discovery_total = 0u32;
            for k in &sorted {
                if let Some(i) = initiative.get(*k).copied() {
                    discovery_total += 1;
                    if i.discovery().is_none() {
                        discoveries.push(DiscoveryGate {
                            key: (*k).clone(),
                            item: ItemType::Initiative,
                            title: i.title().to_owned(),
                        });
                    }
                }
            }
            for k in &sorted {
                if let Some(e) = epic.get(*k).copied() {
                    discovery_total += 1;
                    if e.discovery().is_none() {
                        discoveries.push(DiscoveryGate {
                            key: (*k).clone(),
                            item: ItemType::Epic,
                            title: e.title().to_owned(),
                        });
                    }
                }
            }
            let mut under = goal_items;
            under.extend(served.get(&goal_id).into_iter().flatten().copied());
            let mut gates = Vec::new();
            let mut decisions_total = 0u32;
            for (d, fact) in decisions {
                debug_assert_eq!(d.id(), fact.id, "a decision paired with another's fact");
                let withdrawn = d
                    .closed()
                    .is_some_and(|c| matches!(c.kind, Some(ClosureKind::Withdrawn)));
                if withdrawn || !under.contains(d.on()) {
                    continue;
                }
                decisions_total += 1;
                if let Some(reason) = decision_reason(fact) {
                    gates.push(DecisionGate {
                        on: d.on().clone(),
                        reason,
                    });
                }
            }
            gates.sort_by_key(|g| g.reason.decision_id());
            GoalGates {
                goal_id,
                initiatives: goal_initiatives,
                discoveries,
                decisions: gates,
                discovery_total,
                decisions_total,
            }
        })
        .collect()
}

/// One goal's counts over a sprint's window (kb/ADR-0014 R2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoalProgress {
    pub goal_id: i64,
    pub completed: u32,
    pub in_flight: u32,
    pub todo: u32,
    pub in_review: u32,
    /// Live: recomputed on every read from the decisions open now.
    pub blocked_decisions: u32,
}

/// The sprint's counts: what `summarise` folds, what the sprint view serves and what the `closed` entry carries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SprintCounts {
    pub goals: Vec<GoalProgress>,
    pub completed: u32,
    pub upkeep_completed: u32,
}

/// A story completed inside the sprint's demo window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemoStory {
    pub key: Key,
    pub title: String,
}

/// The counts and the demo list (oldest completion first).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub counts: SprintCounts,
    pub demo: Vec<DemoStory>,
}

/// Stories are read once and folded here; `blocked` maps an unfinished story to the decision ids in its readiness
/// reasons. The window is `[from, to)`: a completion exactly at `to` belongs to the next sprint (kb/ADR-0014 R2).
pub fn summarise(
    goal_ids: &[i64],
    links: &[GoalLink],
    stories: &[StoryTimes],
    blocked: &std::collections::HashMap<Key, Vec<i64>>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Summary {
    let completed = |s: &StoryTimes| s.resolution == Some(Resolution::Completed);
    let completed_inside =
        |s: &StoryTimes| completed(s) && s.done_at.is_some_and(|t| t >= from && t < to);
    let mut goals: Vec<GoalProgress> = goal_ids
        .iter()
        .map(|&goal_id| GoalProgress {
            goal_id,
            ..Default::default()
        })
        .collect();
    let mut decisions: Vec<std::collections::BTreeSet<i64>> =
        vec![Default::default(); goal_ids.len()];
    let mut out = Summary::default();
    for s in stories {
        // Imported stories are completed with no `done_at`: history, like any completion before the window.
        let before = completed(s) && s.done_at.is_none_or(|t| t < from);
        if before || s.resolution == Some(Resolution::Dropped) {
            continue; // history, or never delivered: not this sprint's work
        }
        let mut served = serves(s, links);
        served.retain(|(g, _)| goal_ids.contains(g)); // only this sprint's goals count
        if completed_inside(s) {
            out.counts.completed += 1;
            if served.is_empty() {
                out.counts.upkeep_completed += 1;
            }
        }
        for (gid, _) in &served {
            let Some(i) = goal_ids.iter().position(|g| g == gid) else {
                continue;
            };
            let g = &mut goals[i];
            if completed_inside(s) {
                g.completed += 1;
            } else if s.started_at.is_some_and(|t| t < to) {
                // Not completed inside the window and not before it, so unfinished at `to`: in flight (at close).
                g.in_flight += 1;
                if matches!(s.delivery, DeliveryState::InReview | DeliveryState::Testing) {
                    g.in_review += 1;
                }
            } else {
                g.todo += 1;
            }
            if s.delivery != DeliveryState::Done {
                decisions[i].extend(blocked.get(&s.key).into_iter().flatten().copied());
            }
        }
    }
    for (g, d) in goals.iter_mut().zip(&decisions) {
        g.blocked_decisions = d.len() as u32;
    }
    let demo_from = from.max(to - chrono::Duration::days(7));
    let mut demo: Vec<&StoryTimes> = stories
        .iter()
        .filter(|s| completed(s) && s.done_at.is_some_and(|t| t >= demo_from && t < to))
        .collect();
    demo.sort_by_key(|s| s.done_at);
    out.demo = demo
        .into_iter()
        .map(|s| DemoStory {
            key: s.key.clone(),
            title: s.title.clone(),
        })
        .collect();
    out.counts.goals = goals;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::decision::Closed;
    use crate::domain::items::DiscoveryMark;
    use crate::domain::readiness::decision_fact;
    use chrono::{NaiveDate, TimeZone, Utc};
    use reeve_protocol::{CheckResult, Kind};

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }
    fn t(day: u32, h: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, day, h, 0, 0).unwrap()
    }

    #[test]
    fn a_new_draft_starts_where_the_active_sprint_ends() {
        assert_eq!(
            draft_dates(Some(d(2026, 11, 17)), t(3, 18), 14),
            (d(2026, 11, 17), d(2026, 12, 1))
        );
        assert_eq!(
            draft_dates(None, t(3, 18), 14),
            (d(2026, 11, 3), d(2026, 11, 17))
        );
    }

    #[test]
    fn opening_dates_run_from_today_unless_an_end_is_given() {
        assert_eq!(
            open_dates(t(3, 18), None, 14).unwrap(),
            (d(2026, 11, 3), d(2026, 11, 17))
        );
        assert_eq!(
            open_dates(t(3, 18), Some(d(2026, 11, 20)), 14).unwrap(),
            (d(2026, 11, 3), d(2026, 11, 20))
        );
        assert!(open_dates(t(3, 18), Some(d(2026, 11, 3)), 14).is_err());
    }

    #[test]
    fn writes_follow_the_state() {
        use Op::*;
        use reeve_protocol::{Reason as R, SprintState::*};
        let ops = [
            Open, Close, AddGoal, EditGoal, RemoveGoal, Confidence, Suggest, Link, Unlink,
        ];
        // Rows: Draft, Active, Closed; columns in `ops` order.
        let ok = Ok(());
        let expected: [(_, [Result<(), R>; 9]); 3] = [
            (
                Draft,
                [
                    ok.clone(),
                    Err(R::SprintNotActive),
                    ok.clone(),
                    ok.clone(),
                    ok.clone(),
                    Err(R::SprintNotActive),
                    Err(R::SprintNotActive),
                    ok.clone(),
                    ok.clone(),
                ],
            ),
            (
                Active,
                [
                    Err(R::SprintActive),
                    ok.clone(),
                    ok.clone(),
                    ok.clone(),
                    Err(R::SprintNotDraft),
                    ok.clone(),
                    ok.clone(),
                    ok.clone(),
                    ok.clone(),
                ],
            ),
            (Closed, std::array::from_fn(|_| Err(R::SprintClosed))),
        ];
        for (state, row) in expected {
            for (op, want) in ops.iter().zip(row) {
                assert_eq!(allow(state, *op), want, "{state:?} {op:?}");
            }
        }
    }

    fn k(s: &str) -> Key {
        s.parse().unwrap()
    }
    fn demo_keys(s: &Summary) -> Vec<Key> {
        s.demo.iter().map(|d| d.key.clone()).collect()
    }
    #[allow(clippy::type_complexity)]
    fn st(
        key: &str,
        epic: Option<&str>,
        delivery: DeliveryState,
        res: Option<Resolution>,
        started: Option<(u32, u32)>,
        done: Option<(u32, u32)>,
    ) -> StoryTimes {
        StoryTimes {
            key: k(key),
            title: key.into(),
            epic: epic.map(k),
            initiative: None,
            delivery,
            resolution: res,
            started_at: started.map(|(d, h)| t(d, h)),
            done_at: done.map(|(d, h)| t(d, h)),
        }
    }

    #[test]
    fn summary_counts_completions_inside_the_window_only() {
        use DeliveryState::*;
        use Resolution::*;
        let links = vec![GoalLink {
            goal_id: 1,
            item_key: k("ABC-10"),
        }]; // an epic
        let stories = vec![
            st(
                "ABC-11",
                Some("ABC-10"),
                Done,
                Some(Completed),
                Some((4, 12)),
                Some((6, 12)),
            ), // goal, completed in the sprint
            st(
                "ABC-12",
                Some("ABC-10"),
                InReview,
                None,
                Some((5, 12)),
                None,
            ), // goal, in flight (and in review)
            st("ABC-13", Some("ABC-10"), Todo, None, None, None), // goal, todo
            st(
                "ABC-14",
                None,
                Done,
                Some(Completed),
                Some((4, 12)),
                Some((5, 12)),
            ), // upkeep, completed
            st("ABC-15", None, InProgress, None, Some((5, 12)), None), // upkeep, started not completed: not in the share
            st(
                "ABC-16",
                Some("ABC-10"),
                Done,
                Some(Dropped),
                Some((4, 12)),
                Some((5, 12)),
            ), // dropped: left out
            st(
                "ABC-17",
                Some("ABC-10"),
                Done,
                Some(Completed),
                Some((1, 12)),
                Some((2, 12)),
            ), // before the window: left out
            st(
                "ABC-18",
                Some("ABC-10"),
                Done,
                Some(Completed),
                Some((6, 12)),
                Some((12, 12)),
            ), // after the close: in flight at close
        ];
        let blocked = std::collections::HashMap::from([(k("ABC-13"), vec![7_i64, 7, 9])]);
        // Window 3 Nov 18:00 to 11 Nov 18:00 (a closed sprint); the demo list covers its last seven days (from 4 Nov 18:00).
        let s = summarise(&[1], &links, &stories, &blocked, t(3, 18), t(11, 18));
        let g = &s.counts.goals[0];
        assert_eq!(
            (
                g.completed,
                g.in_flight,
                g.todo,
                g.in_review,
                g.blocked_decisions
            ),
            (1, 2, 1, 1, 2)
        );
        assert_eq!((s.counts.completed, s.counts.upkeep_completed), (2, 1));
        assert_eq!(
            s.demo.iter().map(|d| d.key.to_string()).collect::<Vec<_>>(),
            vec!["ABC-14", "ABC-11"]
        );
    }

    #[test]
    fn the_window_is_half_open_and_a_boundary_completion_belongs_to_the_next_sprint() {
        use DeliveryState::*;
        use Resolution::*;
        let links = vec![GoalLink {
            goal_id: 1,
            item_key: k("ABC-10"),
        }];
        let at_open = st(
            "ABC-1",
            Some("ABC-10"),
            Done,
            Some(Completed),
            Some((1, 12)),
            Some((3, 18)),
        );
        let at_close = st(
            "ABC-2",
            Some("ABC-10"),
            Done,
            Some(Completed),
            Some((4, 12)),
            Some((11, 18)),
        );
        let stories = [at_open, at_close];
        let none = Default::default();
        // The first sprint: [3 Nov 18:00, 11 Nov 18:00). ABC-1 completes at its opening (inside); ABC-2 at its close (outside).
        let first = summarise(&[1], &links, &stories, &none, t(3, 18), t(11, 18));
        assert_eq!(first.counts.completed, 1);
        assert_eq!(first.counts.goals[0].completed, 1);
        assert_eq!(first.counts.goals[0].in_flight, 1); // ABC-2 started inside, not completed by the close
        assert!(first.demo.is_empty()); // ABC-1 is older than the last seven days, ABC-2 is outside
        // The next sprint: [11 Nov 18:00, 15 Nov 18:00). ABC-2 counts here; ABC-1 is history.
        let next = summarise(&[1], &links, &stories, &none, t(11, 18), t(15, 18));
        assert_eq!(next.counts.completed, 1);
        assert_eq!(next.counts.goals[0].completed, 1);
        assert_eq!(next.counts.goals[0].in_flight, 0);
        assert_eq!(next.counts.goals[0].todo, 0);
        assert_eq!(demo_keys(&next), vec![k("ABC-2")]);
    }

    #[test]
    fn a_story_started_at_the_close_is_not_in_flight() {
        let links = vec![GoalLink {
            goal_id: 1,
            item_key: k("ABC-10"),
        }];
        let late = st(
            "ABC-1",
            Some("ABC-10"),
            DeliveryState::InProgress,
            None,
            Some((11, 18)),
            None,
        );
        let s = summarise(
            &[1],
            &links,
            &[late],
            &Default::default(),
            t(3, 18),
            t(11, 18),
        );
        assert_eq!(
            (s.counts.goals[0].in_flight, s.counts.goals[0].todo),
            (0, 1)
        );
    }

    #[test]
    fn a_completed_story_without_a_completion_time_is_history() {
        // Imported stories are Done/Completed with no done_at: never todo, never counted.
        let links = vec![GoalLink {
            goal_id: 1,
            item_key: k("ABC-10"),
        }];
        let imported = st(
            "ABC-1",
            Some("ABC-10"),
            DeliveryState::Done,
            Some(Resolution::Completed),
            None,
            None,
        );
        let s = summarise(
            &[1],
            &links,
            &[imported],
            &Default::default(),
            t(3, 18),
            t(11, 18),
        );
        let g = &s.counts.goals[0];
        assert_eq!(
            (g.completed, g.in_flight, g.todo, s.counts.completed),
            (0, 0, 0, 0)
        );
    }

    #[test]
    fn a_story_serving_two_goals_counts_once_in_the_totals_and_once_per_goal() {
        use DeliveryState::Done;
        use Resolution::Completed;
        let links = vec![
            GoalLink {
                goal_id: 1,
                item_key: k("ABC-1"),
            },
            GoalLink {
                goal_id: 2,
                item_key: k("ABC-10"),
            },
        ];
        let both = st(
            "ABC-1",
            Some("ABC-10"),
            Done,
            Some(Completed),
            Some((4, 12)),
            Some((5, 12)),
        );
        let s = summarise(
            &[1, 2],
            &links,
            &[both],
            &Default::default(),
            t(3, 18),
            t(11, 18),
        );
        assert_eq!((s.counts.completed, s.counts.upkeep_completed), (1, 0));
        assert_eq!(
            (s.counts.goals[0].completed, s.counts.goals[1].completed),
            (1, 1)
        );
    }

    #[test]
    fn the_demo_list_never_reaches_before_a_short_window() {
        use DeliveryState::Done;
        use Resolution::Completed;
        let before = st(
            "ABC-1",
            None,
            Done,
            Some(Completed),
            Some((1, 12)),
            Some((3, 12)),
        );
        let inside = st(
            "ABC-2",
            None,
            Done,
            Some(Completed),
            Some((3, 12)),
            Some((4, 12)),
        );
        let s = summarise(
            &[],
            &[],
            &[before, inside],
            &Default::default(),
            t(3, 18),
            t(6, 18),
        );
        assert_eq!(demo_keys(&s), vec![k("ABC-2")]);
    }

    #[test]
    fn blocked_decisions_come_from_unfinished_stories_and_testing_is_in_review() {
        use DeliveryState::*;
        let links = vec![GoalLink {
            goal_id: 1,
            item_key: k("ABC-10"),
        }];
        let testing = st("ABC-1", Some("ABC-10"), Testing, None, Some((4, 12)), None);
        // Completed after the close (day 12, the window ends day 11): in flight at close, but Done now, so its decision
        // (8) is not counted; only unfinished stories add to blocked_decisions.
        let finished_later = st(
            "ABC-2",
            Some("ABC-10"),
            Done,
            Some(Resolution::Completed),
            Some((5, 12)),
            Some((12, 12)),
        );
        let blocked =
            std::collections::HashMap::from([(k("ABC-1"), vec![7_i64]), (k("ABC-2"), vec![8_i64])]);
        let s = summarise(
            &[1],
            &links,
            &[testing, finished_later],
            &blocked,
            t(3, 18),
            t(11, 18),
        );
        let g = &s.counts.goals[0];
        assert_eq!((g.in_flight, g.in_review, g.blocked_decisions), (2, 1, 1));
    }

    #[test]
    fn upkeep_is_decided_against_this_sprints_goals() {
        use DeliveryState::Done;
        use Resolution::Completed;
        // The link belongs to goal 9, which is not one of this sprint's goals: the story is upkeep here.
        let links = vec![GoalLink {
            goal_id: 9,
            item_key: k("ABC-1"),
        }];
        let story = st(
            "ABC-1",
            None,
            Done,
            Some(Completed),
            Some((4, 12)),
            Some((5, 12)),
        );
        let s = summarise(
            &[1],
            &links,
            &[story],
            &Default::default(),
            t(3, 18),
            t(11, 18),
        );
        assert_eq!((s.counts.completed, s.counts.upkeep_completed), (1, 1));
    }

    #[test]
    fn summary_carries_one_progress_per_goal() {
        let s = summarise(&[1], &[], &[], &Default::default(), t(3, 18), t(11, 18));
        assert_eq!(s.counts.goals.len(), 1);
    }

    #[test]
    fn warnings_start_above_three_goals() {
        use reeve_protocol::SprintWarning::*;
        assert_eq!(warnings(0), vec![NoGoals]);
        assert!(warnings(3).is_empty());
        assert_eq!(warnings(4), vec![MoreThanThreeGoals]);
    }

    #[test]
    fn the_nearest_link_names_how_a_story_serves_a_goal() {
        let k = |s: &str| s.parse::<Key>().unwrap();
        let s = StoryTimes {
            key: k("ABC-3"),
            title: "t".into(),
            epic: Some(k("ABC-2")),
            initiative: Some(k("ABC-1")),
            delivery: DeliveryState::Todo,
            resolution: None,
            started_at: None,
            done_at: None,
        };
        let link = |g, i: &str| GoalLink {
            goal_id: g,
            item_key: k(i),
        };
        let links = [
            link(1, "ABC-1"),
            link(1, "ABC-3"),
            link(2, "ABC-2"),
            link(3, "ABC-9"),
        ];
        assert_eq!(
            serves(&s, &links),
            vec![(1, GoalVia::Story), (2, GoalVia::Epic)]
        );
    }

    // --- goal_gates ---------------------------------------------------------------------------------------------------

    fn ini(key: &str, marked: bool) -> Initiative {
        Initiative::from_persisted(
            k(key),
            format!("I {key}"),
            String::new(),
            "Now".into(),
            None,
            None,
            marked.then(|| DiscoveryMark {
                at: t(1, 0),
                by: "dan".into(),
            }),
            None,
            1,
        )
    }
    fn epic(key: &str, parent: Option<&str>, marked: bool) -> Epic {
        Epic::from_persisted(
            k(key),
            format!("E {key}"),
            parent.map(k),
            String::new(),
            marked.then(|| DiscoveryMark {
                at: t(1, 0),
                by: "dan".into(),
            }),
            None,
            1,
        )
    }
    fn story(key: &str, parent: Option<&str>) -> Story {
        story_with(key, parent, DeliveryState::Todo, None)
    }
    fn story_with(
        key: &str,
        parent: Option<&str>,
        delivery: DeliveryState,
        resolution: Option<Resolution>,
    ) -> Story {
        Story::from_persisted(
            k(key),
            key.into(),
            Kind::Story,
            parent.map(k),
            String::new(),
            vec![],
            None,
            vec![],
            0,
            delivery,
            resolution,
            None,
            1,
        )
    }
    /// A decision on `on`: open when `closed` is None, else closed with that kind (`Some(None)`: closed before close
    /// kinds existed). Its fact comes from the one builder, as the handler builds it.
    fn dec(
        id: i64,
        on: &str,
        on_type: ItemType,
        check: CheckResult,
        closed: Option<Option<ClosureKind>>,
    ) -> (RequiredDecision, DecisionFact) {
        let d = RequiredDecision::from_persisted(
            id,
            k(on),
            on_type,
            format!("d{id}"),
            "dan".into(),
            None,
            None,
            None,
            None,
            check,
            None,
            closed.map(|kind| Closed {
                at: t(2, 0),
                by: "dan".into(),
                reason: "r".into(),
                kind,
                via: None,
            }),
            t(1, 0),
            1,
        );
        let f = decision_fact(&d);
        (d, f)
    }
    /// Open, unchecked, no record: a `DecisionNoRecord` gate.
    fn pending(id: i64, on: &str, on_type: ItemType) -> (RequiredDecision, DecisionFact) {
        dec(id, on, on_type, CheckResult::Unchecked, None)
    }
    fn lk(goal_id: i64, key: &str) -> GoalLink {
        GoalLink {
            goal_id,
            item_key: k(key),
        }
    }
    fn no_record(id: i64, on: &str) -> DecisionGate {
        DecisionGate {
            on: k(on),
            reason: Reason::DecisionNoRecord {
                decision_id: id,
                title: format!("d{id}"),
            },
        }
    }
    fn gate(key: &str, item: ItemType) -> DiscoveryGate {
        let level = if item == ItemType::Initiative {
            "I"
        } else {
            "E"
        };
        DiscoveryGate {
            key: k(key),
            item,
            title: format!("{level} {key}"),
        }
    }

    /// A story link takes in its epic and that epic's initiative as items, never its siblings: the sibling's decision is
    /// not the goal's, and the initiative's other epic (ABC-3) is not an item.
    #[test]
    fn gates_story_link_takes_in_ancestors_not_siblings() {
        let i = [ini("ABC-1", false)];
        let e = [
            epic("ABC-2", Some("ABC-1"), true),
            epic("ABC-3", Some("ABC-1"), false),
        ];
        let s = [story("ABC-4", Some("ABC-2")), story("ABC-5", Some("ABC-2"))];
        let decisions = [
            pending(1, "ABC-4", ItemType::Story),
            pending(2, "ABC-5", ItemType::Story),
            pending(3, "ABC-2", ItemType::Epic),
        ];
        assert_eq!(
            goal_gates(
                &[1],
                &[lk(1, "ABC-4")],
                (&i[..], &e[..], &s[..]),
                &decisions
            ),
            vec![GoalGates {
                goal_id: 1,
                initiatives: vec![k("ABC-1")],
                discoveries: vec![gate("ABC-1", ItemType::Initiative)],
                decisions: vec![no_record(1, "ABC-4"), no_record(3, "ABC-2")],
                discovery_total: 2,
                decisions_total: 2,
            }]
        );
    }

    /// An epic link (the header goal picker's usual case) takes in its initiative; an initiative link takes in all its
    /// epics, one without stories included; discovery gates list the initiative first even when its key is higher.
    #[test]
    fn gates_epic_and_initiative_links() {
        let i = [ini("ABC-9", false)];
        let e = [
            epic("ABC-2", Some("ABC-9"), true),
            epic("ABC-3", Some("ABC-9"), false),
        ];
        let s = [story("ABC-4", Some("ABC-2"))];
        let decisions = [
            pending(1, "ABC-3", ItemType::Epic),
            pending(2, "ABC-4", ItemType::Story),
        ];
        let got = goal_gates(
            &[1, 2],
            &[lk(1, "ABC-2"), lk(2, "ABC-9")],
            (&i[..], &e[..], &s[..]),
            &decisions,
        );
        assert_eq!(
            got,
            vec![
                GoalGates {
                    goal_id: 1,
                    initiatives: vec![k("ABC-9")],
                    discoveries: vec![gate("ABC-9", ItemType::Initiative)],
                    decisions: vec![no_record(2, "ABC-4")],
                    discovery_total: 2,
                    decisions_total: 1,
                },
                GoalGates {
                    goal_id: 2,
                    initiatives: vec![k("ABC-9")],
                    discoveries: vec![
                        gate("ABC-9", ItemType::Initiative),
                        gate("ABC-3", ItemType::Epic)
                    ],
                    decisions: vec![no_record(1, "ABC-3"), no_record(2, "ABC-4")],
                    discovery_total: 3,
                    decisions_total: 2,
                },
            ]
        );
    }

    /// Satisfied, closed `decided` and closed before close kinds all count as decided; withdrawn is in neither count;
    /// each open, unsatisfied one is a gate with readiness's own reason.
    #[test]
    fn gates_each_decision_state_lands_in_its_count() {
        use ClosureKind::{Decided, Withdrawn};
        let accepted = CheckResult::Accepted {
            commit: "abc".into(),
            reversibility: None,
        };
        let i = [ini("ABC-1", true)];
        let e = [epic("ABC-2", Some("ABC-1"), true)];
        let s = [story("ABC-4", Some("ABC-2"))];
        let decisions = [
            dec(1, "ABC-1", ItemType::Initiative, accepted, None), // satisfied
            dec(
                2,
                "ABC-2",
                ItemType::Epic,
                CheckResult::Unchecked,
                Some(Some(Decided)),
            ), // closed decided
            dec(
                3,
                "ABC-2",
                ItemType::Epic,
                CheckResult::Unchecked,
                Some(None),
            ), // closed, no kind
            dec(
                4,
                "ABC-4",
                ItemType::Story,
                CheckResult::Unchecked,
                Some(Some(Withdrawn)),
            ), // withdrawn
            pending(5, "ABC-4", ItemType::Story),                  // a gate
            dec(
                6,
                "ABC-4",
                ItemType::Story,
                CheckResult::ProposedPrOpen,
                None,
            ), // a gate, its own reason
        ];
        let g = &goal_gates(
            &[1],
            &[lk(1, "ABC-2")],
            (&i[..], &e[..], &s[..]),
            &decisions,
        )[0];
        assert_eq!(
            g.decisions,
            vec![
                no_record(5, "ABC-4"),
                DecisionGate {
                    on: k("ABC-4"),
                    reason: Reason::DecisionProposedPrOpen {
                        decision_id: 6,
                        title: "d6".into(),
                    },
                },
            ]
        );
        assert_eq!(g.decisions_total, 5);
        // What the page shows: marked and decided are the totals less the gates.
        assert_eq!(
            (
                g.discovery_total - g.discoveries.len() as u32,
                g.decisions_total - g.decisions.len() as u32
            ),
            (2, 3)
        );
    }

    /// A linked story without an epic: its own decisions only, no initiative, nothing to discover.
    #[test]
    fn gates_linked_story_without_epic() {
        let i: [Initiative; 0] = [];
        let e: [Epic; 0] = [];
        let s = [story("ABC-6", None)];
        let decisions = [pending(1, "ABC-6", ItemType::Story)];
        assert_eq!(
            goal_gates(
                &[1],
                &[lk(1, "ABC-6")],
                (&i[..], &e[..], &s[..]),
                &decisions
            ),
            vec![GoalGates {
                goal_id: 1,
                initiatives: vec![],
                discoveries: vec![],
                decisions: vec![no_record(1, "ABC-6")],
                discovery_total: 0,
                decisions_total: 1,
            }]
        );
    }

    /// Each link resolves to its initiative in link order (links come by key number), once.
    #[test]
    fn gates_initiatives_in_link_order_once() {
        let i = [ini("ABC-10", true), ini("ABC-11", true)];
        let e = [
            epic("ABC-2", Some("ABC-11"), true),
            epic("ABC-3", Some("ABC-10"), true),
        ];
        let s = [story("ABC-4", Some("ABC-2")), story("ABC-5", Some("ABC-3"))];
        let got = goal_gates(
            &[1],
            &[lk(1, "ABC-2"), lk(1, "ABC-4"), lk(1, "ABC-5")],
            (&i[..], &e[..], &s[..]),
            &[],
        );
        assert_eq!(got[0].initiatives, vec![k("ABC-11"), k("ABC-10")]);
    }

    /// A done story's decisions still count; a dropped story is not the goal's, so its open decision is neither a gate
    /// nor in the total.
    #[test]
    fn gates_done_story_counts_dropped_story_does_not() {
        let i = [ini("ABC-1", true)];
        let e = [epic("ABC-2", Some("ABC-1"), true)];
        let done = story_with("ABC-4", Some("ABC-2"), DeliveryState::Done, None);
        let dropped = story_with(
            "ABC-5",
            Some("ABC-2"),
            DeliveryState::Done,
            Some(Resolution::Dropped),
        );
        let s = [done, dropped];
        let decisions = [
            pending(1, "ABC-4", ItemType::Story),
            pending(2, "ABC-5", ItemType::Story),
        ];
        let g = &goal_gates(
            &[1],
            &[lk(1, "ABC-2")],
            (&i[..], &e[..], &s[..]),
            &decisions,
        )[0];
        assert_eq!(g.decisions, vec![no_record(1, "ABC-4")]);
        assert_eq!(g.decisions_total, 1);
    }

    /// A link to an item that no longer exists adds nothing.
    #[test]
    fn gates_link_to_a_removed_item_adds_nothing() {
        let i: [Initiative; 0] = [];
        let e: [Epic; 0] = [];
        let s: [Story; 0] = [];
        // A decision still on the removed key (no foreign key) is not the goal's.
        let decisions = [pending(1, "ABC-99", ItemType::Story)];
        assert_eq!(
            goal_gates(
                &[1],
                &[lk(1, "ABC-99")],
                (&i[..], &e[..], &s[..]),
                &decisions
            ),
            vec![GoalGates {
                goal_id: 1,
                initiatives: vec![],
                discoveries: vec![],
                decisions: vec![],
                discovery_total: 0,
                decisions_total: 0,
            }]
        );
    }

    /// A dropped story linked directly is neither an item nor served: its open decision is not the goal's.
    #[test]
    fn gates_directly_linked_dropped_story_does_not_count() {
        let i = [ini("ABC-1", true)];
        let e = [epic("ABC-2", Some("ABC-1"), true)];
        let s = [story_with(
            "ABC-5",
            Some("ABC-2"),
            DeliveryState::Done,
            Some(Resolution::Dropped),
        )];
        let decisions = [pending(1, "ABC-5", ItemType::Story)];
        let g = &goal_gates(
            &[1],
            &[lk(1, "ABC-5")],
            (&i[..], &e[..], &s[..]),
            &decisions,
        )[0];
        assert_eq!((g.decisions.len(), g.decisions_total), (0, 0));
        assert_eq!(g.initiatives, vec![k("ABC-1")]);
    }
}
