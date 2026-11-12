//! The decision queue: pure, over decisions and the readiness the loader already computed. Nothing here
//! derives readiness (kb/ADR-0027 R1, kb/ADR-0013 R4); affected stories are read off the reasons. `needed_in`, the
//! next step and the lane are derived on read and never stored (kb/ADR-0014 R6, kb/ADR-0027 R1).

use crate::domain::decision::RequiredDecision;
use crate::domain::promise::{HeldBy, State, StoryPromise};
use chrono::NaiveDate;
use reeve_protocol::{
    CheckResult, DeliveryState, ItemType, Key, Kind, NeededIn, NeededInReason, NextStep,
    QueueGroup, QueueLane, Reason,
};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

/// An unfinished story with the facts the queue classifies by (kb/ADR-0027 R5: unfinished is delivery not done).
#[derive(Debug, Clone)]
pub struct AffectedStory {
    pub key: Key,
    pub title: String,
    pub kind: Kind,
    pub delivery: DeliveryState,
    pub owner: Option<String>,
    /// Lineage urgency (kb/ADR-0017 R3), shown only; nothing here sorts or classifies by it (R5).
    pub urgent: bool,
    /// The active sprint's goals the story serves, as (goal id, outcome), nearest link first.
    pub active_goals: Vec<(i64, String)>,
    /// The draft sprint's goals the story serves.
    pub draft_goals: Vec<(i64, String)>,
    /// The story's effective promise and the decision window it asks for (kb/ADR-0025 R7); None when unpromised.
    pub promise: Option<PromiseWindow>,
    pub reasons: Vec<Reason>,
}

/// An affected story's promise as the queue reads it: the item holding it (`key`, `level`: the story, or the epic it
/// inherits from), the effective date, the window `promise::window` derived, and whether it is overdue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromiseWindow {
    pub key: Key,
    pub level: ItemType,
    pub on: NaiveDate,
    pub window: NeededIn,
    pub overdue: bool,
}

impl PromiseWindow {
    /// The queue's view of story `story`'s promise, read from the domain value the story list derived on the queue's
    /// snapshot: the holder is the story itself or the epic it inherits from (`held_by`), and the window is
    /// `StoryPromise::window` (kb/ADR-0025 R7).
    pub fn of(story: &Key, sp: &StoryPromise) -> PromiseWindow {
        let (key, level) = match sp.held_by() {
            HeldBy::Own => (story.clone(), ItemType::Story),
            HeldBy::Epic(k) => (k, ItemType::Epic),
        };
        PromiseWindow {
            key,
            level,
            on: sp.on(),
            window: sp.window(),
            overdue: sp.state() == Some(State::Overdue),
        }
    }
}

/// One item on a decision item's lineage, nearest first: the item itself, then its epic or initiative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineageItem {
    pub key: Key,
    pub level: ItemType,
    pub discovery_marked: bool,
    /// The horizon of the initiative this item is or belongs to (None for an epic without one).
    pub initiative_horizon: Option<String>,
}

pub struct ClassifyInputs<'a> {
    pub stories: &'a [AffectedStory],
    /// Keyed by the decision's `on` key; a story item's lineage starts at its epic.
    pub lineage: &'a HashMap<Key, Vec<LineageItem>>,
    /// The first configured horizon bucket (kb/ADR-0016 R2); None when none is configured.
    pub first_bucket: Option<&'a str>,
    /// Configured repository names (`Settings.repos`).
    pub repos: &'a [String],
}

/// A story a decision holds back (todo, in `held`) or that is under way without it (`under_way`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueStory {
    pub key: Key,
    pub title: String,
    pub kind: Kind,
    pub delivery: DeliveryState,
    pub owner: Option<String>,
    pub urgent: bool,
    /// The story's own window: `this_sprint`, `next_sprint` or `beyond`, the earlier of its goals' and its promise's.
    pub needed_in: NeededIn,
}

/// One decision in the queue, with the todo stories it holds (`held`; the wire calls them `blocks`) and the started
/// ones it affects (`under_way`); both empty outside the live lanes.
#[derive(Debug)]
pub struct QueueEntry {
    pub group: QueueGroup,
    pub lane: QueueLane,
    pub needed_in: NeededIn,
    pub needed_in_reason: Option<NeededInReason>,
    pub next_step: Option<NextStep>,
    pub decision: RequiredDecision,
    pub held: Vec<QueueStory>,
    pub under_way: Vec<QueueStory>,
}

/// Counts over open, unsatisfied decisions only (except the lane sizes); `blocked_stories` counts each story once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueSummary {
    pub open: u32,
    pub blocking: u32,
    pub blocked_stories: u32,
    pub overdue: u32,
    pub review: u32,
    pub this_sprint: u32,
    pub next_sprint: u32,
}

/// The nearest lineage item in discovery under the first horizon bucket: unmarked, and an initiative in that bucket or
/// an epic whose initiative is in it. Any ancestor counts (as readiness's `AncestorDiscoveryIncomplete` does). None when
/// nothing qualifies or no bucket is configured.
pub fn discovery_item<'a>(
    lineage: &'a [LineageItem],
    first_bucket: Option<&str>,
) -> Option<&'a LineageItem> {
    let b = first_bucket?;
    lineage
        .iter()
        .find(|l| !l.discovery_marked && l.initiative_horizon.as_deref() == Some(b))
}

/// The earlier of two windows: `this_sprint` < `next_sprint` < `beyond` < `none` (the declaration order `Ord` follows).
fn earlier(a: NeededIn, b: NeededIn) -> NeededIn {
    a.min(b)
}

/// A story's own window: the earlier of its goals' (`beyond` without any) and its promise's (kb/ADR-0025 R7). A
/// promise window of `none` (later, or no longer open) never moves it.
fn story_window(s: &AffectedStory) -> NeededIn {
    let goals = if !s.active_goals.is_empty() {
        NeededIn::ThisSprint
    } else if !s.draft_goals.is_empty() {
        NeededIn::NextSprint
    } else {
        NeededIn::Beyond
    };
    s.promise
        .as_ref()
        .map_or(goals, |p| earlier(goals, p.window))
}

