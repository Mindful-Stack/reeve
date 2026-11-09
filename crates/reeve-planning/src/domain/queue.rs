//! The decision queue: pure, over decisions and the readiness the loader already computed. Nothing here
//! derives readiness (kb/ADR-0003 R1, kb/ADR-0013 R4); blocked stories are read off the reasons.

use crate::domain::decision::RequiredDecision;
use chrono::NaiveDate;
use reeve_protocol::{
    BlockedStoryDto, DecisionQueueEntryDto, DecisionQueueSummaryDto, Key, QueueGroup,
    RequiredDecisionDto, StoryWithReadiness,
};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};

/// A todo story a decision holds back.
#[derive(Debug, PartialEq, Eq)]
pub struct BlockedStory {
    pub key: Key,
    pub title: String,
}

/// One decision in the queue, with the todo stories it blocks (empty outside the blocking group).
#[derive(Debug)]
pub struct QueueEntry {
    pub group: QueueGroup,
    pub decision: RequiredDecision,
    pub blocks: Vec<BlockedStory>,
}

/// Counts over open, unsatisfied decisions only; `blocked_stories` counts each story once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueSummary {
    pub open: u32,
    pub blocking: u32,
    pub blocked_stories: u32,
    pub overdue: u32,
}

impl From<&BlockedStory> for BlockedStoryDto {
    fn from(b: &BlockedStory) -> Self {
        Self {
            key: b.key.clone(),
            title: b.title.clone(),
        }
    }
}

impl From<&QueueEntry> for DecisionQueueEntryDto {
    fn from(e: &QueueEntry) -> Self {
        Self {
            decision: RequiredDecisionDto::from(&e.decision),
            group: e.group,
            blocks: e.blocks.iter().map(BlockedStoryDto::from).collect(),
        }
    }
}

impl From<QueueSummary> for DecisionQueueSummaryDto {
    fn from(s: QueueSummary) -> Self {
        Self {
            open: s.open,
            blocking: s.blocking,
            blocked_stories: s.blocked_stories,
            overdue: s.overdue,
        }
    }
}

/// decision id → the todo stories whose readiness reasons name it, in input order.
pub fn blocks(todo: &[StoryWithReadiness]) -> BTreeMap<i64, Vec<BlockedStory>> {
    let mut out: BTreeMap<i64, Vec<BlockedStory>> = BTreeMap::new();
    for s in todo {
        let mut seen = HashSet::new();
        for id in s.readiness.reasons.iter().filter_map(|r| r.decision_id()) {
            if seen.insert(id) {
                out.entry(id).or_default().push(BlockedStory {
                    key: s.story.key.clone(),
                    title: s.story.title.clone(),
                });
            }
        }
    }
    out
}

fn overdue(d: &RequiredDecision, today: NaiveDate) -> bool {
    d.due.is_some_and(|due| due < today)
}

/// Urgency order inside the blocking and open groups.
fn urgency(a: &QueueEntry, b: &QueueEntry, today: NaiveDate) -> Ordering {
    let (da, db) = (&a.decision, &b.decision);
    let (oa, ob) = (overdue(da, today), overdue(db, today));
    ob.cmp(&oa) // overdue first
        .then_with(|| {
            if oa && ob {
                da.due.cmp(&db.due)
            } else {
                Ordering::Equal
            }
        })
        .then_with(|| b.blocks.len().cmp(&a.blocks.len()))
        .then_with(|| da.due.is_some().cmp(&db.due.is_some())) // undated first
        .then_with(|| match (da.due, db.due) {
            (Some(x), Some(y)) => x.cmp(&y),
            _ => da.created_at.cmp(&db.created_at),
        })
        .then_with(|| da.id.cmp(&db.id))
}

