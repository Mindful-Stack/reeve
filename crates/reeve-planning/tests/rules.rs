//! Characterisation tests for the rules passes 1 to 3 move (spec 2026-11-09, pass 0). Their assertions must not change.
mod common;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use reeve_planning::*;
use reeve_protocol::{CheckResult, ClosureKind, DeliveryState, Kind, Reason, Resolution, Via};
use serde_json::json;
use std::sync::{Arc, Mutex};

struct MemSink(Mutex<Vec<ActivityRecord>>);
#[async_trait]
impl ActivitySink for MemSink {
    async fn record(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        rec: ActivityRecord,
    ) -> Result<(), RepoError> {
        self.0.lock().unwrap().push(rec);
        Ok(())
    }
}
impl MemSink {
    /// (action, payload) of every entry on `key`, in write order.
    fn on(&self, key: &str) -> Vec<(&'static str, serde_json::Value)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.entity_key == key)
            .map(|r| (r.action, r.payload.clone()))
            .collect()
    }
}

fn mk(pool: sqlx::PgPool) -> (Handlers, Arc<MemSink>) {
    let sink = Arc::new(MemSink(Mutex::new(vec![])));
    let h = Handlers {
        ports: make_planning_ports(pool.clone()),
        activity: sink.clone(),
        pool,
        settings: Settings {
            key_prefix: "ABC".into(),
            horizon_buckets: vec!["Now".into()],
            repos: vec!["kb".into()],
            done_window_days: 14,
            sprint_length_days: 14,
            upkeep_line: 20,
        },
    };
    (h, sink)
}
fn ctx(actor: &str, via: Via) -> Ctx {
    Ctx {
        actor: actor.into(),
        via,
        now: Utc.with_ymd_and_hms(2026, 11, 9, 9, 0, 0).unwrap(),
    }
}
fn dan() -> Ctx {
    ctx("dan@example.com", Via::Interactive)
}

/// A chore with no epic. `criteria` empty makes it not ready.
async fn chore(h: &Handlers, idem: &str, criteria: &[&str], owner: Option<&str>) -> Story {
    h.create_story(
        &dan(),
        idem,
        idem,
        Kind::Chore,
        None,
        "",
        criteria.iter().map(|c| c.to_string()).collect(),
        vec![],
        None,
        owner.map(Into::into),
    )
    .await
    .unwrap()
}
fn refused(e: HandlerError) -> Vec<Reason> {
    match e {
        HandlerError::Refused(r) => r,
        other => panic!("wanted Refused, got {other:?}"),
    }
}

