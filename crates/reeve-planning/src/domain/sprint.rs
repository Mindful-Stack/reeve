//! Sprints and goals (kb/ADR-0014): pure rules and the summary computation. No I/O and no clock: every instant comes
//! from the handler's `Ctx.now`.
use chrono::{DateTime, Days, NaiveDate, Utc};
use reeve_protocol::{
    Confidence, DeliveryState, DemoStoryDto, GoalProgressDto, GoalVia, Key, Reason, Resolution,
    SprintCountsDto, SprintState, SprintWarning, Verdict,
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

/// The counts (the protocol's one counts type: no domain twin, and no serde in planning) and the demo list (the
/// protocol's type too, oldest completion first).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    pub counts: SprintCountsDto,
    pub demo: Vec<DemoStoryDto>,
}

impl Summary {
    pub fn counts(&self) -> SprintCountsDto {
        self.counts.clone()
    }
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
    let mut goals: Vec<GoalProgressDto> = goal_ids
        .iter()
        .map(|&goal_id| GoalProgressDto {
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
        .map(|s| DemoStoryDto {
            key: s.key.clone(),
            title: s.title.clone(),
        })
        .collect();
    out.counts.goals = goals;
    out
}

/// A goal on the wire, with its links (any order in `links`; only this goal's are taken, in their given order).
pub fn goal_dto(g: &Goal, links: &[GoalLink]) -> reeve_protocol::GoalDto {
    reeve_protocol::GoalDto {
        id: g.id,
        sprint_id: g.sprint_id,
        outcome: g.outcome.clone(),
        check: g.check.clone(),
        owner: g.owner.clone(),
        notes: g.notes.clone(),
        confidence: g.confidence,
        confidence_note: g.confidence_note.clone(),
        confidence_at: g.confidence_at,
        suggested_confidence: g.suggested_confidence,
        suggested_note: g.suggested_note.clone(),
        suggested_at: g.suggested_at,
        suggested_by: g.suggested_by.clone(),
        verdict: g.verdict,
        verdict_note: g.verdict_note.clone(),
        version: g.version,
        links: links
            .iter()
            .filter(|l| l.goal_id == g.id)
            .map(|l| l.item_key.clone())
            .collect(),
    }
}

/// A sprint on the wire with its goals (taken from `goals` by sprint id) and, for a draft or the active sprint, the
/// warnings of its goal count.
pub fn sprint_dto(s: &Sprint, goals: &[Goal], links: &[GoalLink]) -> reeve_protocol::SprintDto {
    let goals: Vec<reeve_protocol::GoalDto> = goals
        .iter()
        .filter(|g| g.sprint_id == s.id)
        .map(|g| goal_dto(g, links))
        .collect();
    reeve_protocol::SprintDto {
        id: s.id,
        state: s.state,
        starts_on: s.starts_on,
        ends_on: s.ends_on,
        opened_at: s.opened_at,
        closed_at: s.closed_at,
        version: s.version,
        warnings: match s.state {
            SprintState::Closed => vec![],
            SprintState::Draft | SprintState::Active => warnings(goals.len()),
        },
        goals,
    }
}

/// A sprint with its numbers; `summary` is None for a draft (no window), which shows zero counts and no demo list.
pub fn sprint_view_dto(
    s: &Sprint,
    goals: &[Goal],
    links: &[GoalLink],
    summary: Option<&Summary>,
    upkeep_line: u8,
) -> reeve_protocol::SprintViewDto {
    reeve_protocol::SprintViewDto {
        sprint: sprint_dto(s, goals, links),
        counts: summary.map(Summary::counts).unwrap_or_default(),
        upkeep_line,
        demo: summary.map(|x| x.demo.clone()).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone, Utc};

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
    fn summary_counts_accessor_returns_the_counts() {
        let s = summarise(&[1], &[], &[], &Default::default(), t(3, 18), t(11, 18));
        assert_eq!(s.counts().goals.len(), 1);
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
}
