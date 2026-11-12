//! Characterisation tests for the DTOs built from domain values (spec 2026-11-09, pass 0). Each value has every
//! optional field populated and distinct, so a swapped or dropped field shows. Their expected values must not change.
use chrono::{NaiveDate, TimeZone, Utc};
use reeve_planning::DiscoveryMark; // reeve_protocol::DiscoveryMark (a request body) would otherwise be ambiguous
use reeve_planning::*;
use reeve_protocol::*;
use serde_json::{Value, json};
use std::collections::HashMap;

fn t(h: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 9, h, 0, 0).unwrap()
}
fn day(d: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, d, 12, 0, 0).unwrap()
}
fn k(s: &str) -> Key {
    s.parse().unwrap()
}
fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}
macro_rules! v {
    ($x:expr) => {
        serde_json::to_value($x).unwrap()
    };
}

#[test]
fn story_dto_carries_every_field() {
    let s = Story::from_persisted(
        k("ABC-3"),
        "T".into(),
        Kind::Bug,
        Some(k("ABC-2")),
        "D".into(),
        vec![Criterion {
            text: "c".into(),
            done: true,
        }],
        Some("o@x".into()),
        vec!["l".into()],
        4,
        DeliveryState::Done,
        Some(Resolution::Dropped),
        Some("SCRUM-9".into()),
        7,
    );
    assert_eq!(
        v!(StoryDto::from(&s)),
        json!({
            "key": "ABC-3", "title": "T", "kind": "bug", "epic": "ABC-2", "description": "D",
            "criteria": [{"text": "c", "done": true}], "owner": "o@x", "labels": ["l"], "rank": 4,
            "blocked_by": [], "delivery": "done", "resolution": "dropped", "external_ref": "SCRUM-9",
            "version": 7, "prs": [], "goals": [], "urgent_mark": null, "urgent_sources": [],
            "promise": null
        })
    );
}

#[test]
fn initiative_dto_carries_every_field() {
    let i = Initiative::from_persisted(
        k("ABC-1"),
        "Title".into(),
        "Brief".into(),
        "Now".into(),
        Some("2026-12".into()),
        Some("own@x".into()),
        Some(DiscoveryMark {
            at: t(8),
            by: "disc@x".into(),
        }),
        Some("SCRUM-1".into()),
        5,
    );
    assert_eq!(
        v!(InitiativeDto::from(&i)),
        json!({
            "key": "ABC-1", "title": "Title", "brief": "Brief", "horizon": "Now",
            "target_month": "2026-12", "owner": "own@x",
            "discovery_complete": {"at": "2026-11-09T08:00:00Z", "by": "disc@x"},
            "progress": null, "external_ref": "SCRUM-1", "version": 5, "urgent_mark": null,
            "urgent_sources": []
        })
    );
}

#[test]
fn epic_dto_carries_every_field() {
    let e = Epic::from_persisted(
        k("ABC-2"),
        "Title".into(),
        Some(k("ABC-1")),
        "Brief".into(),
        Some(DiscoveryMark {
            at: t(7),
            by: "disc@x".into(),
        }),
        Some("SCRUM-2".into()),
        6,
    );
    assert_eq!(
        v!(EpicDto::from(&e)),
        json!({
            "key": "ABC-2", "title": "Title", "initiative": "ABC-1", "brief": "Brief",
            "discovery_complete": {"at": "2026-11-09T07:00:00Z", "by": "disc@x"},
            "external_ref": "SCRUM-2", "version": 6, "urgent_mark": null, "urgent_sources": [],
            "promise": null
        })
    );
}