/// Each of the five refusals answers exactly its reasons, and a same-state move answers the story; none changes the
/// story or writes an entry. Catches a `move_to` that maps a refusal to the wrong reason, drops or reorders the
/// readiness reasons, or mutates the story before refusing.
#[tokio::test]
async fn move_refusals_answer_exact_reasons_and_change_nothing() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let d = dan();
    let bot = ctx("dan@example.com", Via::Automation);
    // Kind::Story without an epic is untriaged, and with no criteria it also has NoCriteria: two reasons whose order
    // is pinned literally (domain/readiness.rs emits Untriaged before NoCriteria). Do not change the kind.
    let unready = h
        .create_story(
            &d,
            "u",
            "u",
            Kind::Story,
            None,
            "",
            vec![],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let ready = chore(&h, "r", &["ok"], None).await;
    let reasons = vec![Reason::Untriaged, Reason::NoCriteria];
    assert_eq!(
        h.readiness(unready.key()).await.unwrap().reasons,
        reasons,
        "fixture: two independent reasons"
    );

    type Case<'a> = (
        &'a Ctx,
        &'a Story,
        DeliveryState,
        Option<Resolution>,
        Option<String>,
        Vec<Reason>,
    );
    let cases: Vec<Case> = vec![
        (
            &d,
            &unready,
            DeliveryState::InProgress,
            None,
            None,
            reasons.clone(),
        ),
        (
            &bot,
            &unready,
            DeliveryState::InProgress,
            None,
            Some("ship".into()),
            vec![
                Reason::Untriaged,
                Reason::NoCriteria,
                Reason::AutomationCannotOverride,
            ],
        ),
        (
            &d,
            &unready,
            DeliveryState::InProgress,
            None,
            Some("   ".into()),
            vec![Reason::OverrideNeedsReason],
        ),
        (
            &d,
            &unready,
            DeliveryState::Done,
            None,
            None,
            vec![Reason::ResolutionRequired],
        ),
        (
            &d,
            &unready,
            DeliveryState::Done,
            Some(Resolution::Completed),
            None,
            vec![Reason::CompletedNeedsActiveOrigin],
        ),
    ];
    for (c, s, to, res, why, want) in cases {
        let entries_before = sink.on(&s.key().to_string());
        let got = refused(h.move_story(c, s.key(), 1, to, res, why).await.unwrap_err());
        assert_eq!(got, want, "{to:?} {res:?}");
        assert_eq!(
            &h.get_story(s.key()).await.unwrap(),
            s,
            "refused: the story is unchanged"
        );
        assert_eq!(
            sink.on(&s.key().to_string()),
            entries_before,
            "refused: no entry"
        );
    }
    // Same state: Ok with the story as it was, no entry.
    let entries_before = sink.on(&ready.key().to_string());
    let same = h
        .move_story(&d, ready.key(), 1, DeliveryState::Todo, None, None)
        .await
        .unwrap();
    assert_eq!(&same, &ready);
    assert_eq!(&h.get_story(ready.key()).await.unwrap(), &ready);
    assert_eq!(sink.on(&ready.key().to_string()), entries_before);

    // The one ungated exit: an unready todo story may be dropped (validate_move's done branch precedes the gate).
    let dropped = h
        .move_story(
            &d,
            unready.key(),
            1,
            DeliveryState::Done,
            Some(Resolution::Dropped),
            None,
        )
        .await
        .unwrap();
    assert_eq!(dropped.delivery(), DeliveryState::Done);
    assert_eq!(dropped.resolution(), Some(Resolution::Dropped));
    assert_eq!(
        sink.on(&unready.key().to_string())
            .into_iter()
            .filter(|(a, _)| *a != "created")
            .collect::<Vec<_>>(),
        vec![(
            "moved",
            json!({"from": "todo", "to": "done", "resolution": "dropped"})
        )]
    );
}

/// ADR-0002 R4: entering an active state from done is gated like from todo. Catches a gate that only looks at todo.
#[tokio::test]
async fn leaving_done_for_an_active_state_is_gated_by_readiness() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let s = chore(&h, "s", &[], None).await;
    let key = s.key().to_string();
    h.move_story(
        &dan(),
        s.key(),
        1,
        DeliveryState::InProgress,
        None,
        Some("go".into()),
    )
    .await
    .unwrap();
    h.move_story(
        &dan(),
        s.key(),
        2,
        DeliveryState::Done,
        Some(Resolution::Dropped),
        None,
    )
    .await
    .unwrap();
    let before = h.get_story(s.key()).await.unwrap();
    assert_eq!(before.delivery(), DeliveryState::Done);
    let entries_before = sink.on(&key);
    let got = refused(
        h.move_story(&dan(), s.key(), 3, DeliveryState::InProgress, None, None)
            .await
            .unwrap_err(),
    );
    assert_eq!(got, vec![Reason::NoCriteria]);
    assert_eq!(h.get_story(s.key()).await.unwrap(), before);
    assert_eq!(sink.on(&key), entries_before);
}

/// `override` then `moved`, with `from` the state before the move. Catches pass 3 taking `from` from the story after
/// `move_to` mutated it (it would read in_progress), and a lost or reordered entry.
#[tokio::test]
async fn move_entries_carry_the_state_before_the_move() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let s = chore(&h, "s", &[], None).await;
    let key = s.key().to_string();
    let reasons = h.readiness(s.key()).await.unwrap().reasons;
    h.move_story(
        &dan(),
        s.key(),
        1,
        DeliveryState::InProgress,
        None,
        Some(" ship it ".into()),
    )
    .await
    .unwrap();
    h.move_story(
        &dan(),
        s.key(),
        2,
        DeliveryState::Done,
        Some(Resolution::Completed),
        None,
    )
    .await
    .unwrap();
    h.move_story(
        &dan(),
        s.key(),
        3,
        DeliveryState::Done,
        Some(Resolution::Dropped),
        None,
    )
    .await
    .unwrap();
    for r in sink
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.action != "created")
    {
        assert_eq!(r.entity_type, "story", "{}", r.action);
        assert_eq!(r.actor, "dan@example.com", "{}", r.action);
    }
    let moves: Vec<_> = sink
        .on(&key)
        .into_iter()
        .filter(|(a, _)| *a != "created")
        .collect();
    assert_eq!(
        moves,
        vec![
            (
                "override",
                json!({"from": "todo", "to": "in_progress", "reason": "ship it", "reasons": reasons})
            ),
            (
                "moved",
                json!({"from": "todo", "to": "in_progress", "resolution": null})
            ),
            (
                "moved",
                json!({"from": "in_progress", "to": "done", "resolution": "completed"})
            ),
            (
                "moved",
                json!({"from": "done", "to": "done", "resolution": "dropped"})
            ),
        ]
    );
}