/// One candidate per rule, in tie order: an active goal (`this_sprint`), a draft goal (`next_sprint`), the item (or an
/// ancestor) in discovery under the first bucket (`next_sprint`), the best promise among the affected stories (its
/// window, only `this_sprint` or `next_sprint`; best by window, then date, then story key number), any affected story
/// (`beyond`, no reason). The earliest window wins, and within it the first candidate. With no promise the windows
/// are already in rule order, so this is the first rule that holds. Closed or satisfied: none.
pub fn needed_in(
    d: &RequiredDecision,
    affected: &[&AffectedStory],
    discovery: Option<&LineageItem>,
) -> (NeededIn, Option<NeededInReason>) {
    if d.closed().is_some() || d.is_satisfied() {
        return (NeededIn::None, None);
    }
    let goal = |pick: fn(&AffectedStory) -> &Vec<(i64, String)>| {
        affected
            .iter()
            .flat_map(|s| pick(s).first())
            .min_by_key(|(id, _)| *id)
            .map(|(id, o)| NeededInReason::Goal {
                goal_id: *id,
                outcome: o.clone(),
            })
    };
    let promise = affected
        .iter()
        .filter_map(|s| s.promise.as_ref().map(|p| (s, p)))
        .filter(|(_, p)| matches!(p.window, NeededIn::ThisSprint | NeededIn::NextSprint))
        .min_by_key(|(s, p)| (p.window, p.on, s.key.number(), s.key.to_string()))
        .map(|(_, p)| {
            (
                p.window,
                NeededInReason::Promise {
                    key: p.key.clone(),
                    level: p.level,
                    on: p.on,
                    overdue: p.overdue,
                },
            )
        });
    let candidates = [
        goal(|s| &s.active_goals).map(|r| (NeededIn::ThisSprint, Some(r))),
        goal(|s| &s.draft_goals).map(|r| (NeededIn::NextSprint, Some(r))),
        discovery.map(|l| {
            (
                NeededIn::NextSprint,
                Some(NeededInReason::Discovery {
                    key: l.key.clone(),
                    level: l.level,
                }),
            )
        }),
        promise.map(|(w, r)| (w, Some(r))),
        (!affected.is_empty()).then_some((NeededIn::Beyond, None)),
    ];
    // `min_by_key` keeps the first of equal windows, so the candidate order breaks ties.
    candidates
        .into_iter()
        .flatten()
        .min_by_key(|(w, _)| *w)
        .unwrap_or((NeededIn::None, None))
}

/// First row that holds; None for closed or satisfied decisions.
pub fn next_step(d: &RequiredDecision, repos: &[String]) -> Option<NextStep> {
    if d.closed().is_some() || d.is_satisfied() {
        return None;
    }
    let Some(r) = d.record() else {
        return Some(NextStep::WriteRecord);
    };
    if !repos.iter().any(|x| x == r.repo()) {
        return Some(NextStep::ConfigureRepo {
            repo: r.repo().to_string(),
        });
    }
    match (d.check(), d.pr_url()) {
        (c @ (CheckResult::NotFound | CheckResult::NotAccepted { .. }), _) => {
            Some(NextStep::FixRecord { result: c.clone() })
        }
        (CheckResult::ProposedPrOpen | CheckResult::Unchecked, Some(pr)) => {
            Some(NextStep::ReviewRecord {
                pr_url: pr.to_string(),
            })
        }
        _ => Some(NextStep::AwaitCheck {
            checked_at: d.checked_at(),
        }),
    }
}

/// The lane: closed and satisfied keep their own; a record awaiting review leaves the sprint lanes for Review when it
/// is needed this sprint or next; otherwise the window decides. Lanes are disjoint.
pub fn lane(group: QueueGroup, needed: NeededIn, step: Option<&NextStep>) -> QueueLane {
    match group {
        QueueGroup::Closed => QueueLane::Closed,
        QueueGroup::Satisfied => QueueLane::Satisfied,
        _ => match (needed, step) {
            (NeededIn::ThisSprint | NeededIn::NextSprint, Some(NextStep::ReviewRecord { .. })) => {
                QueueLane::Review
            }
            (NeededIn::ThisSprint, _) => QueueLane::ThisSprint,
            (NeededIn::NextSprint, _) => QueueLane::NextSprint,
            (NeededIn::Beyond, _) => QueueLane::Beyond,
            (NeededIn::None, _) => QueueLane::Open,
        },
    }
}

fn overdue(d: &RequiredDecision, today: NaiveDate) -> bool {
    d.due().is_some_and(|due| due < today)
}

/// Urgency order inside the blocking and open groups.
fn urgency(a: &QueueEntry, b: &QueueEntry, today: NaiveDate) -> Ordering {
    let (da, db) = (&a.decision, &b.decision);
    let (oa, ob) = (overdue(da, today), overdue(db, today));
    ob.cmp(&oa) // overdue first
        .then_with(|| {
            if oa && ob {
                da.due().cmp(&db.due())
            } else {
                Ordering::Equal
            }
        })
        .then_with(|| b.held.len().cmp(&a.held.len()))
        .then_with(|| da.due().is_some().cmp(&db.due().is_some())) // undated first
        .then_with(|| match (da.due(), db.due()) {
            (Some(x), Some(y)) => x.cmp(&y),
            _ => da.created_at().cmp(&db.created_at()),
        })
        .then_with(|| da.id().cmp(&db.id()))
}

fn story_of(s: &AffectedStory) -> QueueStory {
    QueueStory {
        key: s.key.clone(),
        title: s.title.clone(),
        kind: s.kind,
        delivery: s.delivery,
        owner: s.owner.clone(),
        urgent: s.urgent,
        needed_in: story_window(s),
    }
}