fn decision() -> RequiredDecision {
    RequiredDecision::from_persisted(
        11,
        k("ABC-3"),
        ItemType::Story,
        "Pick".into(),
        "dec@x".into(),
        Some(ymd(2026, 11, 20)),
        Some(Reversibility::TwoWay),
        Some(RecordRef::from_persisted(
            "kb",
            "adrs/0001.md",
            "kb/ADR-0001",
        )),
        Some("https://example.com/pr/1".into()),
        CheckResult::Accepted {
            commit: "abc123".into(),
            reversibility: Some(Reversibility::OneWay),
        },
        Some(t(6)),
        Some(Closed {
            at: t(7),
            by: "closer@x".into(),
            reason: "gone".into(),
            kind: Some(ClosureKind::Withdrawn),
            via: Some(TokenKind::Automation),
        }),
        t(5),
        9,
    )
}
fn decision_json() -> Value {
    json!({
        "id": 11, "on": "ABC-3", "on_type": "story", "title": "Pick", "decider": "dec@x",
        "due": "2026-11-20", "reversibility": "two_way",
        "record": {"repo": "kb", "path": "adrs/0001.md", "record_id": "kb/ADR-0001"},
        "pr_url": "https://example.com/pr/1",
        "check": {"kind": "accepted", "commit": "abc123", "reversibility": "one_way"},
        "checked_at": "2026-11-09T06:00:00Z",
        "closed": {"at": "2026-11-09T07:00:00Z", "by": "closer@x", "reason": "gone",
                   "kind": "withdrawn", "via": "automation"},
        "created_at": "2026-11-09T05:00:00Z", "version": 9
    })
}

#[test]
fn required_decision_dto_carries_every_field() {
    assert_eq!(v!(RequiredDecisionDto::from(&decision())), decision_json());
}

#[test]
fn pull_request_dto_carries_every_field() {
    let p = PullRequest {
        repo: "mindful-stack/reeve".into(),
        number: 61,
        url: "https://github.com/mindful-stack/reeve/pull/61".into(),
        title: "A title".into(),
        status: PrStatus::Merged,
        updated_at: t(4),
    };
    assert_eq!(
        v!(PullRequestDto::from(&p)),
        json!({
            "repo": "mindful-stack/reeve", "number": 61,
            "url": "https://github.com/mindful-stack/reeve/pull/61", "title": "A title",
            "status": "merged", "updated_at": "2026-11-09T04:00:00Z"
        })
    );
}

fn goal(id: i64, sprint_id: i64) -> Goal {
    Goal {
        id,
        sprint_id,
        outcome: format!("outcome {id}"),
        check: format!("check {id}"),
        owner: "own@x".into(),
        notes: "notes".into(),
        confidence: Some(Confidence::AtRisk),
        confidence_note: Some("c-note".into()),
        confidence_at: Some(t(1)),
        suggested_confidence: Some(Confidence::OffTrack),
        suggested_note: Some("s-note".into()),
        suggested_at: Some(t(2)),
        suggested_by: Some("agent@x".into()),
        verdict: Some(Verdict::PartlyMet),
        verdict_note: Some("v-note".into()),
        version: 8,
    }
}
fn goal_json(id: i64, sprint_id: i64, links: Value) -> Value {
    json!({
        "id": id, "sprint_id": sprint_id, "outcome": format!("outcome {id}"), "check": format!("check {id}"),
        "owner": "own@x", "notes": "notes",
        "confidence": "at_risk", "confidence_note": "c-note", "confidence_at": "2026-11-09T01:00:00Z",
        "suggested_confidence": "off_track", "suggested_note": "s-note", "suggested_at": "2026-11-09T02:00:00Z",
        "suggested_by": "agent@x",
        "verdict": "partly_met", "verdict_note": "v-note", "version": 8, "links": links
    })
}
fn link(goal_id: i64, key: &str) -> GoalLink {
    GoalLink {
        goal_id,
        item_key: k(key),
    }
}

#[test]
fn goal_dto_carries_every_field_and_only_its_own_links() {
    let links = [link(2, "ABC-8"), link(1, "ABC-5"), link(1, "ABC-4")];
    assert_eq!(
        v!(goal_dto(&goal(1, 7), &links)),
        goal_json(1, 7, json!(["ABC-5", "ABC-4"]))
    );
}

fn sprint(id: i64, state: SprintState) -> Sprint {
    Sprint {
        id,
        state,
        starts_on: ymd(2026, 11, 2),
        ends_on: ymd(2026, 11, 16),
        opened_at: Some(t(3)),
        closed_at: if state == SprintState::Closed {
            Some(t(10))
        } else {
            None
        },
        version: 3,
    }
}