/// Owner on start: set to the normalised actor when unowned, kept when owned, untouched outside in_progress. Catches
/// the rule moving onto `Story` and losing a case.
#[tokio::test]
async fn starting_a_story_assigns_an_owner_only_when_it_has_none() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let mixed = ctx("  Dan@Example.COM ", Via::Interactive);
    let unowned = chore(&h, "a", &["ok"], None).await;
    let owned = chore(&h, "b", &["ok"], Some("mia@example.com")).await;
    let testing = chore(&h, "c", &["ok"], None).await;
    let a = h
        .move_story(
            &mixed,
            unowned.key(),
            1,
            DeliveryState::InProgress,
            None,
            None,
        )
        .await
        .unwrap();
    let b = h
        .move_story(
            &mixed,
            owned.key(),
            1,
            DeliveryState::InProgress,
            None,
            None,
        )
        .await
        .unwrap();
    let c = h
        .move_story(&mixed, testing.key(), 1, DeliveryState::Testing, None, None)
        .await
        .unwrap();
    assert_eq!(a.owner(), Some("dan@example.com"));
    assert_eq!(b.owner(), Some("mia@example.com"));
    assert_eq!(c.owner(), None);
    // Entering in_progress assigns the actor whenever the story has no owner, from any earlier state.
    let c2 = h
        .move_story(
            &mixed,
            testing.key(),
            2,
            DeliveryState::InProgress,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(c2.owner(), Some("dan@example.com"));
}

fn validation(e: HandlerError) -> String {
    match e {
        HandlerError::Validation(m) => m,
        other => panic!("wanted Validation, got {other:?}"),
    }
}

/// The refusal order `RequiredDecision::record_check` must keep: version, then no record, then stale reference, then
/// closed. The closed decisions below are the discriminating cases: a `record_check` that tested `Closed` first would
/// answer "decision is closed" for the closed-unlinked and closed-mismatched ones. Every refusal leaves the decision and
/// the log exactly as they were.
#[tokio::test]
async fn record_check_refuses_in_order_and_writes_nothing() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let d = dan();
    let s = chore(&h, "s", &["ok"], None).await;
    let open = h
        .require_decision(&d, "o", s.key(), "Open", "dan@example.com", None, None)
        .await
        .unwrap();
    let unlinked = h
        .require_decision(&d, "u", s.key(), "Unlinked", "dan@example.com", None, None)
        .await
        .unwrap();
    let linked = h
        .require_decision(&d, "l", s.key(), "Linked", "dan@example.com", None, None)
        .await
        .unwrap();
    h.link_record(&d, open.id(), "kb", "adrs/0001.md", "kb/ADR-0001", None)
        .await
        .unwrap();
    h.link_record(&d, linked.id(), "kb", "adrs/0002.md", "kb/ADR-0002", None)
        .await
        .unwrap();
    h.close_decision(&d, unlinked.id(), ClosureKind::Withdrawn, "gone")
        .await
        .unwrap();
    h.close_decision(&d, linked.id(), ClosureKind::Withdrawn, "gone")
        .await
        .unwrap();

    // (decision id, record id sent, version offset from the current one, expected refusal)
    enum Want {
        Msg(&'static str),
        Conflict,
    }
    let cases = [
        (open.id(), "kb/ADR-0001", 1, Want::Conflict), // stale version first
        (unlinked.id(), "kb/ADR-0001", 1, Want::Conflict), // version before no-record
        (
            unlinked.id(),
            "kb/ADR-0001",
            0,
            Want::Msg("decision has no record reference"),
        ), // no record before closed
        (open.id(), "kb/ADR-0009", 0, Want::Conflict), // stale reference
        (linked.id(), "kb/ADR-0009", 0, Want::Conflict), // stale reference before closed
        (
            linked.id(),
            "kb/ADR-0002",
            0,
            Want::Msg("decision is closed"),
        ),
    ];
    for (id, rec, offset, want) in cases {
        let before = h.get_decision(id).await.unwrap();
        let log_before = sink.on(&id.to_string());
        let err = h
            .record_check(
                &d,
                id,
                rec,
                before.version() + offset,
                CheckResult::NotFound,
                true,
            )
            .await
            .unwrap_err();
        match (want, err) {
            (Want::Msg(m), HandlerError::Validation(got)) => assert_eq!(got, m, "{id} {rec}"),
            (Want::Conflict, HandlerError::Conflict { current }) => {
                assert_eq!(current, before.version(), "{id} {rec}")
            }
            (_, other) => panic!("{id} {rec} {offset}: {other:?}"),
        }
        assert_eq!(h.get_decision(id).await.unwrap(), before, "{id}: unchanged");
        assert_eq!(sink.on(&id.to_string()), log_before, "{id}: no entry");
    }
}