/// Groups, orders and counts `ds`; takes each decision's blocks out of `held`.
pub fn order(
    ds: Vec<RequiredDecision>,
    mut held: BTreeMap<i64, Vec<BlockedStory>>,
    today: NaiveDate,
) -> (Vec<QueueEntry>, QueueSummary) {
    let mut entries: Vec<QueueEntry> = ds
        .into_iter()
        .map(|decision| {
            let stuck = held.remove(&decision.id).unwrap_or_default();
            let (group, blocks) = if decision.closed.is_some() {
                (QueueGroup::Closed, vec![])
            } else if decision.is_satisfied() {
                (QueueGroup::Satisfied, vec![])
            } else if !stuck.is_empty() {
                (QueueGroup::Blocking, stuck)
            } else {
                (QueueGroup::Open, vec![])
            };
            QueueEntry {
                group,
                decision,
                blocks,
            }
        })
        .collect();
    entries.sort_by(|a, b| {
        a.group.cmp(&b.group).then_with(|| match a.group {
            QueueGroup::Blocking | QueueGroup::Open => urgency(a, b, today),
            QueueGroup::Satisfied => b
                .decision
                .checked_at
                .cmp(&a.decision.checked_at)
                .then_with(|| a.decision.id.cmp(&b.decision.id)),
            QueueGroup::Closed => b
                .decision
                .closed
                .as_ref()
                .map(|c| c.at)
                .cmp(&a.decision.closed.as_ref().map(|c| c.at))
                .then_with(|| a.decision.id.cmp(&b.decision.id)),
        })
    });
    let live = |e: &&QueueEntry| matches!(e.group, QueueGroup::Blocking | QueueGroup::Open);
    let stories: HashSet<&Key> = entries
        .iter()
        .filter(|e| e.group == QueueGroup::Blocking)
        .flat_map(|e| e.blocks.iter().map(|b| &b.key))
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
    };
    (entries, summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::decision::{Closed, RequiredDecision};
    use chrono::{DateTime, NaiveDate, TimeZone, Utc};
    use reeve_protocol::{
        CheckResult, ClosureKind, DeliveryState, ItemType, Key, Kind, Readiness, Reason, StoryDto,
        StoryWithReadiness, TokenKind,
    };
    use std::collections::BTreeMap;

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

    /// An open, unchecked decision due `due`, created on `created`.
    fn dec(id: i64, due: Option<NaiveDate>, created: DateTime<Utc>) -> RequiredDecision {
        RequiredDecision {
            id,
            on: key(1),
            on_type: ItemType::Epic,
            title: format!("d{id}"),
            decider: "dan".into(),
            due,
            reversibility: None,
            record: None,
            pr_url: None,
            check: CheckResult::Unchecked,
            checked_at: None,
            closed: None,
            created_at: created,
            version: 1,
        }
    }
    fn accepted(mut d: RequiredDecision, checked: DateTime<Utc>) -> RequiredDecision {
        d.check = CheckResult::Accepted {
            commit: "c".into(),
            reversibility: None,
        };
        d.checked_at = Some(checked);
        d
    }
    fn closed(mut d: RequiredDecision, at: DateTime<Utc>) -> RequiredDecision {
        d.closed = Some(Closed {
            at,
            by: "dan".into(),
            reason: "done".into(),
            kind: Some(ClosureKind::Withdrawn),
            via: Some(TokenKind::Interactive),
        });
        d
    }
    /// decision id → the stories (by number) it blocks.
    fn held(pairs: &[(i64, &[u64])]) -> BTreeMap<i64, Vec<BlockedStory>> {
        pairs
            .iter()
            .map(|(id, ss)| {
                (
                    *id,
                    ss.iter()
                        .map(|n| BlockedStory {
                            key: key(*n),
                            title: format!("s{n}"),
                        })
                        .collect(),
                )
            })
            .collect()
    }
    fn ids(es: &[QueueEntry]) -> Vec<(QueueGroup, i64)> {
        es.iter().map(|e| (e.group, e.decision.id)).collect()
    }
    fn todo(n: u64, reasons: Vec<Reason>) -> StoryWithReadiness {
        StoryWithReadiness {
            story: StoryDto {
                key: key(n),
                title: format!("s{n}"),
                kind: Kind::Story,
                epic: None,
                description: String::new(),
                criteria: vec![],
                owner: None,
                labels: vec![],
                rank: 0,
                blocked_by: vec![],
                delivery: DeliveryState::Todo,
                resolution: None,
                external_ref: None,
                version: 1,
                prs: vec![],
                goals: vec![],
                urgent_mark: None,
                urgent_sources: vec![],
            },
            readiness: Readiness {
                ready: reasons.is_empty(),
                reasons,
            },
        }
    }
    fn unchecked(id: i64) -> Reason {
        Reason::DecisionUnchecked {
            decision_id: id,
            title: format!("d{id}"),
        }
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
        let (es, s) = order(
            ds,
            held(&[(1, &[11, 12]), (2, &[12, 13]), (4, &[14]), (5, &[15])]),
            today(),
        );
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
        assert_eq!(es[0].blocks, held(&[(0, &[11, 12])])[&0]);
        assert!(es[2..].iter().all(|e| e.blocks.is_empty()));
        assert_eq!(
            s,
            QueueSummary {
                open: 3,
                blocking: 2,
                blocked_stories: 3,
                overdue: 2
            }
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
        let (es, _) = order(
            ds,
            held(&[
                (3, &[21, 22, 23]),
                (4, &[24, 25, 26]),
                (10, &[31, 32, 33, 34, 35]),
                (11, &[41, 42]),
                (13, &[51]),
            ]),
            today(),
        );
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
        let (es, _) = order(ds, BTreeMap::new(), today());
        let (sa, cl) = (QueueGroup::Satisfied, QueueGroup::Closed);
        assert_eq!(ids(&es), vec![(sa, 2), (sa, 3), (sa, 1), (cl, 5), (cl, 4)]);
    }

    /// The fold reads decisions off readiness reasons only. Story 1 is held by an epic decision (1) and its own decision
    /// (2); story 2 by decision 1; story 3 only by a non-decision reason. Story 1 is listed under both decisions and the
    /// summary counts it once. Catches counting non-decision reasons and double counting a story.
    #[test]
    fn blocks_fold_reasons_by_decision() {
        let stories = vec![
            todo(
                1,
                vec![
                    unchecked(1),
                    Reason::DecisionNoRecord {
                        decision_id: 2,
                        title: "d2".into(),
                    },
                    Reason::NoCriteria,
                ],
            ),
            todo(2, vec![unchecked(1)]),
            todo(3, vec![Reason::Untriaged]),
        ];
        let b = blocks(&stories);
        assert_eq!(b, held(&[(1, &[1, 2]), (2, &[1])]));
        // Two reasons naming one decision list the story once under it.
        let twice = vec![todo(7, vec![unchecked(3), unchecked(3)])];
        assert_eq!(blocks(&twice), held(&[(3, &[7])]));
        let (_, s) = order(vec![dec(1, None, oct(1)), dec(2, None, oct(2))], b, today());
        assert_eq!((s.blocking, s.blocked_stories), (2, 2));
    }
}