#[test]
fn sprint_dto_of_a_draft_warns_and_takes_only_its_goals() {
    // Four goals is more than three: the warning. Goal 9 belongs to another sprint and is left out.
    let goals = [goal(1, 1), goal(2, 1), goal(9, 2), goal(3, 1), goal(4, 1)];
    let links = [link(2, "ABC-6"), link(9, "ABC-7"), link(1, "ABC-5")];
    let mut s = sprint(1, SprintState::Draft);
    s.opened_at = None;
    assert_eq!(
        v!(sprint_dto(&s, &goals, &links)),
        json!({
            "id": 1, "state": "draft", "starts_on": "2026-11-02", "ends_on": "2026-11-16",
            "opened_at": null, "closed_at": null, "version": 3,
            "warnings": ["more_than_three_goals"],
            "goals": [
                goal_json(1, 1, json!(["ABC-5"])),
                goal_json(2, 1, json!(["ABC-6"])),
                goal_json(3, 1, json!([])),
                goal_json(4, 1, json!([])),
            ]
        })
    );
    // No goals at all: the other warning.
    assert_eq!(
        v!(sprint_dto(&s, &[goal(9, 2)], &links))["warnings"],
        json!(["no_goals"])
    );
}

#[test]
fn sprint_dto_of_an_active_sprint_without_goals_warns() {
    let got = v!(sprint_dto(
        &sprint(5, SprintState::Active),
        &[goal(9, 2)],
        &[]
    ));
    assert_eq!(got["warnings"], json!(["no_goals"]));
    assert_eq!(got["goals"], json!([]));
}

#[test]
fn sprint_dto_of_a_closed_sprint_has_no_warnings() {
    let goals = [goal(1, 5), goal(2, 5), goal(3, 5), goal(4, 5)];
    let got = v!(sprint_dto(&sprint(5, SprintState::Closed), &goals, &[]));
    assert_eq!(
        got["warnings"],
        json!([]),
        "four goals would warn on a draft"
    );
    assert_eq!(got["state"], json!("closed"));
    assert_eq!(got["opened_at"], json!("2026-11-09T03:00:00Z"));
    assert_eq!(got["closed_at"], json!("2026-11-09T10:00:00Z"));
    assert_eq!(got["goals"].as_array().unwrap().len(), 4);
}

fn st(
    key: &str,
    delivery: DeliveryState,
    resolution: Option<Resolution>,
    started: Option<u32>,
    done: Option<u32>,
) -> StoryTimes {
    StoryTimes {
        key: k(key),
        title: format!("title {key}"),
        epic: None,
        initiative: None,
        delivery,
        resolution,
        started_at: started.map(day),
        done_at: done.map(day),
    }
}

/// Goal 42 over the window [2 Nov, 16 Nov). Every count is distinct and non-zero, derived from `summarise`:
/// completed inside the window and linked: C1 C2 C3 (goal completed = 3); completed inside and unlinked: U1
/// (upkeep = 1, total completed = 4); in flight (started before `to`, unfinished): F1..F5, of which F2 and F3 are in
/// review or testing (in_flight = 5, in_review = 2); never started: T1..T6 (todo = 6); decisions 10..16 on unfinished
/// stories (blocked = 7). A completed story is blocked too, but a done story's decisions do not count.
fn summary() -> Summary {
    use DeliveryState::*;
    let done = |key, d| st(key, Done, Some(Resolution::Completed), Some(3), Some(d));
    let stories = vec![
        done("ABC-101", 5), // C1: inside the window, before the demo window
        done("ABC-102", 12),
        done("ABC-103", 10),
        done("ABC-104", 11), // U1: unlinked
        st("ABC-111", InProgress, None, Some(3), None),
        st("ABC-112", InReview, None, Some(3), None),
        st("ABC-113", Testing, None, Some(4), None),
        st("ABC-114", InProgress, None, Some(5), None),
        st("ABC-115", InProgress, None, Some(6), None),
        st("ABC-121", Todo, None, None, None),
        st("ABC-122", Todo, None, None, None),
        st("ABC-123", Todo, None, None, None),
        st("ABC-124", Todo, None, None, None),
        st("ABC-125", Todo, None, None, None),
        st("ABC-126", Todo, None, None, None),
    ];
    let mut links = vec![];
    for n in [
        101, 102, 103, 111, 112, 113, 114, 115, 121, 122, 123, 124, 125, 126,
    ] {
        links.push(link(42, &format!("ABC-{n}")));
    }
    let blocked: HashMap<Key, Vec<i64>> = HashMap::from([
        (k("ABC-111"), vec![10, 11]),
        (k("ABC-112"), vec![12]),
        (k("ABC-121"), vec![13, 10]),
        (k("ABC-122"), vec![14]),
        (k("ABC-123"), vec![15, 16]),
        (k("ABC-101"), vec![99]),
    ]);
    summarise(&[42], &links, &stories, &blocked, day(2), day(16))
}

