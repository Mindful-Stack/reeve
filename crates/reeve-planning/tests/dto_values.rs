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
            "version": 7, "prs": [], "goals": [], "urgent_mark": null, "urgent_sources": []
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
            "external_ref": "SCRUM-2", "version": 6, "urgent_mark": null, "urgent_sources": []
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
            ]
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
        17
    ));
    assert_eq!(
        got["counts"],
        json!({"goals": [], "completed": 0, "upkeep_completed": 0})
    );
    assert_eq!(got["demo"], json!([]));
    assert_eq!(got["upkeep_line"], json!(17));
    assert_eq!(got["sprint"]["warnings"], json!(["no_goals"]));
}

#[test]
fn queue_entry_dto_carries_decision_group_and_blocked_stories() {
    let e = QueueEntry {
        group: QueueGroup::Blocking,
        decision: decision(),
        blocks: vec![
            BlockedStory {
                key: k("ABC-31"),
                title: "first".into(),
            },
            BlockedStory {
                key: k("ABC-32"),
                title: "second".into(),
            },
        ],
    };
    assert_eq!(
        v!(DecisionQueueEntryDto::from(&e)),
        json!({
            "decision": decision_json(), "group": "blocking",
            "blocks": [{"key": "ABC-31", "title": "first"}, {"key": "ABC-32", "title": "second"}]
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
    };
    assert_eq!(
        v!(DecisionQueueSummaryDto::from(s)),
        json!({"open": 4, "blocking": 3, "blocked_stories": 2, "overdue": 1})
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
        state: mark_state(ItemType::Epic),
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
