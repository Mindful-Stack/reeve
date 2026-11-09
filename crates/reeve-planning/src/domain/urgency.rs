//! Urgency (kb/ADR-0017): which items the stored urgent marks make urgent. Pure: the stored marks, their lineage and
//! the handler's clock in; per-item sources out. Nothing here reads the clock or the database.
use chrono::{DateTime, Duration, Utc};
use reeve_protocol::{ItemType, Key};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct UrgentMark {
    pub at: DateTime<Utc>,
    pub by: String,
    pub reason: Option<String>,
}

/// A stored mark and the facts about its own item.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkFact {
    pub item: ItemType,
    pub key: Key,
    pub mark: UrgentMark,
    /// A story's epic or an epic's initiative; None for an initiative and for a missing parent.
    pub parent: Option<Key>,
    /// A story in delivery `done`; always false for epics and initiatives.
    pub done: bool,
}

/// The hierarchy around the marks: the epics a mark reaches (its own, a marked story's, those under a marked
/// initiative) with their initiative, and the stories under marked epics and under marked initiatives' epics.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lineage {
    pub epics: Vec<(Key, Option<Key>)>,
    /// (story, epic, done)
    pub stories: Vec<(Key, Key, bool)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MarkState {
    pub item: ItemType,
    pub mark: UrgentMark,
    pub expires_at: DateTime<Utc>,
    pub active: bool,
    pub done: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Urgency {
    marks: HashMap<Key, MarkState>,
    sources: HashMap<Key, Vec<(Key, ItemType)>>,
}

impl Urgency {
    /// The mark stored on `key`, active or expired.
    pub fn mark(&self, key: &Key) -> Option<&MarkState> {
        self.marks.get(key)
    }
    /// The active marks making `key` urgent: its own first, then in key-number order. Empty: not urgent.
    pub fn sources(&self, key: &Key) -> &[(Key, ItemType)] {
        self.sources.get(key).map(Vec::as_slice).unwrap_or(&[])
    }
    pub fn marks(&self) -> impl Iterator<Item = (&Key, &MarkState)> {
        self.marks.iter()
    }
}

/// When a mark set at `at` stops being active: one cycle later (kb/ADR-0017 R2).
pub fn expires_at(at: DateTime<Utc>, length_days: u32) -> DateTime<Utc> {
    at + Duration::days(length_days.into())
}

pub fn urgency(
    marks: &[MarkFact],
    lineage: &Lineage,
    now: DateTime<Utc>,
    length_days: u32,
) -> Urgency {
    let initiative_of: HashMap<&Key, &Key> = lineage
        .epics
        .iter()
        .filter_map(|(e, i)| i.as_ref().map(|i| (e, i)))
        .collect();
    let mut out = Urgency::default();
    for m in marks {
        let expires = expires_at(m.mark.at, length_days);
        let active = now < expires;
        out.marks.insert(
            m.key.clone(),
            MarkState {
                item: m.item,
                mark: m.mark.clone(),
                expires_at: expires,
                active,
                done: m.done,
            },
        );
        if !active || m.done {
            continue;
        }
        let mut targets: Vec<&Key> = vec![&m.key];
        match m.item {
            ItemType::Story => {
                if let Some(e) = &m.parent {
                    targets.push(e);
                    if let Some(i) = initiative_of.get(e) {
                        targets.push(i);
                    }
                }
            }
            ItemType::Epic => {
                targets.extend(m.parent.iter());
                targets.extend(
                    lineage
                        .stories
                        .iter()
                        .filter(|(_, e, d)| e == &m.key && !d)
                        .map(|(s, _, _)| s),
                );
            }
            ItemType::Initiative => {
                let epics: Vec<&Key> = lineage
                    .epics
                    .iter()
                    .filter(|(_, i)| i.as_ref() == Some(&m.key))
                    .map(|(e, _)| e)
                    .collect();
                targets.extend(epics.iter().copied());
                targets.extend(
                    lineage
                        .stories
                        .iter()
                        .filter(|(_, e, d)| !d && epics.contains(&e))
                        .map(|(s, _, _)| s),
                );
            }
        }
        for t in targets {
            let v = out.sources.entry(t.clone()).or_default();
            if !v.iter().any(|(s, _)| s == &m.key) {
                v.push((m.key.clone(), m.item));
            }
        }
    }
    for (k, v) in out.sources.iter_mut() {
        v.sort_by_key(|(s, _)| (s != k, s.number()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn k(n: u64) -> Key {
        format!("ABC-{n}").parse().unwrap()
    }
    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 9, 9, 0, 0).unwrap()
    }
    fn mark(item: ItemType, n: u64, parent: Option<u64>, done: bool) -> MarkFact {
        MarkFact {
            item,
            key: k(n),
            mark: UrgentMark {
                at: t0(),
                by: "dan@example.com".into(),
                reason: None,
            },
            parent: parent.map(k),
            done,
        }
    }
    /// I1 ⊃ E2 ⊃ {S3, S4 (done)}; I1 ⊃ E5 ⊃ S6; E7 (no initiative) ⊃ S8.
    fn tree() -> Lineage {
        Lineage {
            epics: vec![(k(2), Some(k(1))), (k(5), Some(k(1))), (k(7), None)],
            stories: vec![
                (k(3), k(2), false),
                (k(4), k(2), true),
                (k(6), k(5), false),
                (k(8), k(7), false),
            ],
        }
    }
    fn urgent(u: &Urgency) -> Vec<u64> {
        let mut v: Vec<u64> = u.sources.keys().map(|k| k.number()).collect();
        v.sort();
        v
    }

    #[test]
    fn a_story_mark_spreads_up_never_sideways() {
        let u = urgency(
            &[mark(ItemType::Story, 3, Some(2), false)],
            &tree(),
            t0(),
            14,
        );
        assert_eq!(
            urgent(&u),
            [1, 2, 3],
            "S6 under the same initiative and S4 beside it stay calm"
        );
    }
    #[test]
    fn an_epic_mark_spreads_up_and_down_but_skips_done_stories() {
        let u = urgency(
            &[mark(ItemType::Epic, 2, Some(1), false)],
            &tree(),
            t0(),
            14,
        );
        assert_eq!(urgent(&u), [1, 2, 3], "S4 is done; E5 is a sibling");
    }
    #[test]
    fn an_initiative_mark_spreads_down_through_every_epic() {
        let u = urgency(
            &[mark(ItemType::Initiative, 1, None, false)],
            &tree(),
            t0(),
            14,
        );
        assert_eq!(urgent(&u), [1, 2, 3, 5, 6]);
    }
    #[test]
    fn missing_parents_shorten_the_lineage() {
        let u = urgency(
            &[mark(ItemType::Story, 8, Some(7), false)],
            &tree(),
            t0(),
            14,
        );
        assert_eq!(urgent(&u), [7, 8]);
        let orphan = urgency(
            &[mark(ItemType::Story, 9, None, false)],
            &Lineage::default(),
            t0(),
            14,
        );
        assert_eq!(urgent(&orphan), [9]);
    }
    #[test]
    fn a_mark_on_a_done_story_makes_nothing_urgent_but_is_still_listed() {
        let u = urgency(
            &[mark(ItemType::Story, 4, Some(2), true)],
            &tree(),
            t0(),
            14,
        );
        assert!(urgent(&u).is_empty());
        let m = u.mark(&k(4)).unwrap();
        assert!(m.active && m.done);
    }
    #[test]
    fn a_mark_expires_exactly_one_cycle_after_it_was_set() {
        let m = [mark(ItemType::Initiative, 1, None, false)];
        let edge = t0() + Duration::days(14);
        assert!(
            urgency(&m, &tree(), edge - Duration::seconds(1), 14)
                .mark(&k(1))
                .unwrap()
                .active
        );
        let at_edge = urgency(&m, &tree(), edge, 14);
        assert!(!at_edge.mark(&k(1)).unwrap().active);
        assert!(
            urgent(&at_edge).is_empty(),
            "an expired mark spreads nothing"
        );
        assert_eq!(at_edge.mark(&k(1)).unwrap().expires_at, edge);
    }
    #[test]
    fn sources_are_deduplicated_own_first_then_key_order() {
        let marks = [
            mark(ItemType::Story, 6, Some(5), false),
            mark(ItemType::Initiative, 1, None, false),
            mark(ItemType::Epic, 5, Some(1), false),
            mark(ItemType::Story, 3, Some(2), false),
        ];
        let u = urgency(&marks, &tree(), t0(), 14);
        let nums = |key: u64| {
            u.sources(&k(key))
                .iter()
                .map(|(s, _)| s.number())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            nums(5),
            [5, 1, 6],
            "own first, then key order; no duplicates"
        );
        assert_eq!(nums(1), [1, 3, 5, 6]);
        assert_eq!(nums(6), [6, 1, 5]);
        assert_eq!(
            u.sources(&k(6))[1].1,
            ItemType::Initiative,
            "sources carry their item type"
        );
    }
}