#[test]
fn sprint_view_dto_of_an_active_sprint_carries_counts_and_demo() {
    let got = v!(sprint_view_dto(
        &sprint(5, SprintState::Active),
        &[goal(42, 5)],
        &[link(42, "ABC-9")],
        Some(&summary()),
        &[],
        20
    ));
    assert_eq!(
        got,
        json!({
            "sprint": {
                "id": 5, "state": "active", "starts_on": "2026-11-02", "ends_on": "2026-11-16",
                "opened_at": "2026-11-09T03:00:00Z", "closed_at": null, "version": 3, "warnings": [],
                "goals": [goal_json(42, 5, json!(["ABC-9"]))]
            },
            "counts": {
                "goals": [{"goal_id": 42, "completed": 3, "in_flight": 5, "todo": 6, "in_review": 2,
                           "blocked_decisions": 7}],
                "completed": 4, "upkeep_completed": 1
            },
            "upkeep_line": 20,
            "demo": [
                {"key": "ABC-103", "title": "title ABC-103"},
                {"key": "ABC-104", "title": "title ABC-104"},
                {"key": "ABC-102", "title": "title ABC-102"},
            ],
            "promises": []
        })
    );
}

#[test]
fn sprint_view_dto_of_a_draft_has_zero_counts_and_no_demo() {
    let got = v!(sprint_view_dto(
        &sprint(5, SprintState::Draft),
        &[],
        &[],
        None,
        &[],
        17
    ));
    assert_eq!(
        got["counts"],
        json!({"goals": [], "completed": 0, "upkeep_completed": 0})
    );
    assert_eq!(got["demo"], json!([]));
    assert_eq!(got["upkeep_line"], json!(17));
    assert_eq!(got["promises"], json!([]));
    assert_eq!(got["sprint"]["warnings"], json!(["no_goals"]));
}

#[test]
fn queue_entry_dto_carries_every_field() {
    let e = QueueEntry {
        group: QueueGroup::Blocking,
        lane: QueueLane::ThisSprint,
        needed_in: NeededIn::ThisSprint,
        needed_in_reason: Some(NeededInReason::Goal {
            goal_id: 7,
            outcome: "ship it".into(),
        }),
        next_step: Some(NextStep::ReviewRecord {
            pr_url: "https://github.com/acme/kb/pull/9".into(),
        }),
        decision: decision(),
        held: vec![
            QueueStory {
                kind: Kind::Bug,
                delivery: DeliveryState::Todo,
                owner: Some("o@x".into()),
                urgent: true,
                needed_in: NeededIn::ThisSprint,
                key: k("ABC-31"),
                title: "first".into(),
            },
            QueueStory {
                kind: Kind::Story,
                delivery: DeliveryState::Todo,
                owner: None,
                urgent: false,
                needed_in: NeededIn::Beyond,
                key: k("ABC-32"),
                title: "second".into(),
            },
        ],
        under_way: vec![QueueStory {
            kind: Kind::Chore,
            delivery: DeliveryState::InReview,
            owner: Some("p@x".into()),
            urgent: false,
            needed_in: NeededIn::NextSprint,
            key: k("ABC-33"),
            title: "third".into(),
        }],
    };
    assert_eq!(
        v!(DecisionQueueEntryDto::from(&e)),
        json!({
            "decision": decision_json(), "group": "blocking",
            "lane": "this_sprint", "needed_in": "this_sprint",
            "needed_in_reason": {"kind": "goal", "goal_id": 7, "outcome": "ship it"},
            "next_step": {"kind": "review_record", "pr_url": "https://github.com/acme/kb/pull/9"},
            "blocks": [
                {"key": "ABC-31", "title": "first", "kind": "bug", "delivery": "todo", "owner": "o@x", "urgent": true, "needed_in": "this_sprint"},
                {"key": "ABC-32", "title": "second", "kind": "story", "delivery": "todo", "owner": null, "urgent": false, "needed_in": "beyond"}
            ],
            "under_way": [
                {"key": "ABC-33", "title": "third", "kind": "chore", "delivery": "in_review", "owner": "p@x", "urgent": false, "needed_in": "next_sprint"}
            ]
        })
    );
}