/// Link validation runs before the read (a missing id answers the validation message), the reference before the PR
/// url, and every message is today's, verbatim. Catches value objects that validate after the lookup or reword.
#[tokio::test]
async fn link_record_validates_first_with_todays_messages() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let d = dan();
    let missing = 999_999;
    let link = |id: i64,
                repo: &'static str,
                path: &'static str,
                rid: &'static str,
                pr: Option<&'static str>| {
        h.link_record(&d, id, repo, path, rid, pr.map(Into::into))
    };
    for (repo, path, rid, pr, want) in [
        (
            " ",
            "a.md",
            "kb/ADR-1",
            None,
            "repo, path and record_id are required",
        ),
        (
            "a/b",
            "a.md",
            "kb/ADR-1",
            None,
            "repo `a/b` is not a single repository name",
        ),
        (
            "kb",
            "a.md",
            " ",
            None,
            "repo, path and record_id are required",
        ),
        // Trimmed before validation: the message shows the trimmed value.
        (
            " kb ",
            "  /abs.md  ",
            "kb/ADR-1",
            None,
            "path `/abs.md` must be relative to the repository",
        ),
        (
            " kb ",
            " ./-x.md ",
            "kb/ADR-1",
            None,
            "path `./-x.md` must not start with `-`",
        ),
        (
            "kb",
            "a.md",
            "kb/ADR-1",
            Some("  javascript:x  "),
            "pr_url `javascript:x` must be an absolute http(s) URL",
        ),
        (
            "kb",
            "/abs.md",
            "kb/ADR-1",
            None,
            "path `/abs.md` must be relative to the repository",
        ),
        (
            "kb",
            "x/../y.md",
            "kb/ADR-1",
            None,
            "path `x/../y.md` must not contain a `..` component",
        ),
        ("kb", "./", "kb/ADR-1", None, "path `./` names no file"),
        (
            "kb",
            "./-x.md",
            "kb/ADR-1",
            None,
            "path `./-x.md` must not start with `-`",
        ),
        (
            "kb",
            "/abs.md",
            "kb/ADR-1",
            Some("javascript:x"),
            "path `/abs.md` must be relative to the repository",
        ),
        (
            "kb",
            "a.md",
            "kb/ADR-1",
            Some("javascript:x"),
            "pr_url `javascript:x` must be an absolute http(s) URL",
        ),
    ] {
        assert_eq!(
            validation(link(missing, repo, path, rid, pr).await.unwrap_err()),
            want,
            "{repo} {path} {pr:?}"
        );
    }
    let s = chore(&h, "s", &["ok"], None).await;
    let dec = h
        .require_decision(&d, "d", s.key(), "Pick", "dan@example.com", None, None)
        .await
        .unwrap();
    h.close_decision(&d, dec.id(), ClosureKind::Withdrawn, "gone")
        .await
        .unwrap();
    let before = h.get_decision(dec.id()).await.unwrap();
    let log_before = sink.on(&dec.id().to_string());
    assert_eq!(
        validation(
            link(dec.id(), "kb", "a.md", "kb/ADR-1", None)
                .await
                .unwrap_err()
        ),
        "decision is closed"
    );
    assert_eq!(h.get_decision(dec.id()).await.unwrap(), before);
    assert_eq!(sink.on(&dec.id().to_string()), log_before, "no entry");
}