/// Classifies, groups, orders and counts `ds`. Each decision's affected stories are the unfinished ones whose readiness
/// reasons name it (deduplicated per story); the todo ones are `held`, the started ones `under_way`.
pub fn order(
    ds: Vec<RequiredDecision>,
    inp: &ClassifyInputs,
    today: NaiveDate,
) -> (Vec<QueueEntry>, QueueSummary) {
    let mut affected: BTreeMap<i64, Vec<&AffectedStory>> = BTreeMap::new();
    for s in inp
        .stories
        .iter()
        .filter(|s| s.delivery != DeliveryState::Done)
    {
        let mut seen = HashSet::new();
        for id in s.reasons.iter().filter_map(|r| r.decision_id()) {
            if seen.insert(id) {
                affected.entry(id).or_default().push(s);
            }
        }
    }
    let mut entries: Vec<QueueEntry> = ds
        .into_iter()
        .map(|decision| {
            let all = affected.remove(&decision.id()).unwrap_or_default();
            let live = decision.closed().is_none() && !decision.is_satisfied();
            let (held, under_way): (Vec<_>, Vec<_>) = if live {
                all.iter()
                    .copied()
                    .partition(|s| s.delivery == DeliveryState::Todo)
            } else {
                (vec![], vec![])
            };
            let group = if decision.closed().is_some() {
                QueueGroup::Closed
            } else if decision.is_satisfied() {
                QueueGroup::Satisfied
            } else if !held.is_empty() {
                QueueGroup::Blocking
            } else {
                QueueGroup::Open
            };
            // Closed and satisfied decisions are classified `none` whatever their lineage.
            let discovery = live
                .then(|| {
                    discovery_item(
                        inp.lineage
                            .get(decision.on())
                            .map_or(&[][..], Vec::as_slice),
                        inp.first_bucket,
                    )
                })
                .flatten();
            let (needed, needed_in_reason) = needed_in(&decision, &all, discovery);
            let step = next_step(&decision, inp.repos);
            QueueEntry {
                group,
                lane: lane(group, needed, step.as_ref()),
                needed_in: needed,
                needed_in_reason,
                next_step: step,
                held: held.iter().map(|s| story_of(s)).collect(),
                under_way: under_way.iter().map(|s| story_of(s)).collect(),
                decision,
            }
        })
        .collect();
    entries.sort_by(|a, b| {
        a.lane
            .cmp(&b.lane)
            .then_with(|| a.group.cmp(&b.group))
            .then_with(|| match a.group {
                QueueGroup::Blocking | QueueGroup::Open => urgency(a, b, today),
                QueueGroup::Satisfied => b
                    .decision
                    .checked_at()
                    .cmp(&a.decision.checked_at())
                    .then_with(|| a.decision.id().cmp(&b.decision.id())),
                QueueGroup::Closed => b
                    .decision
                    .closed()
                    .map(|c| c.at)
                    .cmp(&a.decision.closed().map(|c| c.at))
                    .then_with(|| a.decision.id().cmp(&b.decision.id())),
            })
    });
    let live = |e: &&QueueEntry| matches!(e.group, QueueGroup::Blocking | QueueGroup::Open);
    let in_lane = |l: QueueLane| entries.iter().filter(|e| e.lane == l).count() as u32;
    let stories: HashSet<&Key> = entries
        .iter()
        .filter(|e| e.group == QueueGroup::Blocking)
        .flat_map(|e| e.held.iter().map(|b| &b.key))
        .collect();
    let summary = QueueSummary {
        open: entries.iter().filter(live).count() as u32,
        blocking: entries
            .iter()
            .filter(|e| e.group == QueueGroup::Blocking)
            .count() as u32,
        blocked_stories: stories.len() as u32,
        overdue: entries
            .iter()
            .filter(live)
            .filter(|e| overdue(&e.decision, today))
            .count() as u32,
        review: in_lane(QueueLane::Review),
        this_sprint: in_lane(QueueLane::ThisSprint),
        next_sprint: in_lane(QueueLane::NextSprint),
    };
    (entries, summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::decision::{RecordRef, RequiredDecision};
    use chrono::{DateTime, NaiveDate, TimeZone, Utc};
    use reeve_protocol::{CheckResult, ClosureKind, ItemType, Key, Reason, TokenKind};

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 3).unwrap()
    }
    fn nov(d: u32) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(2026, 11, d)
    }
    fn oct(d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, d, 9, 0, 0).unwrap()
    }
    fn key(n: u64) -> Key {
        Key::new("ABC", n).unwrap()
    }
    fn repos() -> Vec<String> {
        vec!["kb".to_string()]
    }

    /// An open, unchecked decision due `due`, created on `created`, on epic ABC-1.
    fn dec(id: i64, due: Option<NaiveDate>, created: DateTime<Utc>) -> RequiredDecision {
        full(
            id,
            key(1),
            due,
            created,
            None,
            None,
            CheckResult::Unchecked,
            None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn full(
        id: i64,
        on: Key,
        due: Option<NaiveDate>,
        created: DateTime<Utc>,
        record: Option<RecordRef>,
        pr: Option<&str>,
        check: CheckResult,
        checked: Option<DateTime<Utc>>,
    ) -> RequiredDecision {
        RequiredDecision::from_persisted(
            id,
            on,
            ItemType::Epic,
            format!("d{id}"),
            "dan".into(),
            due,
            None,
            record,
            pr.map(String::from),
            check,
            checked,
            None,
            created,
            1,
        )
    }
    /// A decision whose record sits in `repo`, optionally with a PR.
    fn with_record(
        id: i64,
        repo: &str,
        pr: Option<&str>,
        check: CheckResult,
        checked: Option<DateTime<Utc>>,
    ) -> RequiredDecision {
        let r = RecordRef::new(repo, "adr/1.md", "ADR-1").unwrap();
        full(id, key(1), None, oct(1), Some(r), pr, check, checked)
    }
    fn accepted(mut d: RequiredDecision, checked: DateTime<Utc>) -> RequiredDecision {
        d.set_check_for_test(
            CheckResult::Accepted {
                commit: "c".into(),
                reversibility: None,
            },
            Some(checked),
        );
        d
    }
    fn closed(mut d: RequiredDecision, at: DateTime<Utc>) -> RequiredDecision {
        d.close(
            ClosureKind::Withdrawn,
            "done",
            "dan",
            TokenKind::Interactive,
            at,
        )
        .unwrap();
        d
    }
    fn story(n: u64, delivery: DeliveryState, reasons: Vec<Reason>) -> AffectedStory {
        AffectedStory {
            key: key(n),
            title: format!("s{n}"),
            kind: Kind::Story,
            delivery,
            owner: None,
            urgent: false,
            active_goals: vec![],
            draft_goals: vec![],
            promise: None,
            reasons,
        }
    }
    fn unchecked(id: i64) -> Reason {
        Reason::DecisionUnchecked {
            decision_id: id,
            title: format!("d{id}"),
        }
    }
    fn inputs<'a>(
        stories: &'a [AffectedStory],
        lineage: &'a HashMap<Key, Vec<LineageItem>>,
        repos: &'a [String],
    ) -> ClassifyInputs<'a> {
        ClassifyInputs {
            stories,
            lineage,
            first_bucket: Some("Now"),
            repos,
        }
    }
    /// Todo stories by number, each named by the decisions in `pairs` (decision id → story numbers).
    fn todo_for(pairs: &[(i64, &[u64])]) -> Vec<AffectedStory> {
        let mut by_story: BTreeMap<u64, Vec<Reason>> = BTreeMap::new();
        for (id, ss) in pairs {
            for n in *ss {
                by_story.entry(*n).or_default().push(unchecked(*id));
            }
        }
        by_story
            .into_iter()
            .map(|(n, r)| story(n, DeliveryState::Todo, r))
            .collect()
    }
    /// Runs the queue with no lineage and the "kb" repository configured.
    fn run(
        ds: Vec<RequiredDecision>,
        stories: &[AffectedStory],
    ) -> (Vec<QueueEntry>, QueueSummary) {
        let (l, r) = (HashMap::new(), repos());
        order(ds, &inputs(stories, &l, &r), today())
    }
    fn ids(es: &[QueueEntry]) -> Vec<(QueueGroup, i64)> {
        es.iter().map(|e| (e.group, e.decision.id())).collect()
    }
    fn block_keys(e: &QueueEntry) -> Vec<Key> {
        e.held.iter().map(|b| b.key.clone()).collect()
    }
    fn lineage_of(on: u64, items: Vec<LineageItem>) -> HashMap<Key, Vec<LineageItem>> {
        HashMap::from([(key(on), items)])
    }
    fn li(n: u64, level: ItemType, marked: bool, horizon: Option<&str>) -> LineageItem {
        LineageItem {
            key: key(n),
            level,
            discovery_marked: marked,
            initiative_horizon: horizon.map(String::from),
        }
    }
    fn needed_of(
        d: RequiredDecision,
        stories: &[AffectedStory],
        lineage: &HashMap<Key, Vec<LineageItem>>,
        bucket: Option<&str>,
    ) -> (NeededIn, Option<NeededInReason>) {
        let r = repos();
        let inp = ClassifyInputs {
            stories,
            lineage,
            first_bucket: bucket,
            repos: &r,
        };
        let (es, _) = order(vec![d], &inp, today());
        (es[0].needed_in, es[0].needed_in_reason.clone())
    }

    /// Groups and summary. d1 and d2 share story 12; d3 is overdue and blocks nothing; d4 is accepted and d5 closed,
    /// both with a due date long past. Catches an accepted or closed decision counted as open or overdue, a shared story
    /// counted twice, and blocks kept outside the blocking group.
    #[test]
    fn groups_and_summary() {
        let ds = vec![
            // closed and accepted, and held: closed wins
            closed(accepted(dec(5, nov(1), oct(5)), oct(20)), oct(21)),
            // accepted and held: satisfied wins, its story is not counted
            accepted(dec(4, nov(1), oct(4)), oct(20)),
            dec(3, nov(1), oct(3)),
            dec(2, None, oct(2)),
            // blocking and overdue
            dec(1, nov(2), oct(1)),
        ];
        let stories = todo_for(&[(1, &[11, 12]), (2, &[12, 13]), (4, &[14]), (5, &[15])]);
        let (es, s) = run(ds, &stories);
        assert_eq!(
            ids(&es),
            vec![
                (QueueGroup::Blocking, 1),
                (QueueGroup::Blocking, 2),
                (QueueGroup::Open, 3),
                (QueueGroup::Satisfied, 4),
                (QueueGroup::Closed, 5)
            ]
        );
        assert_eq!(block_keys(&es[0]), vec![key(11), key(12)]);
        assert!(es[2..].iter().all(|e| e.held.is_empty()));
        assert_eq!(
            (s.open, s.blocking, s.blocked_stories, s.overdue),
            (3, 2, 3, 2)
        );
    }

    /// Order within blocking and open: overdue first by due ascending; then blocked count descending; then undated by
    /// created_at; then dated by due ascending; ties by id. Catches blocked-first ordering (the approved-then-revised
    /// order), Option's None-first ordering leaking into rule 1, and a no-op sort (the input is reversed).
    #[test]
    fn order_within_open_groups() {
        let ds = vec![
            dec(9, None, oct(10)),   // open, undated, ties with 6 on created_at → by id
            dec(12, nov(3), oct(1)), // open, due today: not overdue, first of the dated
            dec(8, nov(4), oct(1)),  // open, dated
            dec(7, nov(5), oct(1)),  // open, dated, later
            dec(6, None, oct(10)),   // open, undated
            dec(5, None, oct(20)),   // open, undated, newer
            dec(4, nov(10), oct(1)), // blocking 3, dated
            dec(3, None, oct(1)),    // blocking 3, undated → before 4
            dec(
                2,
                Some(NaiveDate::from_ymd_opt(2026, 10, 30).unwrap()),
                oct(1),
            ), // open, overdue, earliest
            dec(1, nov(1), oct(1)),  // open, overdue
            dec(10, nov(20), oct(1)), // blocking 5 → before 3 and 4
            dec(11, nov(2), oct(1)), // blocking 2, overdue, later due than 13
            dec(13, nov(1), oct(1)), // blocking 1, overdue, earlier due → first despite fewer blocks
        ];
        let stories = todo_for(&[
            (3, &[21, 22, 23]),
            (4, &[24, 25, 26]),
            (10, &[31, 32, 33, 34, 35]),
            (11, &[41, 42]),
            (13, &[51]),
        ]);
        let (es, _) = run(ds, &stories);
        let b = QueueGroup::Blocking;
        let o = QueueGroup::Open;
        assert_eq!(
            ids(&es),
            vec![
                (b, 13),
                (b, 11),
                (b, 10),
                (b, 3),
                (b, 4),
                (o, 2),
                (o, 1),
                (o, 6),
                (o, 9),
                (o, 5),
                (o, 12),
                (o, 8),
                (o, 7)
            ]
        );
    }

    /// Inside one lane the group comes before urgency: a decision that holds work sorts above an overdue one that holds
    /// none. Both are needed next sprint (the held one by a draft goal, the other by discovery of epic ABC-1), so they
    /// share the lane. Catches urgency compared across groups.
    #[test]
    fn group_orders_before_urgency_inside_a_lane() {
        let mut s = story(7, DeliveryState::Todo, vec![unchecked(2)]);
        s.draft_goals = vec![(1, "G".into())];
        let l = lineage_of(1, vec![li(1, ItemType::Epic, false, Some("Now"))]);
        let r = repos();
        // Decision 1 is overdue and holds nothing; decision 2 is not due and holds story 7.
        let ds = vec![dec(1, nov(1), oct(1)), dec(2, nov(20), oct(1))];
        let stories = [s];
        let (es, _) = order(ds, &inputs(&stories, &l, &r), today());
        assert!(es.iter().all(|e| e.lane == QueueLane::NextSprint));
        assert_eq!(
            ids(&es),
            vec![(QueueGroup::Blocking, 2), (QueueGroup::Open, 1)]
        );
    }

    /// Satisfied: newest check first; closed: newest close first; ties by id. Catches oldest-first and a tie broken by
    /// input order.
    #[test]
    fn satisfied_and_closed_newest_first() {
        let ds = vec![
            accepted(dec(1, None, oct(1)), oct(20)),
            accepted(dec(3, None, oct(1)), oct(25)),
            accepted(dec(2, None, oct(1)), oct(25)),
            closed(dec(4, None, oct(1)), oct(21)),
            closed(dec(5, None, oct(1)), oct(28)),
        ];
        let (es, _) = run(ds, &[]);
        let (sa, cl) = (QueueGroup::Satisfied, QueueGroup::Closed);
        assert_eq!(ids(&es), vec![(sa, 2), (sa, 3), (sa, 1), (cl, 5), (cl, 4)]);
    }

    /// The fold reads decisions off readiness reasons only. Story 1 is held by an epic decision (1) and its own decision
    /// (2); story 2 by decision 1; story 3 only by a non-decision reason. Story 1 is listed under both decisions and the
    /// summary counts it once. Catches counting non-decision reasons and double counting a story.
    #[test]
    fn blocks_fold_reasons_by_decision() {
        let stories = vec![
            story(
                1,
                DeliveryState::Todo,
                vec![
                    unchecked(1),
                    Reason::DecisionNoRecord {
                        decision_id: 2,
                        title: "d2".into(),
                    },
                    Reason::NoCriteria,
                ],
            ),
            story(2, DeliveryState::Todo, vec![unchecked(1)]),
            story(3, DeliveryState::Todo, vec![Reason::Untriaged]),
            // Two reasons naming one decision list the story once under it.
            story(7, DeliveryState::Todo, vec![unchecked(3), unchecked(3)]),
        ];
        let ds = vec![
            dec(1, None, oct(1)),
            dec(2, None, oct(2)),
            dec(3, None, oct(3)),
        ];
        let (es, s) = run(ds, &stories);
        let by_id = |id: i64| es.iter().find(|e| e.decision.id() == id).unwrap();
        assert_eq!(block_keys(by_id(1)), vec![key(1), key(2)]);
        assert_eq!(block_keys(by_id(2)), vec![key(1)]);
        assert_eq!(block_keys(by_id(3)), vec![key(7)]);
        assert_eq!((s.blocking, s.blocked_stories), (3, 3));
    }

    /// The first rule wins: an active goal beats a draft goal on the same story. Catches draft goals checked before
    /// active ones.
    #[test]
    fn needed_in_first_rule_wins() {
        let mut s = story(1, DeliveryState::Todo, vec![unchecked(1)]);
        s.active_goals = vec![(7, "A".into())];
        s.draft_goals = vec![(8, "B".into())];
        let got = needed_of(dec(1, None, oct(1)), &[s], &HashMap::new(), Some("Now"));
        assert_eq!(
            got,
            (
                NeededIn::ThisSprint,
                Some(NeededInReason::Goal {
                    goal_id: 7,
                    outcome: "A".into()
                })
            )
        );
    }

    /// A draft goal alone gives next_sprint. Catches draft goals ignored.
    #[test]
    fn needed_in_draft_goal() {
        let mut s = story(1, DeliveryState::Todo, vec![unchecked(1)]);
        s.draft_goals = vec![(8, "B".into())];
        let got = needed_of(dec(1, None, oct(1)), &[s], &HashMap::new(), Some("Now"));
        assert_eq!(
            got,
            (
                NeededIn::NextSprint,
                Some(NeededInReason::Goal {
                    goal_id: 8,
                    outcome: "B".into()
                })
            )
        );
    }

    /// A decision on an unmarked epic, or on a story under a marked epic whose unmarked initiative is in the first
    /// bucket, is needed next sprint with no stories at all. Catches the counted-but-no-lane case, the `none` row winning
    /// over discovery, and a discovery rule that looks only at the parent.
    #[test]
    fn needed_in_discovery_without_stories() {
        let epic_case = lineage_of(
            5,
            vec![
                li(5, ItemType::Epic, false, Some("Now")),
                li(4, ItemType::Initiative, true, Some("Now")),
            ],
        );
        let d = full(
            2,
            key(5),
            None,
            oct(1),
            None,
            None,
            CheckResult::Unchecked,
            None,
        );
        let got = needed_of(d, &[], &epic_case, Some("Now"));
        assert_eq!(
            got,
            (
                NeededIn::NextSprint,
                Some(NeededInReason::Discovery {
                    key: key(5),
                    level: ItemType::Epic
                })
            )
        );
        // A story decision: the epic is marked, the initiative above it is not.
        let story_case = lineage_of(
            6,
            vec![
                li(5, ItemType::Epic, true, Some("Now")),
                li(4, ItemType::Initiative, false, Some("Now")),
            ],
        );
        let d = full(
            3,
            key(6),
            None,
            oct(1),
            None,
            None,
            CheckResult::Unchecked,
            None,
        );
        let got = needed_of(d, &[], &story_case, Some("Now"));
        assert_eq!(
            got,
            (
                NeededIn::NextSprint,
                Some(NeededInReason::Discovery {
                    key: key(4),
                    level: ItemType::Initiative
                })
            )
        );
    }

    /// An unmarked item whose initiative is in another bucket does not count. Catches any unmarked item counting.
    #[test]
    fn needed_in_discovery_skips_other_buckets() {
        let l = lineage_of(
            1,
            vec![
                li(1, ItemType::Epic, false, Some("Next")),
                li(4, ItemType::Initiative, false, Some("Next")),
            ],
        );
        let got = needed_of(dec(2, None, oct(1)), &[], &l, Some("Now"));
        assert_eq!(got, (NeededIn::None, None));
    }

    /// The bucket is the configured one. Catches the literal "Now".
    #[test]
    fn needed_in_discovery_uses_configured_first_bucket() {
        let l = lineage_of(1, vec![li(1, ItemType::Epic, false, Some("Soon"))]);
        let got = needed_of(dec(2, None, oct(1)), &[], &l, Some("Soon"));
        assert_eq!(
            got,
            (
                NeededIn::NextSprint,
                Some(NeededInReason::Discovery {
                    key: key(1),
                    level: ItemType::Epic
                })
            )
        );
        // With no bucket configured nothing is in discovery.
        let got = needed_of(dec(2, None, oct(1)), &[], &l, None);
        assert_eq!(got, (NeededIn::None, None));
    }

    /// An affected story with no goals gives beyond; nothing affected gives none; closed and satisfied give none even
    /// when stories are affected.
    #[test]
    fn needed_in_beyond_and_none() {
        let s = story(1, DeliveryState::Todo, vec![unchecked(1)]);
        let l = HashMap::new();
        assert_eq!(
            needed_of(
                dec(1, None, oct(1)),
                std::slice::from_ref(&s),
                &l,
                Some("Now")
            ),
            (NeededIn::Beyond, None)
        );
        assert_eq!(
            needed_of(
                dec(2, None, oct(1)),
                std::slice::from_ref(&s),
                &l,
                Some("Now")
            ),
            (NeededIn::None, None)
        );
        assert_eq!(
            needed_of(
                closed(dec(1, None, oct(1)), oct(2)),
                std::slice::from_ref(&s),
                &l,
                Some("Now")
            ),
            (NeededIn::None, None)
        );
        assert_eq!(
            needed_of(
                accepted(dec(1, None, oct(1)), oct(2)),
                &[s],
                &l,
                Some("Now")
            ),
            (NeededIn::None, None)
        );
    }

    /// A todo story is held, a started one is under way, a done one is neither. Catches done stories counted, and the
    /// todo/active split lost.
    #[test]
    fn affected_splits_held_and_under_way() {
        let stories = vec![
            story(1, DeliveryState::Todo, vec![unchecked(1)]),
            story(2, DeliveryState::InProgress, vec![unchecked(1)]),
            story(3, DeliveryState::Done, vec![unchecked(1)]),
        ];
        let (es, s) = run(vec![dec(1, None, oct(1))], &stories);
        assert_eq!(block_keys(&es[0]), vec![key(1)]);
        assert_eq!(
            es[0]
                .under_way
                .iter()
                .map(|u| u.key.clone())
                .collect::<Vec<_>>(),
            vec![key(2)]
        );
        assert_eq!(es[0].group, QueueGroup::Blocking);
        assert_eq!(s.blocked_stories, 1);
        // Only a started story: not held, so the group is open, yet it is still affected.
        let started = vec![story(2, DeliveryState::InProgress, vec![unchecked(1)])];
        let (es, _) = run(vec![dec(1, None, oct(1))], &started);
        assert_eq!(es[0].group, QueueGroup::Open);
        assert!(es[0].held.is_empty());
        assert_eq!(es[0].under_way.len(), 1);
        assert_eq!(es[0].needed_in, NeededIn::Beyond);
    }

    /// Every next-step row, in order. Catches a row order slip (for example `ConfigureRepo` losing to `AwaitCheck`).
    #[test]
    fn next_step_every_row() {
        let r = repos();
        let step = |d: RequiredDecision| next_step(&d, &r);
        assert_eq!(step(dec(1, None, oct(1))), Some(NextStep::WriteRecord));
        assert_eq!(
            step(with_record(1, "infra", None, CheckResult::Unchecked, None)),
            Some(NextStep::ConfigureRepo {
                repo: "infra".into()
            })
        );
        assert_eq!(
            step(with_record(1, "kb", None, CheckResult::NotFound, None)),
            Some(NextStep::FixRecord {
                result: CheckResult::NotFound
            })
        );
        let na = CheckResult::NotAccepted {
            status: "superseded".into(),
            superseded_by: Some("ADR-2".into()),
            commit: "c".into(),
            reversibility: None,
        };
        assert_eq!(
            step(with_record(1, "kb", None, na.clone(), None)),
            Some(NextStep::FixRecord { result: na })
        );
        let pr = "https://github.com/o/r/pull/1";
        for check in [CheckResult::ProposedPrOpen, CheckResult::Unchecked] {
            assert_eq!(
                step(with_record(1, "kb", Some(pr), check, None)),
                Some(NextStep::ReviewRecord { pr_url: pr.into() })
            );
        }
        assert_eq!(
            step(with_record(
                1,
                "kb",
                None,
                CheckResult::Unchecked,
                Some(oct(3))
            )),
            Some(NextStep::AwaitCheck {
                checked_at: Some(oct(3))
            })
        );
        // Proposed without a PR url cannot happen, but the type allows it.
        assert_eq!(
            step(with_record(
                1,
                "kb",
                None,
                CheckResult::ProposedPrOpen,
                None
            )),
            Some(NextStep::AwaitCheck { checked_at: None })
        );
        assert_eq!(step(accepted(dec(1, None, oct(1)), oct(2))), None);
        assert_eq!(step(closed(dec(1, None, oct(1)), oct(2))), None);
    }

    /// Lanes are disjoint and ordered, and the order is lane-first across groups. The input is scrambled and each lane
    /// with two entries has conflicting due, count and age, so a no-op or group-first sort fails. Catches a review lane
    /// that ignores the window (a beyond review stays beyond) and overlapping lanes.
    #[test]
    fn lanes_disjoint_and_ordered() {
        let pr = Some("https://github.com/o/r/pull/1");
        let rec = || Some(RecordRef::new("kb", "adr/1.md", "ADR-1").unwrap());
        let mk = |id: i64, due: Option<NaiveDate>, created, review: bool| {
            if review {
                full(
                    id,
                    key(1),
                    due,
                    created,
                    rec(),
                    pr,
                    CheckResult::Unchecked,
                    None,
                )
            } else {
                full(
                    id,
                    key(1),
                    due,
                    created,
                    None,
                    None,
                    CheckResult::Unchecked,
                    None,
                )
            }
        };
        let ds = vec![
            closed(dec(70, None, oct(1)), oct(9)),
            mk(40, nov(5), oct(5), false), // beyond, 1 story, dated
            mk(10, nov(20), oct(1), true), // review (this sprint), 1 story
            accepted(dec(60, None, oct(1)), oct(9)),
            dec(50, None, oct(1)),         // open: no stories
            mk(21, None, oct(7), false),   // this sprint, 2 stories
            mk(41, None, oct(1), true),    // beyond review: stays beyond
            mk(30, None, oct(1), false),   // next sprint
            mk(20, None, oct(1), false),   // this sprint, 1 story, older
            mk(11, nov(25), oct(9), true), // review, 3 stories
        ];
        let mut stories = todo_for(&[
            (10, &[1]),
            (11, &[2, 3, 4]),
            (20, &[5]),
            (21, &[6, 7]),
            (30, &[8]),
            (40, &[9]),
            (41, &[10]),
        ]);
        for s in &mut stories {
            let n = s.key.number();
            if [1, 2, 3, 4, 5, 6, 7].contains(&n) {
                s.active_goals = vec![(1, "G".into())];
            } else if n == 8 {
                s.draft_goals = vec![(2, "H".into())];
            }
        }
        stories.reverse();
        let (es, s) = run(ds, &stories);
        assert_eq!(es.len(), 10);
        let got: Vec<(QueueLane, i64)> = es.iter().map(|e| (e.lane, e.decision.id())).collect();
        use QueueLane::*;
        assert_eq!(
            got,
            vec![
                (Review, 11),
                (Review, 10),
                (ThisSprint, 21),
                (ThisSprint, 20),
                (NextSprint, 30),
                (Beyond, 41),
                (Beyond, 40),
                (Open, 50),
                (Satisfied, 60),
                (Closed, 70)
            ]
        );
        assert_eq!((s.review, s.this_sprint, s.next_sprint), (2, 2, 1));
        // Lane-first: a blocking entry in a later lane follows an open-group entry in an earlier one.
        let started = vec![{
            let mut a = story(1, DeliveryState::InProgress, vec![unchecked(1)]);
            a.active_goals = vec![(1, "G".into())];
            a
        }];
        let mut b = story(2, DeliveryState::Todo, vec![unchecked(2)]);
        b.draft_goals = vec![(2, "H".into())];
        let both = [started, vec![b]].concat();
        let (es, _) = run(vec![dec(2, None, oct(1)), dec(1, None, oct(2))], &both);
        assert_eq!(
            es.iter().map(|e| (e.group, e.lane)).collect::<Vec<_>>(),
            vec![
                (QueueGroup::Open, QueueLane::ThisSprint),
                (QueueGroup::Blocking, QueueLane::NextSprint)
            ]
        );
    }
    /// A promise held by story `n` itself, on `on`, asking for `window`.
    fn own_promise(
        n: u64,
        on: NaiveDate,
        window: NeededIn,
        overdue: bool,
    ) -> Option<PromiseWindow> {
        Some(PromiseWindow {
            key: key(n),
            level: ItemType::Story,
            on,
            window,
            overdue,
        })
    }
    fn promise_reason(n: u64, level: ItemType, on: NaiveDate, overdue: bool) -> NeededInReason {
        NeededInReason::Promise {
            key: key(n),
            level,
            on,
            overdue,
        }
    }
    fn needed(stories: &[AffectedStory], discovery_on: bool) -> (NeededIn, Option<NeededInReason>) {
        let l = if discovery_on {
            lineage_of(1, vec![li(1, ItemType::Epic, false, Some("Now"))])
        } else {
            HashMap::new()
        };
        needed_of(dec(1, None, oct(1)), stories, &l, Some("Now"))
    }

    /// An overdue promise on one story outranks a draft goal on another: the earliest lane wins across rules. Catches
    /// the old first-rule order (goals before anything else) surviving.
    #[test]
    fn an_overdue_promise_beats_a_draft_goal() {
        let mut a = story(1, DeliveryState::Todo, vec![unchecked(1)]);
        a.draft_goals = vec![(8, "B".into())];
        let mut b = story(2, DeliveryState::Todo, vec![unchecked(1)]);
        b.promise = own_promise(2, nov(1).unwrap(), NeededIn::ThisSprint, true);
        assert_eq!(
            needed(&[a, b], true),
            (
                NeededIn::ThisSprint,
                Some(promise_reason(2, ItemType::Story, nov(1).unwrap(), true))
            )
        );
    }

    /// A promise due next sprint does not pull a decision an active goal already needs this sprint. Catches the
    /// promise replacing an earlier reason.
    #[test]
    fn a_promise_due_next_sprint_keeps_an_active_goal() {
        let mut a = story(1, DeliveryState::Todo, vec![unchecked(1)]);
        a.active_goals = vec![(7, "A".into())];
        let mut b = story(2, DeliveryState::Todo, vec![unchecked(1)]);
        b.promise = own_promise(2, nov(20).unwrap(), NeededIn::NextSprint, false);
        assert_eq!(
            needed(&[a, b], false),
            (
                NeededIn::ThisSprint,
                Some(NeededInReason::Goal {
                    goal_id: 7,
                    outcome: "A".into()
                })
            )
        );
    }

    /// Inside one lane the reason is the goal, then discovery, then the promise. Catches the promise winning a tie.
    #[test]
    fn same_lane_prefers_goal_then_discovery_then_promise() {
        let mut a = story(1, DeliveryState::Todo, vec![unchecked(1)]);
        a.draft_goals = vec![(8, "B".into())];
        let mut b = story(2, DeliveryState::Todo, vec![unchecked(1)]);
        b.promise = own_promise(2, nov(20).unwrap(), NeededIn::NextSprint, false);
        let goal = NeededInReason::Goal {
            goal_id: 8,
            outcome: "B".into(),
        };
        assert_eq!(
            needed(&[a, b.clone()], true),
            (NeededIn::NextSprint, Some(goal))
        );
        // Discovery and a next-sprint promise: discovery.
        assert_eq!(
            needed(&[b], true),
            (
                NeededIn::NextSprint,
                Some(NeededInReason::Discovery {
                    key: key(1),
                    level: ItemType::Epic
                })
            )
        );
    }

    /// A promise due later (window `none`) gives no reason: the decision is beyond, as for any affected story. This
    /// pins the outcome only: without `needed_in`'s window filter the `beyond` candidate would still win, because the
    /// promised story is itself affected and `beyond` sorts before `none`. The filter is what keeps a promise from ever
    /// being the reason outside this sprint and next sprint.
    #[test]
    fn a_later_promise_gives_beyond_without_a_reason() {
        let mut a = story(1, DeliveryState::Todo, vec![unchecked(1)]);
        a.promise = own_promise(1, nov(30).unwrap(), NeededIn::None, false);
        assert_eq!(needed(&[a], false), (NeededIn::Beyond, None));
    }

    /// A story inheriting its epic's promise gives a reason naming the epic, not the story. Catches the story's key
    /// substituted for the holder's.
    #[test]
    fn the_reason_names_the_epic_when_inherited() {
        let mut a = story(7, DeliveryState::Todo, vec![unchecked(1)]);
        a.promise = Some(PromiseWindow {
            key: key(5),
            level: ItemType::Epic,
            on: nov(10).unwrap(),
            window: NeededIn::ThisSprint,
            overdue: false,
        });
        assert_eq!(
            needed(&[a], false),
            (
                NeededIn::ThisSprint,
                Some(promise_reason(5, ItemType::Epic, nov(10).unwrap(), false))
            )
        );
    }

    /// Among promised stories the earliest lane wins, then the earliest date, then the lowest key number. The input
    /// is ordered against each rule. Catches a first-found or date-only pick.
    #[test]
    fn earliest_lane_then_date_then_key_number_picks_the_story() {
        let mk = |n: u64, d: u32, w: NeededIn| {
            let mut s = story(n, DeliveryState::Todo, vec![unchecked(1)]);
            s.promise = own_promise(n, nov(d).unwrap(), w, false);
            s
        };
        // Next sprint with the earliest date loses to this sprint.
        let stories = [
            mk(1, 2, NeededIn::NextSprint),
            mk(2, 9, NeededIn::ThisSprint),
        ];
        assert_eq!(
            needed(&stories, false).1,
            Some(promise_reason(2, ItemType::Story, nov(9).unwrap(), false))
        );
        // Same lane: the earlier date.
        let stories = [
            mk(3, 9, NeededIn::ThisSprint),
            mk(4, 8, NeededIn::ThisSprint),
        ];
        assert_eq!(
            needed(&stories, false).1,
            Some(promise_reason(4, ItemType::Story, nov(8).unwrap(), false))
        );
        // Same lane and date: the lower key number, compared as a number ("ABC-10" sorts before "ABC-9" as text).
        let stories = [
            mk(10, 9, NeededIn::NextSprint),
            mk(9, 9, NeededIn::NextSprint),
        ];
        assert_eq!(
            needed(&stories, false),
            (
                NeededIn::NextSprint,
                Some(promise_reason(9, ItemType::Story, nov(9).unwrap(), false))
            )
        );
    }

    /// A held story's own window is the earlier of its goals' and its promise's; a promise of `none` does not count.
    /// Catches the story's window read from goals only.
    #[test]
    fn story_window_counts_the_story_s_own_promise() {
        let mut a = story(1, DeliveryState::Todo, vec![unchecked(1)]);
        a.draft_goals = vec![(8, "B".into())];
        a.promise = own_promise(1, nov(1).unwrap(), NeededIn::ThisSprint, true);
        let mut b = story(2, DeliveryState::Todo, vec![unchecked(1)]);
        b.promise = own_promise(2, nov(30).unwrap(), NeededIn::None, false);
        let mut c = story(3, DeliveryState::Todo, vec![unchecked(1)]);
        c.active_goals = vec![(7, "A".into())];
        c.promise = own_promise(3, nov(20).unwrap(), NeededIn::NextSprint, false);
        let (es, _) = run(vec![dec(1, None, oct(1))], &[a, b, c]);
        let windows: Vec<(Key, NeededIn)> = es[0]
            .held
            .iter()
            .map(|s| (s.key.clone(), s.needed_in))
            .collect();
        assert_eq!(
            windows,
            vec![
                (key(1), NeededIn::ThisSprint),
                (key(2), NeededIn::Beyond),
                (key(3), NeededIn::ThisSprint)
            ]
        );
    }
}