#[test]
fn queue_summary_dto_carries_every_count() {
    let s = QueueSummary {
        open: 4,
        blocking: 3,
        blocked_stories: 2,
        overdue: 1,
        review: 5,
        this_sprint: 6,
        next_sprint: 7,
    };
    assert_eq!(
        v!(DecisionQueueSummaryDto::from(s)),
        json!({"open": 4, "blocking": 3, "blocked_stories": 2, "overdue": 1, "review": 5, "this_sprint": 6, "next_sprint": 7})
    );
}

/// An old server's queue (no field added by the lanes change) still reads: a new client against an older server.
#[test]
fn an_older_servers_queue_json_reads_with_defaults() {
    let old = json!({
        "entries": [{
            "decision": decision_json(), "group": "blocking",
            "blocks": [{"key": "ABC-31", "title": "first"}]
        }],
        "summary": {"open": 4, "blocking": 3, "blocked_stories": 2, "overdue": 1}
    });
    let q: DecisionQueueDto = serde_json::from_value(old).unwrap();
    let e = &q.entries[0];
    assert_eq!(e.lane, QueueLane::Open);
    assert_eq!(e.needed_in, NeededIn::None);
    assert_eq!(e.needed_in_reason, None);
    assert_eq!(e.next_step, None);
    assert!(e.under_way.is_empty());
    let b = &e.blocks[0];
    assert_eq!(
        (
            b.kind,
            b.delivery,
            b.owner.as_deref(),
            b.urgent,
            b.needed_in
        ),
        (
            Kind::Story,
            DeliveryState::Todo,
            None,
            false,
            NeededIn::None
        )
    );
    assert_eq!(e.decision.record.as_ref().unwrap().browse_url, None);
    let s = &q.summary;
    assert_eq!((s.review, s.this_sprint, s.next_sprint), (0, 0, 0));
}

/// A newer server's queue with a field this build does not know still reads: an older client against a newer server.
#[test]
fn a_queue_with_an_unknown_extra_field_reads() {
    let mut e = json!({
        "decision": decision_json(), "group": "open", "lane": "open", "needed_in": "none",
        "needed_in_reason": null, "next_step": null, "blocks": [], "under_way": [],
        "added_later": {"x": 1}
    });
    e["decision"]["record"]["browse_url"] =
        json!("https://github.com/acme/kb/blob/HEAD/adrs/0001.md");
    e["decision"]["added_later"] = json!(true);
    let q: DecisionQueueDto = serde_json::from_value(json!({
        "entries": [e],
        "summary": {"open": 1, "blocking": 0, "blocked_stories": 0, "overdue": 0,
                    "review": 0, "this_sprint": 0, "next_sprint": 0, "added_later": 3}
    }))
    .unwrap();
    assert_eq!(q.entries.len(), 1);
    assert_eq!(
        q.entries[0]
            .decision
            .record
            .as_ref()
            .unwrap()
            .browse_url
            .as_deref(),
        Some("https://github.com/acme/kb/blob/HEAD/adrs/0001.md")
    );
}

fn mark_state(item: ItemType) -> MarkState {
    MarkState {
        item,
        mark: UrgentMark {
            at: t(3),
            by: "marker@x".into(),
            reason: Some("outage".into()),
        },
        expires_at: day(23),
        active: true,
        done: false,
    }
}

fn mark_json() -> Value {
    json!({
        "at": "2026-11-09T03:00:00Z", "by": "marker@x", "reason": "outage",
        "expires_at": "2026-11-23T12:00:00Z", "active": true, "done": false
    })
}

#[test]
fn urgent_mark_dto_carries_every_field() {
    assert_eq!(
        v!(UrgentMarkDto::from(&mark_state(ItemType::Story))),
        mark_json()
    );
}

#[test]
fn war_room_entry_dto_carries_every_field() {
    let with = WarRoomEntry {
        item: ItemType::Epic,
        key: k("ABC-5"),
        title: "Epic title".into(),
        initiative: Some(WarRoomInitiative {
            key: k("ABC-4"),
            title: "Initiative title".into(),
            horizon: "now".into(),
        }),
        // A different item type on the mark, so a builder that read `state.item` for the entry's item shows.
        state: mark_state(ItemType::Story),
    };
    assert_eq!(
        v!(WarRoomEntryDto::from(&with)),
        json!({
            "item": {"key": "ABC-5", "item": "epic"}, "title": "Epic title",
            "initiative": {"key": "ABC-4", "title": "Initiative title", "horizon": "now"},
            "mark": mark_json()
        })
    );
    let without = WarRoomEntry {
        initiative: None,
        ..with
    };
    assert_eq!(
        v!(WarRoomEntryDto::from(&without)),
        json!({
            "item": {"key": "ABC-5", "item": "epic"}, "title": "Epic title", "initiative": null,
            "mark": mark_json()
        })
    );
}

fn kept_promise(on: NaiveDate, by: &str, at: chrono::DateTime<Utc>, to: Option<&str>) -> Promise {
    Promise::from_persisted(on, by.into(), at, to.map(str::to_string))
}

fn promise_clock(today: NaiveDate) -> PromiseClock {
    PromiseClock {
        today,
        active_ends: Some(ymd(2026, 11, 24)),
        draft_ends: Some(ymd(2026, 12, 8)),
    }
}

/// Both promises, the effective date and its holder, the state, the due period and the days: every field distinct, so
/// a renamed or dropped field shows. The epic's earlier date holds; on 21 Nov it is three days overdue.
#[test]
fn story_promise_dto_carries_every_field() {
    let own = kept_promise(ymd(2026, 11, 20), "pm@x", t(5), Some("Acme"));
    let epic = kept_promise(ymd(2026, 11, 18), "lead@x", t(4), None);
    let epic_key = k("ABC-7");
    let facts = StoryFacts {
        delivery: DeliveryState::Todo,
        resolution: None,
        done_at: None,
        own: Some(&own),
        epic: Some((&epic_key, &epic)),
    };
    let sp = story_promise(&facts, &promise_clock(ymd(2026, 11, 21)));
    assert_eq!(
        v!(story_promise_field(sp.as_ref())),
        json!({
            "own": {"on": "2026-11-20", "by": "pm@x", "at": "2026-11-09T05:00:00Z", "to": "Acme"},
            "epic": {"key": "ABC-7", "fact": {
                "on": "2026-11-18", "by": "lead@x", "at": "2026-11-09T04:00:00Z", "to": null
            }},
            "on": "2026-11-18", "holder": "epic", "state": "overdue", "due": null, "days": 3
        })
    );
    assert_eq!(story_promise_field(None), None);
}

/// An epic's own promise has no state: its due period while not past, `past` once its date is behind today.
#[test]
fn epic_promise_dto_carries_every_field() {
    let p = kept_promise(ymd(2026, 12, 1), "lead@x", t(6), Some("Beta"));
    let fact =
        json!({"on": "2026-12-01", "by": "lead@x", "at": "2026-11-09T06:00:00Z", "to": "Beta"});
    assert_eq!(
        v!(epic_promise_field(
            Some(&p),
            &promise_clock(ymd(2026, 11, 21))
        )),
        json!({"fact": fact, "due": "next_sprint", "past": false})
    );
    assert_eq!(
        v!(epic_promise_field(
            Some(&p),
            &promise_clock(ymd(2026, 12, 2))
        )),
        json!({"fact": fact, "due": null, "past": true})
    );
    assert_eq!(
        epic_promise_field(None, &promise_clock(ymd(2026, 11, 21))),
        None
    );
}

/// A sprint's promise row: the epic's earlier date holds, so `to` is the epic's recipient (not the story's own), the
/// epic's key is named, and the story, done two days after it, is late with its done day. Beside a later epic promise,
/// the story's own date holds: its own recipient, and no epic named although the story inherits one. Every field
/// distinct.
#[test]
fn sprint_promise_dto_carries_every_field() {
    let own = kept_promise(ymd(2026, 11, 22), "pm@x", t(5), Some("Acme"));
    let epic = kept_promise(ymd(2026, 11, 18), "lead@x", t(4), Some("Beta"));
    let later_epic = kept_promise(ymd(2026, 11, 25), "lead@x", t(4), Some("Beta"));
    let epic_key = k("ABC-7");
    let facts = StoryFacts {
        delivery: DeliveryState::Done,
        resolution: Some(Resolution::Completed),
        done_at: Some(Utc.with_ymd_and_hms(2026, 11, 20, 23, 30, 0).unwrap()),
        own: Some(&own),
        epic: Some((&epic_key, &epic)),
    };
    let row = |f: &StoryFacts<'_>| {
        let sp = story_promise(f, &promise_clock(ymd(2026, 11, 25))).unwrap();
        SprintPromise::listed(
            k("ABC-9"),
            "Ship it".into(),
            sp,
            Some(ymd(2026, 11, 10)),
            ymd(2026, 11, 24),
        )
        .unwrap()
    };
    assert_eq!(
        v!(sprint_promise_dto(&row(&facts))),
        json!({
            "key": "ABC-9", "title": "Ship it", "on": "2026-11-18", "to": "Beta", "holder": "epic",
            "epic": "ABC-7", "state": "late", "done_on": "2026-11-20", "days": 2
        })
    );
    let own_holds = StoryFacts {
        epic: Some((&epic_key, &later_epic)),
        ..facts
    };
    assert_eq!(
        v!(sprint_promise_dto(&row(&own_holds))),
        json!({
            "key": "ABC-9", "title": "Ship it", "on": "2026-11-22", "to": "Acme", "holder": "own",
            "epic": null, "state": "kept", "done_on": "2026-11-20", "days": null
        })
    );
}

/// An older server's sprint view (no `promises`) still reads, as none.
#[test]
fn old_sprint_view_json_without_promises_parses() {
    let view = json!({
        "sprint": {
            "id": 5, "state": "active", "starts_on": "2026-11-02", "ends_on": "2026-11-16",
            "opened_at": "2026-11-09T03:00:00Z", "closed_at": null, "version": 3, "warnings": [], "goals": []
        },
        "counts": {"goals": [], "completed": 0, "upkeep_completed": 0},
        "upkeep_line": 20,
        "demo": []
    });
    let v: SprintViewDto = serde_json::from_value(view).unwrap();
    assert_eq!(v.promises, vec![]);
}

/// An older server's story and epic (no `promise`) still read, as no promise.
#[test]
fn old_story_and_epic_json_without_promise_parse() {
    let story = json!({
        "key": "ABC-3", "title": "T", "kind": "bug", "epic": null, "description": "D", "criteria": [],
        "owner": null, "labels": [], "rank": 4, "blocked_by": [], "delivery": "todo", "resolution": null,
        "external_ref": null, "version": 7, "prs": [], "goals": [], "urgent_mark": null, "urgent_sources": []
    });
    let s: StoryDto = serde_json::from_value(story).unwrap();
    assert_eq!(s.promise, None);
    let epic = json!({
        "key": "ABC-2", "title": "Title", "initiative": null, "brief": "Brief", "discovery_complete": null,
        "external_ref": null, "version": 6, "urgent_mark": null, "urgent_sources": []
    });
    let e: EpicDto = serde_json::from_value(epic).unwrap();
    assert_eq!(e.promise, None);
}

/// `today` is on the wire under that name; an older server's sprints (without it) still read.
#[test]
fn sprints_dto_carries_today() {
    let dto = SprintsDto {
        active: None,
        draft: None,
        closed: vec![],
        today: Some(ymd(2026, 11, 12)),
    };
    assert_eq!(
        v!(&dto),
        json!({"active": null, "draft": null, "closed": [], "today": "2026-11-12"})
    );
    let old: SprintsDto =
        serde_json::from_value(json!({"active": null, "draft": null, "closed": []})).unwrap();
    assert_eq!(old.today, None);
}

/// An open promise the story holds itself: no epic, its own date due this sprint, no days.
#[test]
fn story_promise_dto_of_an_open_own_promise() {
    let own = kept_promise(ymd(2026, 11, 20), "pm@x", t(5), None);
    let facts = StoryFacts {
        delivery: DeliveryState::InProgress,
        resolution: None,
        done_at: None,
        own: Some(&own),
        epic: None,
    };
    let sp = story_promise(&facts, &promise_clock(ymd(2026, 11, 12)));
    assert_eq!(
        v!(story_promise_field(sp.as_ref())),
        json!({
            "own": {"on": "2026-11-20", "by": "pm@x", "at": "2026-11-09T05:00:00Z", "to": null},
            "epic": null, "on": "2026-11-20", "holder": "own", "state": "open", "due": "this_sprint",
            "days": null
        })
    );
}

/// `SetPromise` takes a date, a shortcut, a recipient, any of them or none (the domain, not the parser, refuses an empty
/// request); an unknown field is refused.
#[test]
fn set_promise_body_parses() {
    let parse = |v: Value| serde_json::from_value::<SetPromise>(v).unwrap();
    let body = |on, shortcut, to: Option<&str>| SetPromise {
        on,
        shortcut,
        to: to.map(str::to_string),
    };
    assert_eq!(
        parse(json!({"on": "2026-11-20"})),
        body(Some(ymd(2026, 11, 20)), None, None)
    );
    assert_eq!(
        parse(json!({"shortcut": "this_sprint"})),
        body(None, Some(PromiseShortcut::ThisSprint), None)
    );
    assert_eq!(
        parse(json!({"shortcut": "next_sprint"})),
        body(None, Some(PromiseShortcut::NextSprint), None)
    );
    assert_eq!(
        parse(json!({"on": "2026-11-20", "to": "Acme"})),
        body(Some(ymd(2026, 11, 20)), None, Some("Acme"))
    );
    assert_eq!(parse(json!({"to": ""})), body(None, None, Some("")));
    assert_eq!(parse(json!({})), body(None, None, None));
    assert_eq!(
        parse(json!({"to": null})),
        body(None, None, None),
        "null keeps, as absent does"
    );
    assert!(
        serde_json::from_value::<SetPromise>(json!({"date": "2026-11-20"})).is_err(),
        "an unknown field is refused"
    );
}

/// The promise reason carries the holder's key and level, the effective date and `overdue`, and round-trips; JSON
/// without `overdue` reads as false.
#[test]
fn needed_in_reason_promise_carries_every_field() {
    let r = NeededInReason::Promise {
        key: k("ABC-5"),
        level: ItemType::Epic,
        on: ymd(2026, 11, 10),
        overdue: true,
    };
    let want = json!({"kind": "promise", "key": "ABC-5", "level": "epic", "on": "2026-11-10", "overdue": true});
    assert_eq!(v!(&r), want);
    assert_eq!(serde_json::from_value::<NeededInReason>(want).unwrap(), r);
    assert_eq!(
        serde_json::from_value::<NeededInReason>(
            json!({"kind": "promise", "key": "ABC-5", "level": "story", "on": "2026-11-10"})
        )
        .unwrap(),
        NeededInReason::Promise {
            key: k("ABC-5"),
            level: ItemType::Story,
            on: ymd(2026, 11, 10),
            overdue: false,
        }
    );
}

/// A reason kind this build does not know decodes to `Unknown` rather than failing the whole queue.
#[test]
fn an_unknown_needed_in_reason_decodes_to_unknown() {
    assert_eq!(
        serde_json::from_value::<NeededInReason>(json!({"kind": "future_thing", "x": 1})).unwrap(),
        NeededInReason::Unknown
    );
}
