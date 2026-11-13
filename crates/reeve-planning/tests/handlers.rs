mod common;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use reeve_planning::*;
use reeve_protocol::{
    CheckResult, ClosureKind, DeliveryState, DoneFilter, ItemType, Key, Kind, Reason, Resolution,
    Reversibility, TokenKind, Via,
};
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

/// An entry with the probe's answers inside and outside the mutation's transaction.
type Seen = (ActivityRecord, Option<String>, Option<String>);
/// Records each entry with what `sql` answers at that moment (one nullable text column; `$1` is the entry's
/// `entity_key`), once on the mutation's own transaction and once on another pool connection. The first value shows the
/// change was made before the entry was written; the second, that the entry was written before the change committed.
struct ProbeSink {
    pool: sqlx::PgPool,
    sql: &'static str,
    seen: Mutex<Vec<Seen>>,
}
#[async_trait]
impl ActivitySink for ProbeSink {
    async fn record(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        rec: ActivityRecord,
    ) -> Result<(), RepoError> {
        let inside: Option<String> = sqlx::query_scalar(self.sql)
            .bind(&rec.entity_key)
            .fetch_one(&mut **tx)
            .await?;
        let outside: Option<String> = sqlx::query_scalar(self.sql)
            .bind(&rec.entity_key)
            .fetch_one(&self.pool)
            .await?;
        self.seen.lock().unwrap().push((rec, inside, outside));
        Ok(())
    }
}
impl ProbeSink {
    fn entries(&self, action: &str) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(r, ..)| r.action == action)
            .cloned()
            .collect()
    }
}
fn mk_probe(pool: sqlx::PgPool, sql: &'static str) -> (Handlers, Arc<ProbeSink>) {
    let sink = Arc::new(ProbeSink {
        pool: pool.clone(),
        sql,
        seen: Mutex::new(vec![]),
    });
    let h = Handlers {
        ports: make_planning_ports(pool.clone()),
        activity: sink.clone(),
        pool,
        settings: settings(),
    };
    (h, sink)
}

fn ctx(actor: &str, automation: bool) -> Ctx {
    ctx_via(
        actor,
        if automation {
            Via::Automation
        } else {
            Via::Interactive
        },
    )
}
fn ctx_via(actor: &str, via: Via) -> Ctx {
    Ctx {
        actor: actor.into(),
        via,
        now: Utc.with_ymd_and_hms(2026, 10, 26, 9, 0, 0).unwrap(),
    }
}
fn settings() -> Settings {
    Settings {
        key_prefix: "ABC".into(),
        horizon_buckets: vec!["Now".into(), "Later".into()],
        repos: vec!["knowledge-base".into(), "kb".into()],
        done_window_days: 14,
        sprint_length_days: 14,
        upkeep_line: 20,
    }
}

#[tokio::test]
async fn the_acceptance_flow_at_handler_level() {
    let (_c, pool) = common::pg().await;
    let ports = make_planning_ports(pool.clone());
    let sink = Arc::new(MemSink(Mutex::new(vec![])));
    let h = Handlers {
        ports: ports.clone(),
        activity: sink.clone(),
        pool: pool.clone(),
        settings: settings(),
    };
    let dan = ctx("dan@example.com", false);

    let ini = h
        .create_initiative(&dan, "i1", "Orion", "", "Now", None, None, None)
        .await
        .unwrap();
    let epic = h
        .create_epic(&dan, "e1", "Orion v2", Some(ini.key().clone()), "", None)
        .await
        .unwrap();
    let story = h
        .create_story(
            &dan,
            "s1",
            "Booking flow",
            Kind::Story,
            Some(epic.key().clone()),
            "",
            vec!["works".into()],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    // idempotent re-create returns the same key
    assert_eq!(
        h.create_story(
            &dan,
            "s1",
            "Booking flow",
            Kind::Story,
            Some(epic.key().clone()),
            "",
            vec!["works".into()],
            vec![],
            None,
            None
        )
        .await
        .unwrap()
        .key(),
        story.key()
    );

    // Discovery not marked: refused with the ancestor reasons.
    let entries_before = sink.0.lock().unwrap().len();
    let refused = h
        .move_story(&dan, story.key(), 1, DeliveryState::InProgress, None, None)
        .await
        .unwrap_err();
    assert_eq!(
        sink.0.lock().unwrap().len(),
        entries_before,
        "a refused move writes no activity entry"
    );
    let reasons = match refused {
        HandlerError::Refused(r) => r,
        e => panic!("{e:?}"),
    };
    assert!(
        reasons
            .iter()
            .any(|r| matches!(r, Reason::AncestorDiscoveryIncomplete { .. }))
    );

    h.mark_discovery(&dan, ItemType::Initiative, ini.key(), 1, true)
        .await
        .unwrap();
    h.mark_discovery(&dan, ItemType::Epic, epic.key(), 1, true)
        .await
        .unwrap();
    let d = h
        .require_decision(
            &dan,
            "d1",
            epic.key(),
            "Pick queue",
            "dan@example.com",
            None,
            None,
        )
        .await
        .unwrap();

    let r = h.readiness(story.key()).await.unwrap();
    assert_eq!(
        r.reasons,
        vec![Reason::DecisionNoRecord {
            decision_id: d.id(),
            title: "Pick queue".into()
        }]
    );

    // Automation cannot override; interactive can, and the override is recorded.
    let bot = ctx("dan@example.com", true);
    match h
        .move_story(
            &bot,
            story.key(),
            1,
            DeliveryState::InProgress,
            None,
            Some("urgent".into()),
        )
        .await
        .unwrap_err()
    {
        HandlerError::Refused(r) => assert!(r.contains(&Reason::AutomationCannotOverride)),
        e => panic!("{e:?}"),
    }
    h.move_story(
        &dan,
        story.key(),
        1,
        DeliveryState::InProgress,
        None,
        Some("urgent demo".into()),
    )
    .await
    .unwrap();
    assert!(
        sink.0
            .lock()
            .unwrap()
            .iter()
            .any(|a| a.action == "override" && a.payload["reason"] == "urgent demo")
    );
    assert!(sink.0.lock().unwrap().iter().any(|a| {
        a.action == "override"
            && a.payload["reasons"]
                .as_array()
                .is_some_and(|r| !r.is_empty())
    }));
    h.move_story(&dan, story.key(), 2, DeliveryState::Todo, None, None)
        .await
        .unwrap(); // release

    // Link, check accepted, ready.
    h.link_record(
        &dan,
        d.id(),
        "knowledge-base",
        "knowledge/adrs/0031-queue.md",
        "kb/ADR-0031",
        Some("https://github.com/x/y/pull/9".into()),
    )
    .await
    .unwrap();
    assert_eq!(
        h.readiness(story.key()).await.unwrap().reasons,
        vec![Reason::DecisionUnchecked {
            decision_id: d.id(),
            title: "Pick queue".into()
        }],
        "linking alone does not satisfy; the reason says a check is pending"
    );
    let linked = h.get_decision(d.id()).await.unwrap();
    h.record_check(
        &dan,
        d.id(),
        "kb/ADR-0031",
        linked.version(),
        CheckResult::ProposedPrOpen,
        true,
    )
    .await
    .unwrap();
    assert!(
        matches!(
            h.record_check(
                &dan,
                d.id(),
                "kb/ADR-0031",
                linked.version(),
                CheckResult::Accepted {
                    commit: "zzz".into(),
                    reversibility: None,
                },
                true
            )
            .await
            .unwrap_err(),
            HandlerError::Conflict { .. }
        ),
        "a check carrying a version older than the row is refused, even for the same record id"
    );
    assert!(matches!(
        h.readiness(story.key()).await.unwrap().reasons[0],
        Reason::DecisionProposedPrOpen { .. }
    ));
    let v = h.get_decision(d.id()).await.unwrap().version();
    h.record_check(
        &dan,
        d.id(),
        "kb/ADR-0031",
        v,
        CheckResult::Accepted {
            commit: "abc".into(),
            reversibility: None,
        },
        true,
    )
    .await
    .unwrap();
    assert!(h.readiness(story.key()).await.unwrap().ready);
    h.move_story(&dan, story.key(), 3, DeliveryState::InProgress, None, None)
        .await
        .unwrap();

    // Context bundle carries the decision and the readiness.
    let bundle = h.story_context(story.key(), Utc::now()).await.unwrap();
    assert_eq!(bundle.required_decisions.len(), 1);
    assert!(bundle.readiness.ready);
    assert_eq!(bundle.initiative.as_ref().unwrap().key, *ini.key());

    // Done needs a resolution; dropping from todo is allowed; stale version is a conflict.
    let s2 = h
        .create_story(
            &dan,
            "s2",
            "Old idea",
            Kind::Chore,
            None,
            "",
            vec![],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    assert!(matches!(
        h.move_story(&dan, s2.key(), 1, DeliveryState::Done, None, None)
            .await
            .unwrap_err(),
        HandlerError::Refused(_)
    ));
    h.move_story(
        &dan,
        s2.key(),
        1,
        DeliveryState::Done,
        Some(Resolution::Dropped),
        None,
    )
    .await
    .unwrap();
    assert!(matches!(
        h.move_story(&dan, s2.key(), 1, DeliveryState::Todo, None, None)
            .await
            .unwrap_err(),
        HandlerError::Conflict { current: 2 }
    ));

    // Progress excludes dropped work: a second story under the epic is dropped and leaves the denominator at one.
    let s3 = h
        .create_story(
            &dan,
            "s3",
            "Dropped idea",
            Kind::Story,
            Some(epic.key().clone()),
            "",
            vec![],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    h.move_story(
        &dan,
        s3.key(),
        1,
        DeliveryState::Done,
        Some(Resolution::Dropped),
        None,
    )
    .await
    .unwrap();
    let cur = h.get_story(story.key()).await.unwrap();
    h.move_story(
        &dan,
        story.key(),
        cur.version(),
        DeliveryState::Done,
        Some(Resolution::Completed),
        None,
    )
    .await
    .unwrap();
    let dto = h.initiative_dto(ini.key(), Utc::now()).await.unwrap();
    assert_eq!(
        dto.progress,
        Some(1.0),
        "completed 1 of 1 non-dropped stories"
    );
}

fn mk(pool: sqlx::PgPool) -> (Handlers, Arc<MemSink>) {
    let sink = Arc::new(MemSink(Mutex::new(vec![])));
    let h = Handlers {
        ports: make_planning_ports(pool.clone()),
        activity: sink.clone(),
        pool,
        settings: settings(),
    };
    (h, sink)
}

/// An epic that is discovery-complete under no initiative: a story placed under it is ready when its own checks pass.
/// Call once per test (fixed idempotency key).
async fn ready_epic(h: &Handlers, c: &Ctx) -> reeve_protocol::Key {
    let epic = h
        .create_epic(c, "ready-epic", "E", None, "", None)
        .await
        .unwrap();
    h.mark_discovery(c, ItemType::Epic, epic.key(), epic.version(), true)
        .await
        .unwrap();
    epic.key().clone()
}

fn count(sink: &MemSink, action: &str, key: &str) -> usize {
    sink.0
        .lock()
        .unwrap()
        .iter()
        .filter(|a| a.action == action && a.entity_key == key)
        .count()
}

#[tokio::test]
async fn create_replays_record_one_created_entry() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);

    // A row that already exists under the key (inserted directly): create returns it and writes no entry.
    let mut tx = pool.begin().await.unwrap();
    let key = h.ports.keys.next_key(&mut tx, "ABC").await.unwrap();
    let pre = Story::new(
        key,
        "Pre-existing",
        Kind::Story,
        None,
        "",
        vec![],
        None,
        vec![],
    )
    .unwrap();
    assert!(
        h.ports
            .stories
            .insert_story(&mut tx, &pre, "k-direct", dan.now)
            .await
            .unwrap()
    );
    tx.commit().await.unwrap();
    let got = h
        .create_story(
            &dan,
            "k-direct",
            "Other title",
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
    assert_eq!(got.key(), pre.key());
    assert_eq!(count(&sink, "created", &pre.key().to_string()), 0);

    // Overlapping creates with one key (the insert-false path can fire): one item, one `created` entry.
    let (a, b) = tokio::join!(
        h.create_story(
            &dan,
            "k-race",
            "Racing",
            Kind::Story,
            None,
            "",
            vec![],
            vec![],
            None,
            None
        ),
        h.create_story(
            &dan,
            "k-race",
            "Racing",
            Kind::Story,
            None,
            "",
            vec![],
            vec![],
            None,
            None
        ),
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a.key(), b.key());
    assert_eq!(count(&sink, "created", &a.key().to_string()), 1);

    // ...and with an external reference: the loser waits on the reference's lock, then still answers the same item
    // rather than refusing the reference its own retry holds.
    for n in 0..10 {
        let (idem, r) = (format!("k-ref-{n}"), format!("SCRUM-{}", 900 + n));
        let create = || {
            h.create_story(
                &dan,
                &idem,
                "Racing",
                Kind::Story,
                None,
                "",
                vec![],
                vec![],
                Some(r.clone()),
                None,
            )
        };
        let (a, b) = tokio::join!(create(), create());
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(a.key(), b.key());
        assert_eq!(a.external_ref(), Some(r.as_str()));
        assert_eq!(count(&sink, "created", &a.key().to_string()), 1);
    }
}

/// The reference check runs on the create's own connection: holding the reference's lock, it takes no second one, so
/// a refusal answers even when the pool has a single connection (it would otherwise wait out the acquire timeout).
#[tokio::test]
async fn a_held_reference_is_refused_on_the_creates_own_connection() {
    let (_c, pool) = common::pg().await;
    let one = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(2))
        .connect_with((*pool.connect_options()).clone())
        .await
        .unwrap();
    let (h, _sink) = mk(one);
    let dan = ctx("dan@example.com", false);
    let story = |idem: &'static str| {
        h.create_story(
            &dan,
            idem,
            "Held",
            Kind::Story,
            None,
            "",
            vec![],
            vec![],
            Some("SCRUM-77".into()),
            None,
        )
    };
    let first = story("a").await.unwrap();
    match story("b").await {
        Err(HandlerError::Validation(m)) => assert_eq!(
            m,
            format!(
                "SCRUM-77 is already the external reference of story {}",
                first.key()
            )
        ),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn concurrent_moves_with_one_version_have_one_winner() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let epic = ready_epic(&h, &dan).await;
    let s = h
        .create_story(
            &dan,
            "s",
            "Race",
            Kind::Chore,
            Some(epic.clone()),
            "",
            vec!["ok".into()],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        h.move_story(&dan, s.key(), 1, DeliveryState::InProgress, None, None),
        h.move_story(&dan, s.key(), 1, DeliveryState::InProgress, None, None),
    );
    let results = [a, b];
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "{results:?}"
    );
    assert!(
        results
            .iter()
            .any(|r| matches!(r, Err(HandlerError::Conflict { .. }))),
        "{results:?}"
    );
}

/// Distinguishes a handler that stores and compares owners verbatim (a mixed-case assignee vanishing from `--mine`)
/// from one that normalises on write and on the filter.
#[tokio::test]
async fn owners_are_normalised_on_write_and_on_the_list_filter() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let s = h
        .create_story(
            &dan,
            "own",
            "Owned",
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
    let e = h
        .edit_story(
            &dan,
            s.key(),
            s.version(),
            None,
            None,
            None,
            None,
            None,
            Some(Some(" Bob@Example.com ".into())),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(e.owner(), Some("bob@example.com"));
    for filter in ["bob@example.com", "BOB@example.com "] {
        let f = StoryFilter {
            owner: Some(filter.into()),
            ..Default::default()
        };
        let found = h
            .list_stories(&f, None, None, false, DoneFilter::All, Utc::now())
            .await
            .unwrap();
        assert_eq!(found.len(), 1, "filter {filter:?}");
    }
}

/// A content edit of a neighbour that a rerank renumbered keeps the rerank's rank (the repo test pins the stale-read case).
#[tokio::test]
async fn editing_a_title_after_a_rerank_keeps_the_reranked_rank() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let mut keys = Vec::new();
    for (i, t) in ["A", "B", "C"].iter().enumerate() {
        let s = h
            .create_story(
                &dan,
                &format!("e{i}"),
                t,
                Kind::Chore,
                None,
                "",
                vec![],
                vec![],
                None,
                None,
            )
            .await
            .unwrap();
        keys.push(s.key().clone());
    }
    let b = h.get_story(&keys[1]).await.unwrap();
    h.rank_story(&dan, &keys[2], 1, Some(&keys[0]))
        .await
        .unwrap();
    let ranked = h.get_story(&keys[1]).await.unwrap().rank();
    assert_ne!(ranked, b.rank());
    let e = h
        .edit_story(
            &dan,
            &keys[1],
            b.version(),
            Some("B2".into()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(e.rank(), ranked);
}

#[tokio::test]
async fn smaller_use_cases() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan@example.com", false);

    // edit_story trims the title and criteria.
    let s = h
        .create_story(
            &dan,
            "s",
            "Title",
            Kind::Story,
            None,
            "",
            vec!["a".into()],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let e = h
        .edit_story(
            &dan,
            s.key(),
            1,
            Some("  Padded  ".into()),
            None,
            None,
            None,
            Some(vec![Criterion {
                text: "  crit  ".into(),
                done: false,
            }]),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(e.title(), "Padded");
    assert_eq!(e.criteria()[0].text, "crit");
    assert!(matches!(
        h.edit_story(
            &dan,
            s.key(),
            2,
            Some("   ".into()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None
        )
        .await
        .unwrap_err(),
        HandlerError::Validation(_)
    ));

    // A story cannot block itself; a real edge is logged once.
    assert!(matches!(
        h.add_block(&dan, s.key(), s.key()).await.unwrap_err(),
        HandlerError::Validation(_)
    ));
    let t = h
        .create_story(
            &dan,
            "t",
            "Other",
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
    h.add_block(&dan, s.key(), t.key()).await.unwrap();
    h.add_block(&dan, s.key(), t.key()).await.unwrap();
    assert_eq!(count(&sink, "blocked_by", &s.key().to_string()), 1);

    // Comments.
    let id = h
        .comment_story(&dan, s.key(), "  hello ", None)
        .await
        .unwrap();
    assert!(id > 0);
    assert_eq!(h.comments_for_story(s.key()).await.unwrap().len(), 1);
    assert!(matches!(
        h.comment_story(&dan, s.key(), "  ", None)
            .await
            .unwrap_err(),
        HandlerError::Validation(_)
    ));

    // close_decision: closes once, then refuses.
    let d = h
        .require_decision(&dan, "d", s.key(), "Pick", "dan", None, None)
        .await
        .unwrap();
    assert!(matches!(
        h.link_record(&dan, d.id(), "kb", " ", "kb/ADR-1", None)
            .await
            .unwrap_err(),
        HandlerError::Validation(_)
    ));
    let closed = h
        .close_decision(&dan, d.id(), ClosureKind::Withdrawn, " not needed ")
        .await
        .unwrap();
    assert_eq!(closed.closed().as_ref().unwrap().reason, "not needed");
    assert!(
        matches!(h.close_decision(&dan, d.id(), ClosureKind::Withdrawn, "again").await.unwrap_err(), HandlerError::Validation(m) if m == "already closed")
    );

    // mark_discovery: re-marking keeps the first mark and logs nothing; reopening clears it.
    let ini = h
        .create_initiative(&dan, "i", "Init", "", "Now", None, None, None)
        .await
        .unwrap();
    h.mark_discovery(&dan, ItemType::Initiative, ini.key(), 1, true)
        .await
        .unwrap();
    let v = h
        .initiative_dto(ini.key(), Utc::now())
        .await
        .unwrap()
        .version;
    h.mark_discovery(&dan, ItemType::Initiative, ini.key(), v, true)
        .await
        .unwrap();
    assert_eq!(count(&sink, "discovery_marked", &ini.key().to_string()), 1);
    // A stale version is a conflict even when the mark would not change.
    assert!(matches!(
        h.mark_discovery(&dan, ItemType::Initiative, ini.key(), v - 1, true)
            .await
            .unwrap_err(),
        HandlerError::Conflict { current } if current == v
    ));
    h.mark_discovery(&dan, ItemType::Initiative, ini.key(), v, false)
        .await
        .unwrap();
    assert_eq!(
        count(&sink, "discovery_reopened", &ini.key().to_string()),
        1
    );
    assert!(
        h.initiative_dto(ini.key(), Utc::now())
            .await
            .unwrap()
            .discovery_complete
            .is_none()
    );
}

#[tokio::test]
async fn item_scoped_decision_list_honours_open_with_record() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let s = h
        .create_story(
            &dan,
            "s",
            "S",
            Kind::Chore,
            None,
            "",
            vec![],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let mut ids = vec![];
    for k in ["linked", "unlinked", "closed"] {
        ids.push(
            h.require_decision(&dan, k, s.key(), k, "dan@example.com", None, None)
                .await
                .unwrap()
                .id(),
        );
    }
    for id in [ids[0], ids[2]] {
        h.link_record(
            &dan,
            id,
            "knowledge-base",
            "knowledge/adrs/0001.md",
            "kb/ADR-0001",
            None,
        )
        .await
        .unwrap();
    }
    h.close_decision(&dan, ids[2], ClosureKind::Withdrawn, "not needed")
        .await
        .unwrap();
    let all: Vec<i64> = h
        .list_decisions(Some(s.key()), false, false)
        .await
        .unwrap()
        .iter()
        .map(|d| d.id())
        .collect();
    assert_eq!(all, ids);
    let open_linked: Vec<i64> = h
        .list_decisions(Some(s.key()), true, false)
        .await
        .unwrap()
        .iter()
        .map(|d| d.id())
        .collect();
    assert_eq!(open_linked, vec![ids[0]]);
    // The context bundle leaves the closed decision out.
    let bundle: Vec<i64> = h
        .story_context(s.key(), Utc::now())
        .await
        .unwrap()
        .required_decisions
        .iter()
        .map(|d| d.id)
        .collect();
    assert_eq!(bundle, vec![ids[0], ids[1]]);
}

#[tokio::test]
async fn unscoped_decision_list_includes_closed_only_when_asked() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let s = h
        .create_story(
            &dan,
            "s",
            "S",
            Kind::Chore,
            None,
            "",
            vec![],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let mut ids = vec![];
    for k in ["open", "closed"] {
        ids.push(
            h.require_decision(&dan, k, s.key(), k, "dan@example.com", None, None)
                .await
                .unwrap()
                .id(),
        );
    }
    h.link_record(
        &dan,
        ids[1],
        "knowledge-base",
        "knowledge/adrs/0001.md",
        "kb/ADR-0001",
        None,
    )
    .await
    .unwrap();
    h.close_decision(&dan, ids[1], ClosureKind::Withdrawn, "not needed")
        .await
        .unwrap();
    let list = |open_with_record: bool, include_closed: bool| {
        let h = &h;
        async move {
            h.list_decisions(None, open_with_record, include_closed)
                .await
                .unwrap()
                .iter()
                .map(|d| d.id())
                .collect::<Vec<i64>>()
        }
    };
    assert_eq!(list(false, false).await, vec![ids[0]]);
    assert_eq!(list(false, true).await, ids);
    // `open_with_record` asks for open decisions, so it wins over `include_closed`.
    assert_eq!(list(true, true).await, Vec::<i64>::new());
}

#[tokio::test]
async fn ready_filter_lists_only_pickable_work() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let epic = ready_epic(&h, &dan).await;
    let mut keys = vec![];
    for k in ["todo", "active", "done"] {
        keys.push(
            h.create_story(
                &dan,
                k,
                k,
                Kind::Chore,
                Some(epic.clone()),
                "",
                vec!["ok".into()],
                vec![],
                None,
                None,
            )
            .await
            .unwrap()
            .key()
            .clone(),
        );
    }
    h.move_story(&dan, &keys[1], 1, DeliveryState::InProgress, None, None)
        .await
        .unwrap();
    h.move_story(&dan, &keys[2], 1, DeliveryState::InProgress, None, None)
        .await
        .unwrap();
    h.move_story(
        &dan,
        &keys[2],
        2,
        DeliveryState::Done,
        Some(Resolution::Completed),
        None,
    )
    .await
    .unwrap();
    let all = h
        .list_stories(
            &StoryFilter::default(),
            None,
            None,
            false,
            DoneFilter::All,
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(all.len(), 3);
    assert!(
        all.iter().all(|x| x.readiness.ready),
        "readiness is still computed in every state"
    );
    let ready: Vec<_> = h
        .list_stories(
            &StoryFilter::default(),
            None,
            None,
            true,
            DoneFilter::All,
            Utc::now(),
        )
        .await
        .unwrap()
        .into_iter()
        .map(|x| x.story.key)
        .collect();
    assert_eq!(
        ready,
        vec![keys[0].clone()],
        "in-progress and done stories are not pickable"
    );
}

#[tokio::test]
async fn link_refuses_unconfigured_repos_and_escaping_paths() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let s = h
        .create_story(
            &dan,
            "s",
            "S",
            Kind::Chore,
            None,
            "",
            vec![],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let d = h
        .require_decision(&dan, "d", s.key(), "Pick", "dan@example.com", None, None)
        .await
        .unwrap();
    // A repository need not be configured, but its name is one plain directory name, as a checker uses it.
    for (repo, path, needle) in [
        (
            "../../secrets",
            "docs/adr/1.md",
            "not a single repository name",
        ),
        (
            "/home/ci/x",
            "docs/adr/1.md",
            "not a single repository name",
        ),
        ("-x", "docs/adr/1.md", "not a single repository name"),
        ("knowledge-base", "/etc/passwd", "relative"),
        ("knowledge-base", "docs/../../x.md", "`..`"),
        ("knowledge-base", "..", "`..`"),
        ("knowledge-base", "-x.md", "`-`"),
        ("knowledge-base", "./-x.md", "`-`"),
    ] {
        match h
            .link_record(&dan, d.id(), repo, path, "kb/ADR-1", None)
            .await
            .unwrap_err()
        {
            HandlerError::Validation(m) => assert!(m.contains(needle), "{repo} {path}: {m}"),
            e => panic!("{repo} {path}: {e:?}"),
        }
    }
    assert_eq!(count(&sink, "linked", &d.id().to_string()), 0);
    h.link_record(
        &dan,
        d.id(),
        "knowledge-base",
        "knowledge/adrs/0001..draft.md",
        "kb/ADR-1",
        None,
    )
    .await
    .unwrap();
}

/// A path is stored in normal form (no empty or `.` components), so the checker compares the name `ls-tree` prints
/// against what was linked, and a path naming no file at all is refused.
#[tokio::test]
async fn link_stores_the_path_in_normal_form() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let s = h
        .create_story(
            &dan,
            "s",
            "S",
            Kind::Chore,
            None,
            "",
            vec![],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let d = h
        .require_decision(&dan, "d", s.key(), "Pick", "dan@example.com", None, None)
        .await
        .unwrap();
    for path in [".", "./", ".//./"] {
        match h
            .link_record(&dan, d.id(), "knowledge-base", path, "kb/ADR-1", None)
            .await
            .unwrap_err()
        {
            HandlerError::Validation(m) => assert!(m.contains("names no file"), "{path}: {m}"),
            e => panic!("{path}: {e:?}"),
        }
    }
    assert_eq!(count(&sink, "linked", &d.id().to_string()), 0);
    let linked = h
        .link_record(
            &dan,
            d.id(),
            "knowledge-base",
            " ./knowledge//adrs/./0001.md ",
            "kb/ADR-1",
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        linked.record().unwrap().path(),
        "knowledge/adrs/0001.md",
        "stored in normal form"
    );
    let logged = sink
        .0
        .lock()
        .unwrap()
        .iter()
        .find(|a| a.action == "linked")
        .map(|a| a.payload["path"].clone());
    assert_eq!(
        logged,
        Some(serde_json::json!("knowledge/adrs/0001.md")),
        "logged as stored"
    );
}

#[tokio::test]
async fn link_refuses_a_pr_url_that_is_not_http() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let s = h
        .create_story(
            &dan,
            "s",
            "S",
            Kind::Chore,
            None,
            "",
            vec![],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let d = h
        .require_decision(&dan, "d", s.key(), "Pick", "dan@example.com", None, None)
        .await
        .unwrap();
    for bad in [
        "javascript:alert(1)",
        "JavaScript:alert(1)",
        "data:text/html,<script>alert(1)</script>",
        "ftp://example.com/pr/1",
        "/relative/pull/1",
        "https://",
        "",
    ] {
        match h
            .link_record(
                &dan,
                d.id(),
                "knowledge-base",
                "docs/adr/1.md",
                "kb/ADR-1",
                Some(bad.into()),
            )
            .await
            .unwrap_err()
        {
            HandlerError::Validation(m) => assert!(m.contains("http(s)"), "{bad}: {m}"),
            e => panic!("{bad}: {e:?}"),
        }
    }
    assert_eq!(count(&sink, "linked", &d.id().to_string()), 0);
    let linked = h
        .link_record(
            &dan,
            d.id(),
            "knowledge-base",
            "docs/adr/1.md",
            "kb/ADR-1",
            Some(" https://github.com/x/y/pull/1 ".into()),
        )
        .await
        .unwrap();
    assert_eq!(linked.pr_url(), Some("https://github.com/x/y/pull/1"));
}

#[tokio::test]
async fn comment_replay_with_the_same_key_returns_the_same_comment() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let s = h
        .create_story(
            &dan,
            "s",
            "S",
            Kind::Chore,
            None,
            "",
            vec![],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let first = h
        .comment_story(&dan, s.key(), "queue chosen", Some("c1"))
        .await
        .unwrap();
    let again = h
        .comment_story(&dan, s.key(), "queue chosen", Some("c1"))
        .await
        .unwrap();
    assert_eq!(first, again);
    assert_eq!(h.comments_for_story(s.key()).await.unwrap().len(), 1);
    assert_eq!(count(&sink, "commented", &s.key().to_string()), 1);
    // A different key is a different comment; so is no key at all.
    let other = h
        .comment_story(&dan, s.key(), "queue chosen", Some("c2"))
        .await
        .unwrap();
    assert_ne!(other, first);
    h.comment_story(&dan, s.key(), "no key", None)
        .await
        .unwrap();
    h.comment_story(&dan, s.key(), "no key", None)
        .await
        .unwrap();
    assert_eq!(h.comments_for_story(s.key()).await.unwrap().len(), 4);
    // Decisions take the same key.
    let d = h
        .require_decision(&dan, "d", s.key(), "Pick", "dan@example.com", None, None)
        .await
        .unwrap();
    let a = h
        .comment_decision(&dan, d.id(), "why", Some("dc1"))
        .await
        .unwrap();
    let b = h
        .comment_decision(&dan, d.id(), "why", Some("dc1"))
        .await
        .unwrap();
    assert_eq!(a, b);
    assert_eq!(h.comments_for_decision(d.id()).await.unwrap().len(), 1);
}

#[tokio::test]
async fn done_to_done_with_a_new_resolution_changes_it_and_logs_it() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let epic = ready_epic(&h, &dan).await;
    let s = h
        .create_story(
            &dan,
            "s",
            "S",
            Kind::Chore,
            Some(epic.clone()),
            "",
            vec!["ok".into()],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let key = s.key().to_string();
    h.move_story(&dan, s.key(), 1, DeliveryState::InProgress, None, None)
        .await
        .unwrap();
    h.move_story(
        &dan,
        s.key(),
        2,
        DeliveryState::Done,
        Some(Resolution::Completed),
        None,
    )
    .await
    .unwrap();
    // Same resolution: nothing changes, nothing is logged.
    let same = h
        .move_story(
            &dan,
            s.key(),
            3,
            DeliveryState::Done,
            Some(Resolution::Completed),
            None,
        )
        .await
        .unwrap();
    assert_eq!(same.version(), 3);
    assert_eq!(count(&sink, "moved", &key), 2);
    // A different resolution is applied and logged.
    let dropped = h
        .move_story(
            &dan,
            s.key(),
            3,
            DeliveryState::Done,
            Some(Resolution::Dropped),
            None,
        )
        .await
        .unwrap();
    assert_eq!(dropped.resolution(), Some(Resolution::Dropped));
    assert_eq!(count(&sink, "moved", &key), 3);
    // Back to completed needs active work first.
    assert!(matches!(
        h.move_story(
            &dan,
            s.key(),
            4,
            DeliveryState::Done,
            Some(Resolution::Completed),
            None
        )
        .await
        .unwrap_err(),
        HandlerError::Refused(_)
    ));
}

#[tokio::test]
async fn rank_story_renumbers_todo_in_one_write_and_bumps_only_the_moved_story() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let epic = ready_epic(&h, &dan).await;
    let mut keys = Vec::new();
    for (i, t) in ["A", "B", "C", "D"].iter().enumerate() {
        let s = h
            .create_story(
                &dan,
                &format!("r{i}"),
                t,
                Kind::Chore,
                Some(epic.clone()),
                "",
                vec!["ok".into()],
                vec![],
                None,
                None,
            )
            .await
            .unwrap();
        keys.push(s.key().clone());
    }
    // D leaves todo: it can neither be ranked nor be ranked before.
    h.move_story(&dan, &keys[3], 1, DeliveryState::InProgress, None, None)
        .await
        .unwrap();
    let todo = |h: &Handlers| {
        let h = h.clone();
        async move {
            h.list_stories(
                &StoryFilter {
                    delivery: Some(DeliveryState::Todo),
                    ..Default::default()
                },
                None,
                None,
                false,
                DoneFilter::All,
                Utc::now(),
            )
            .await
            .unwrap()
            .into_iter()
            .map(|x| (x.story.key, x.story.version, x.story.rank))
            .collect::<Vec<_>>()
        }
    };
    // Every new story has rank 0, so the column starts in key order: A, B, C.
    let before = todo(&h).await;
    assert_eq!(
        before.iter().map(|x| x.0.clone()).collect::<Vec<_>>(),
        keys[..3].to_vec()
    );

    // Move the last before the first.
    let moved = h
        .rank_story(&dan, &keys[2], 1, Some(&keys[0]))
        .await
        .unwrap();
    assert_eq!(moved.version(), 2);
    let after = todo(&h).await;
    assert_eq!(
        after.iter().map(|x| x.0.clone()).collect::<Vec<_>>(),
        vec![keys[2].clone(), keys[0].clone(), keys[1].clone()]
    );
    assert_eq!(after.iter().map(|x| x.2).collect::<Vec<_>>(), vec![0, 1, 2]);
    // Only the moved story's version changed; the neighbours' ranks changed without a bump.
    assert_eq!(after[1].1, 1);
    assert_eq!(after[2].1, 1);
    let k2 = keys[2].to_string();
    assert_eq!(count(&sink, "reranked", &k2), 1);
    let total = sink
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|a| a.action == "reranked")
        .count();
    assert_eq!(total, 1, "one entry for the whole renumbering");
    let payload = sink
        .0
        .lock()
        .unwrap()
        .iter()
        .find(|a| a.action == "reranked")
        .unwrap()
        .payload
        .clone();
    assert_eq!(payload, serde_json::json!({"from": 2, "to": 0}));

    // A placement that changes nothing writes nothing.
    let same = h
        .rank_story(&dan, &keys[2], 2, Some(&keys[0]))
        .await
        .unwrap();
    assert_eq!(same.version(), 2);
    assert_eq!(count(&sink, "reranked", &k2), 1);

    // To the end.
    let end = h.rank_story(&dan, &keys[2], 2, None).await.unwrap();
    assert_eq!(end.rank(), 2);

    // A stale version is a conflict and changes nothing.
    assert!(matches!(
        h.rank_story(&dan, &keys[2], 2, Some(&keys[0]))
            .await
            .unwrap_err(),
        HandlerError::Conflict { current: 3 }
    ));
    assert_eq!(todo(&h).await.last().unwrap().0, keys[2]);

    // Only todo stories are ranked, and only before a todo story.
    assert!(matches!(
        h.rank_story(&dan, &keys[3], 2, None).await.unwrap_err(),
        HandlerError::Validation(m) if m.contains("only todo stories")
    ));
    assert!(matches!(
        h.rank_story(&dan, &keys[0], 1, Some(&keys[3]))
            .await
            .unwrap_err(),
        HandlerError::Validation(_)
    ));
    assert!(matches!(
        h.rank_story(&dan, &keys[0], 1, Some(&keys[0]))
            .await
            .unwrap_err(),
        HandlerError::Validation(_)
    ));
    assert_eq!(count(&sink, "reranked", &k2), 2);
}

#[tokio::test]
async fn only_interactive_callers_may_override_a_system_actor_is_refused() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let story = h
        .create_story(
            &dan,
            "s1",
            "No criteria",
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
    let system = ctx_via("github", Via::System);
    match h
        .move_story(
            &system,
            story.key(),
            1,
            DeliveryState::InProgress,
            None,
            Some("ship it".into()),
        )
        .await
        .unwrap_err()
    {
        HandlerError::Refused(r) => {
            assert!(r.contains(&Reason::NoCriteria), "{r:?}");
            assert!(r.contains(&Reason::AutomationCannotOverride), "{r:?}");
        }
        e => panic!("{e:?}"),
    }
    assert_eq!(count(&sink, "override", &story.key().to_string()), 0);
    h.move_story(
        &dan,
        story.key(),
        1,
        DeliveryState::InProgress,
        None,
        Some("ship it".into()),
    )
    .await
    .unwrap();
}

fn pull_request(
    number: i64,
    status: reeve_protocol::PrStatus,
    updated_at: chrono::DateTime<Utc>,
    title: &str,
) -> PullRequest {
    PullRequest {
        repo: "acme/portal".into(),
        number,
        url: format!("https://github.com/acme/portal/pull/{number}"),
        title: title.into(),
        status,
        updated_at,
    }
}

#[tokio::test]
async fn recording_a_pull_request_links_updates_and_never_moves_the_story() {
    use chrono::Duration;
    use reeve_protocol::PrStatus;
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool.clone());
    let c = ctx("dan@example.com", false);
    let story = h
        .create_story(
            &c,
            "s1",
            "Match",
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
    let pr = |status, updated, title: &str| pull_request(42, status, updated, title);
    let t0 = c.now;
    let gh = Ctx {
        actor: "github".into(),
        via: Via::System,
        now: t0,
    };
    assert_eq!(
        h.record_pull_request(&gh, story.key(), pr(PrStatus::Open, t0, "fix"))
            .await
            .unwrap(),
        PrChange::Linked
    );
    let dto = h.story_dto(story.key(), Utc::now()).await.unwrap();
    assert_eq!(dto.prs.len(), 1);
    assert_eq!(dto.prs[0].status, PrStatus::Open);
    assert_eq!(dto.prs[0].number, 42);
    assert_eq!(dto.prs[0].url, "https://github.com/acme/portal/pull/42");
    // newer → Updated; older → Ignored; exact duplicate → Ignored; same time, different payload → Updated (last write wins)
    let t10 = t0 + Duration::seconds(10);
    assert_eq!(
        h.record_pull_request(&gh, story.key(), pr(PrStatus::Merged, t10, "fix"))
            .await
            .unwrap(),
        PrChange::Updated
    );
    assert_eq!(
        h.record_pull_request(
            &gh,
            story.key(),
            pr(PrStatus::Draft, t0 + Duration::seconds(5), "fix")
        )
        .await
        .unwrap(),
        PrChange::Ignored
    );
    assert_eq!(
        h.record_pull_request(&gh, story.key(), pr(PrStatus::Merged, t10, "fix"))
            .await
            .unwrap(),
        PrChange::Ignored
    );
    assert_eq!(
        h.record_pull_request(
            &gh,
            story.key(),
            pr(PrStatus::Merged, t10, "fix (retitled)")
        )
        .await
        .unwrap(),
        PrChange::Updated
    );
    // newer but identical (a push, a label): Touched, the time advances, no entry
    let updated_before = count(&sink, "pr_updated", &story.key().to_string());
    let t12 = t10 + Duration::seconds(2);
    assert_eq!(
        h.record_pull_request(
            &gh,
            story.key(),
            pr(PrStatus::Merged, t12, "fix (retitled)")
        )
        .await
        .unwrap(),
        PrChange::Touched
    );
    assert_eq!(
        count(&sink, "pr_updated", &story.key().to_string()),
        updated_before,
        "a touch writes no activity entry"
    );
    // the advanced time guards against a late older event with a different status
    assert_eq!(
        h.record_pull_request(
            &gh,
            story.key(),
            pr(
                PrStatus::Draft,
                t10 + Duration::seconds(1),
                "fix (retitled)"
            )
        )
        .await
        .unwrap(),
        PrChange::Ignored
    );
    let fresh = h.get_story(story.key()).await.unwrap();
    let dto = h.story_dto(fresh.key(), Utc::now()).await.unwrap();
    assert_eq!(dto.prs[0].status, PrStatus::Merged);
    assert_eq!(dto.prs[0].title, "fix (retitled)");
    assert_eq!(dto.prs[0].updated_at, t12);
    assert_eq!(
        dto.delivery,
        DeliveryState::Todo,
        "pull-request status must never move a story (kb/ADR-0002 R4)"
    );
    assert_eq!(
        fresh.version(),
        story.version(),
        "linking a pull request is not an edit of the story"
    );
    // re-linking the same PR to another story moves the row
    let other = h
        .create_story(
            &c,
            "s2",
            "Other",
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
    assert_eq!(
        h.record_pull_request(
            &gh,
            other.key(),
            pr(PrStatus::Merged, t0 + Duration::seconds(20), "x")
        )
        .await
        .unwrap(),
        PrChange::Relinked {
            from: story.key().clone()
        }
    );
    assert!(
        h.story_dto(story.key(), Utc::now())
            .await
            .unwrap()
            .prs
            .is_empty()
    );
    assert_eq!(
        h.story_dto(other.key(), Utc::now())
            .await
            .unwrap()
            .prs
            .len(),
        1
    );
    // an unknown story is NotFound and records nothing
    assert!(matches!(
        h.record_pull_request(
            &gh,
            &"ABC-999".parse().unwrap(),
            pr(PrStatus::Open, t0 + Duration::seconds(30), "x")
        )
        .await
        .unwrap_err(),
        HandlerError::NotFound
    ));
    // activity: pr_linked twice (link + relink), pr_updated twice; ignored events log nothing; via system
    assert_eq!(
        count(&sink, "pr_linked", &story.key().to_string())
            + count(&sink, "pr_linked", &other.key().to_string()),
        2
    );
    assert_eq!(count(&sink, "pr_linked", &other.key().to_string()), 1);
    assert_eq!(count(&sink, "pr_updated", &story.key().to_string()), 2);
    assert_eq!(count(&sink, "pr_unlinked", &story.key().to_string()), 1);
    let log = sink.0.lock().unwrap();
    let prs: Vec<_> = log.iter().filter(|r| r.action.starts_with("pr_")).collect();
    assert_eq!(prs.len(), 5);
    // the relink names both stories; every payload carries the title
    let unlinked = prs.iter().find(|r| r.action == "pr_unlinked").unwrap();
    assert_eq!(unlinked.entity_key, story.key().to_string());
    assert_eq!(unlinked.payload["to"], other.key().to_string());
    let relinked = prs
        .iter()
        .find(|r| r.action == "pr_linked" && r.entity_key == other.key().to_string())
        .unwrap();
    assert_eq!(relinked.payload["from"], story.key().to_string());
    let retitled = prs.iter().rfind(|r| r.action == "pr_updated").unwrap();
    assert_eq!(retitled.payload["title"], "fix (retitled)");
    assert!(prs.iter().all(|r| r.payload["title"].is_string()));
    assert!(
        prs.iter()
            .all(|r| r.via == Via::System && r.actor == "github" && r.entity_type == "story")
    );
}

/// `story_dtos_in` is the one assembly: on one read-only transaction it answers exactly what `story_dto` answers per
/// story (order of blockers and pull requests, stories with neither, an empty list), and blockers are in key-number
/// order (`ABC-9` before `ABC-10`; a text-ordered read would put `ABC-10` first).
#[tokio::test]
async fn story_dtos_in_assembles_and_matches_story_dto() {
    use reeve_protocol::PrStatus;
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let c = ctx("dan@example.com", false);
    let mut stories = Vec::new();
    for i in 0..10 {
        stories.push(
            h.create_story(
                &c,
                &format!("s{i}"),
                "S",
                Kind::Story,
                None,
                "",
                vec![],
                vec![],
                None,
                None,
            )
            .await
            .unwrap(),
        );
    }
    assert_eq!(stories[9].key().to_string(), "ABC-10");
    h.add_block(&c, stories[0].key(), stories[9].key())
        .await
        .unwrap();
    h.add_block(&c, stories[0].key(), stories[8].key())
        .await
        .unwrap();
    h.add_block(&c, stories[3].key(), stories[1].key())
        .await
        .unwrap();
    let gh = Ctx {
        actor: "github".into(),
        via: Via::System,
        now: c.now,
    };
    for (n, s) in [(7, 0), (3, 0), (9, 2)] {
        h.record_pull_request(
            &gh,
            stories[s].key(),
            pull_request(n, PrStatus::Open, c.now, "x"),
        )
        .await
        .unwrap();
    }
    let mut fresh = Vec::new();
    for s in &stories {
        fresh.push(h.get_story(s.key()).await.unwrap());
    }
    let mut tx = pool
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .unwrap();
    let urg = h.urgency_in(&mut tx, Utc::now()).await.unwrap();
    let clock = h.promise_clock_in(&mut tx, Utc::now()).await.unwrap();
    let batched = h
        .story_dtos_in(&mut tx, &fresh, &urg, &clock)
        .await
        .unwrap();
    assert!(
        h.story_dtos_in(&mut tx, &[], &urg, &clock)
            .await
            .unwrap()
            .is_empty()
    );
    tx.rollback().await.unwrap();
    let mut single = Vec::new();
    for s in &fresh {
        single.push(h.story_dto(s.key(), Utc::now()).await.unwrap());
    }
    assert_eq!(batched, single);
    assert_eq!(
        batched[0]
            .blocked_by
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["ABC-9", "ABC-10"]
    );
    // The readiness reasons name the open blockers in the same key-number order.
    let blocked_by_reasons: Vec<String> = h
        .readiness(stories[0].key())
        .await
        .unwrap()
        .reasons
        .into_iter()
        .filter_map(|r| match r {
            Reason::BlockedBy { story, .. } => Some(story.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(blocked_by_reasons, ["ABC-9", "ABC-10"]);
    assert_eq!(batched[3].blocked_by, vec![stories[1].key().clone()]);
    assert_eq!(
        batched[0].prs.iter().map(|p| p.number).collect::<Vec<_>>(),
        [3, 7]
    );
    assert!(batched[1].prs.is_empty() && batched[1].blocked_by.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_first_deliveries_of_the_same_pull_request_do_not_error() {
    use reeve_protocol::PrStatus;
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool.clone());
    let c = ctx("dan@example.com", false);
    let story = h
        .create_story(
            &c,
            "s1",
            "Match",
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
    let gh = ctx_via("github", Via::System);
    // Several PR numbers so the race window is exercised more than once.
    let numbers: Vec<i64> = (1..=8).collect();
    for &n in &numbers {
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let deliver = |h: Handlers, b: Arc<tokio::sync::Barrier>| {
            let gh = gh.clone();
            let key = story.key().clone();
            tokio::spawn(async move {
                b.wait().await;
                h.record_pull_request(&gh, &key, pull_request(n, PrStatus::Open, gh.now, "fix"))
                    .await
            })
        };
        let a = deliver(h.clone(), barrier.clone());
        let b = deliver(h.clone(), barrier);
        let (a, b) = (a.await.unwrap(), b.await.unwrap());
        let mut got = vec![
            a.expect("first delivery must not error"),
            b.expect("second delivery must not error"),
        ];
        got.sort_by_key(|c| format!("{c:?}"));
        assert_eq!(got, vec![PrChange::Ignored, PrChange::Linked], "PR {n}");
    }
    let dto = h.story_dto(story.key(), Utc::now()).await.unwrap();
    assert_eq!(dto.prs.len(), numbers.len(), "one row per pull request");
    assert_eq!(
        count(&sink, "pr_linked", &story.key().to_string()),
        numbers.len()
    );
    assert_eq!(count(&sink, "pr_updated", &story.key().to_string()), 0);
}

#[tokio::test]
async fn a_redelivery_with_sub_microsecond_digits_is_still_a_repeat() {
    use reeve_protocol::PrStatus;
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool.clone());
    let c = ctx("dan@example.com", false);
    let story = h
        .create_story(
            &c,
            "s1",
            "Match",
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
    let gh = ctx_via("github", Via::System);
    // PostgreSQL keeps microseconds; without truncation the stored row would read as older than the redelivery.
    let at = Utc.timestamp_opt(1_790_000_000, 123_456_789).unwrap();
    let first = h
        .record_pull_request(&gh, story.key(), pull_request(7, PrStatus::Open, at, "fix"))
        .await
        .unwrap();
    let again = h
        .record_pull_request(&gh, story.key(), pull_request(7, PrStatus::Open, at, "fix"))
        .await
        .unwrap();
    assert_eq!((first, again), (PrChange::Linked, PrChange::Ignored));
    assert_eq!(count(&sink, "pr_updated", &story.key().to_string()), 0);
}

#[tokio::test]
async fn find_story_by_external_ref_finds_only_stories() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let c = ctx("dan@example.com", false);
    let story = h
        .create_story(
            &c,
            "s1",
            "Legacy",
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
    let ini = h
        .create_initiative(&c, "i1", "Orion", "", "Now", None, None, None)
        .await
        .unwrap();
    for (table, key, r) in [
        ("stories", story.key().to_string(), "SCRUM-11"),
        ("initiatives", ini.key().to_string(), "JPD-5"),
    ] {
        sqlx::query(&format!(
            "UPDATE {table} SET external_ref = $2 WHERE key = $1"
        ))
        .bind(key)
        .bind(r)
        .execute(&pool)
        .await
        .unwrap();
    }
    assert_eq!(
        h.find_story_by_external_ref("SCRUM-11").await.unwrap(),
        Some(story.key().clone())
    );
    assert_eq!(h.find_story_by_external_ref("JPD-5").await.unwrap(), None);
    assert_eq!(
        h.find_story_by_external_ref("SCRUM-12").await.unwrap(),
        None
    );
}

/// Keys first, then the external reference of that item type: distinguishes a lookup across every table, a resolver
/// that looks up references before keys, and one that demands `Key` grammar of references.
#[tokio::test]
async fn resolve_ref_prefers_keys_and_scopes_external_refs_by_type() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let c = ctx("dan@example.com", false);
    let a = h
        .create_story(
            &c,
            "a",
            "A",
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
    let b = h
        .create_story(
            &c,
            "b",
            "B",
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
    let e = h.create_epic(&c, "e1", "E", None, "", None).await.unwrap();
    for (sql, r, k) in [
        (
            "UPDATE stories SET external_ref = $1 WHERE key = $2",
            "SCRUM-11".to_string(),
            a.key().to_string(),
        ),
        (
            "UPDATE stories SET external_ref = $1 WHERE key = $2",
            a.key().to_string(),
            b.key().to_string(),
        ),
        (
            "UPDATE epics SET external_ref = $1 WHERE key = $2",
            "S2-4".to_string(),
            e.key().to_string(),
        ),
    ] {
        sqlx::query(sql)
            .bind(r)
            .bind(k)
            .execute(&pool)
            .await
            .unwrap();
    }
    assert_eq!(
        &h.resolve_ref("SCRUM-11", ItemType::Story).await.unwrap(),
        a.key()
    );
    assert!(matches!(
        h.resolve_ref("SCRUM-11", ItemType::Epic).await.unwrap_err(),
        HandlerError::NotFound
    ));
    assert_eq!(
        &h.resolve_ref(&a.key().to_string(), ItemType::Story)
            .await
            .unwrap(),
        a.key()
    );
    assert_eq!(
        &h.resolve_ref("S2-4", ItemType::Epic).await.unwrap(),
        e.key()
    );
    assert!(matches!(
        h.resolve_ref("not-a-key", ItemType::Story)
            .await
            .unwrap_err(),
        HandlerError::NotFound
    ));
}

/// Items issued before a `key_prefix` change keep their keys: a key of an earlier prefix resolves natively (before any
/// external reference, even one equal to it) for show, edit, move and as a parent, and a key-shaped external reference
/// still resolves when no item of that type has it as its key.
#[tokio::test]
async fn resolve_ref_resolves_keys_of_an_earlier_prefix_before_external_refs() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let old = Handlers {
        ports: make_planning_ports(pool.clone()),
        activity: Arc::new(MemSink(Mutex::new(vec![]))),
        pool: pool.clone(),
        settings: Settings {
            key_prefix: "OLD".into(),
            ..settings()
        },
    };
    let c = ctx("dan@example.com", false);
    let old_epic = old
        .create_epic(&c, "oe", "Old epic", None, "", None)
        .await
        .unwrap();
    let old_story = old
        .create_story(
            &c,
            "os",
            "Old story",
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
    assert_eq!(old_story.key().prefix(), "OLD");
    let current = h
        .create_story(
            &c,
            "cs",
            "Current",
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
    // A current story whose external reference equals the old key, and another with a key-shaped Jira reference.
    for (r, k) in [
        (old_story.key().to_string(), current.key().to_string()),
        ("JPD-9".to_string(), current.key().to_string()),
    ] {
        sqlx::query("UPDATE stories SET external_ref = $1 WHERE key = $2")
            .bind(r)
            .bind(k)
            .execute(&pool)
            .await
            .unwrap();
    }
    let raw = old_story.key().to_string();

    // show
    let k = h.resolve_ref(&raw, ItemType::Story).await.unwrap();
    assert_eq!(&k, old_story.key());
    assert_eq!(h.get_story(&k).await.unwrap().title(), "Old story");
    // edit
    let edited = h
        .edit_story(
            &c,
            &h.resolve_ref(&raw, ItemType::Story).await.unwrap(),
            1,
            Some("Renamed".into()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(edited.key(), old_story.key());
    // move
    let moved = h
        .move_story(
            &c,
            &h.resolve_ref(&raw, ItemType::Story).await.unwrap(),
            2,
            DeliveryState::Done,
            Some(Resolution::Dropped),
            None,
        )
        .await
        .unwrap();
    assert_eq!(moved.key(), old_story.key());
    assert_eq!(moved.delivery(), DeliveryState::Done);
    // parent
    let parent = h
        .resolve_ref(&old_epic.key().to_string(), ItemType::Epic)
        .await
        .unwrap();
    assert_eq!(&parent, old_epic.key());
    let child = h
        .create_story(
            &c,
            "ch",
            "Child",
            Kind::Story,
            Some(parent),
            "",
            vec![],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(child.epic(), Some(old_epic.key()));

    // The old key names a story, not an epic: as an epic it is NotFound.
    assert!(matches!(
        h.resolve_ref(&raw, ItemType::Epic).await.unwrap_err(),
        HandlerError::NotFound
    ));
    // A key-shaped external reference no item has as its key still resolves.
    assert_eq!(
        &h.resolve_ref("JPD-9", ItemType::Story).await.unwrap(),
        current.key()
    );
}

/// An untyped reference (a decision target) checks native keys of every item type before any external reference: an
/// epic issued under an earlier prefix (`OLD-1`) wins over a story whose `external_ref` is `OLD-1`, so a decision on
/// `OLD-1` targets the epic. Distinguishes a resolver that tries each type's key-then-reference in turn (story first).
#[tokio::test]
async fn resolve_any_checks_native_keys_of_every_type_before_external_refs() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let old = Handlers {
        ports: make_planning_ports(pool.clone()),
        activity: Arc::new(MemSink(Mutex::new(vec![]))),
        pool: pool.clone(),
        settings: Settings {
            key_prefix: "OLD".into(),
            ..settings()
        },
    };
    let c = ctx("dan@example.com", false);
    let old_epic = old
        .create_epic(&c, "oe", "Old epic", None, "", None)
        .await
        .unwrap();
    let raw = old_epic.key().to_string();
    let imported = h
        .create_story(
            &c,
            "is",
            "Imported",
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
    sqlx::query("UPDATE stories SET external_ref = $1 WHERE key = $2")
        .bind(&raw)
        .bind(imported.key().to_string())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE epics SET external_ref = 'JIRA-7' WHERE key = $1")
        .bind(&raw)
        .execute(&pool)
        .await
        .unwrap();

    let (item, k) = h.resolve_any(&raw).await.unwrap();
    assert_eq!((item, &k), (ItemType::Epic, old_epic.key()));
    let d = h
        .require_decision(&c, "d", &k, "Pick", "dan@example.com", None, None)
        .await
        .unwrap();
    assert_eq!(d.on(), old_epic.key());
    let listed = h.list_decisions(Some(&k), false, true).await.unwrap();
    assert_eq!(listed.len(), 1);

    // A current-prefix key of each type resolves to its type; an external reference resolves when no key matches.
    assert_eq!(
        h.resolve_any(&imported.key().to_string()).await.unwrap(),
        (ItemType::Story, imported.key().clone())
    );
    assert_eq!(
        h.resolve_any("JIRA-7").await.unwrap(),
        (ItemType::Epic, old_epic.key().clone())
    );
    // An unknown current-prefix key and a non-reference are NotFound.
    for raw in ["ABC-999", "not a ref"] {
        assert!(matches!(
            h.resolve_any(raw).await.unwrap_err(),
            HandlerError::NotFound
        ));
    }
}

/// The planning skills reclassify a story: `kind` on an edit changes it and leaves everything else alone.
#[tokio::test]
async fn edit_story_changes_the_kind() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let c = ctx("dan@example.com", false);
    let s = h
        .create_story(
            &c,
            "k",
            "Kinded",
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
    let e = h
        .edit_story(
            &c,
            s.key(),
            s.version(),
            None,
            None,
            Some(Kind::Bug),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(e.kind(), Kind::Bug);
    assert_eq!(e.title(), "Kinded");
    assert_eq!(h.get_story(s.key()).await.unwrap().kind(), Kind::Bug);
}

/// A blank imported owner is no owner, so kb/ADR-0027 R6's "no owner" holds for imported stories.
#[tokio::test]
async fn import_stores_a_blank_owner_as_none() {
    use reeve_protocol::{ImportEpic, ImportInitiative, ImportStory};
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let c = ctx("dan@example.com", false);
    let ini = h
        .import_initiative(
            &c,
            ImportInitiative {
                external_ref: "JPD-5".into(),
                title: "Booking".into(),
                brief: None,
                horizon: "Now".into(),
                target_month: None,
                owner: None,
                discovery_complete: true,
            },
        )
        .await
        .unwrap();
    let epic = h
        .import_epic(
            &c,
            ImportEpic {
                external_ref: "SCRUM-10".into(),
                title: "v2".into(),
                initiative: ini.key.clone(),
                brief: None,
                discovery_complete: true,
            },
        )
        .await
        .unwrap();
    let story = h
        .import_story(
            &c,
            ImportStory {
                external_ref: "SCRUM-11".into(),
                title: "Match".into(),
                kind: Kind::Story,
                epic: epic.key.clone(),
                description: None,
                criteria: vec![],
                owner: Some("  ".into()),
                labels: vec![],
                delivery: DeliveryState::InProgress,
                resolution: None,
            },
        )
        .await
        .unwrap();
    let key = story.key.unwrap();
    assert_eq!(h.get_story(&key).await.unwrap().owner(), None);
    assert_eq!(h.story_dto(&key, Utc::now()).await.unwrap().owner, None);
}

/// Grandfathering is create-with-state, never an override; the import is create-only and re-runnable.
#[tokio::test]
async fn import_creates_with_state_once_and_never_logs_an_override() {
    use chrono::Datelike;
    use reeve_protocol::{CriterionDto, ImportComment, ImportEpic, ImportInitiative, ImportStory};
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool.clone());
    let c = ctx("dan@example.com", false);
    let same = || ImportInitiative {
        external_ref: "JPD-5".into(),
        title: "Booking".into(),
        brief: Some("# Brief".into()),
        horizon: "Now".into(),
        target_month: Some("2026-11".into()),
        owner: Some("Dan@Example.com".into()),
        discovery_complete: true,
    };
    let ini = h.import_initiative(&c, same()).await.unwrap();
    assert!(ini.created);
    let again = h.import_initiative(&c, same()).await.unwrap();
    assert!(!again.created);
    assert_eq!(again.key, ini.key);
    assert_eq!(
        count(&sink, "imported", &ini.key.as_ref().unwrap().to_string()),
        1
    );
    let epic = h
        .import_epic(
            &c,
            ImportEpic {
                external_ref: "SCRUM-10".into(),
                title: "v2".into(),
                initiative: ini.key.clone(),
                brief: None,
                discovery_complete: true,
            },
        )
        .await
        .unwrap();
    assert!(epic.created);
    let story = h
        .import_story(
            &c,
            ImportStory {
                external_ref: "SCRUM-11".into(),
                title: "Match".into(),
                kind: Kind::Story,
                epic: epic.key.clone(),
                description: None,
                criteria: vec![],
                owner: Some("bob@example.com".into()),
                labels: vec!["legacy-import".into()],
                delivery: DeliveryState::InProgress,
                resolution: None,
            },
        )
        .await
        .unwrap();
    let sk = story.key.clone().unwrap();
    let s = h.get_story(&sk).await.unwrap();
    let dto = h.story_dto(s.key(), Utc::now()).await.unwrap();
    assert_eq!(dto.delivery, DeliveryState::InProgress); // created in state: no gate, no override
    assert_eq!(dto.external_ref.as_deref(), Some("SCRUM-11"));
    assert_eq!(dto.owner.as_deref(), Some("bob@example.com"));
    assert_eq!(dto.labels, vec!["legacy-import".to_string()]);
    let ini_dto = h
        .initiative_dto(ini.key.as_ref().unwrap(), Utc::now())
        .await
        .unwrap();
    assert_eq!(ini_dto.owner.as_deref(), Some("dan@example.com"));
    assert_eq!(ini_dto.external_ref.as_deref(), Some("JPD-5"));
    assert_eq!(ini_dto.discovery_complete.as_ref().unwrap().by, c.actor);
    assert_eq!(ini_dto.discovery_complete.as_ref().unwrap().at, c.now);
    let e = h
        .epic_dto(epic.key.as_ref().unwrap(), Utc::now())
        .await
        .unwrap();
    assert_eq!(e.discovery_complete.unwrap().by, c.actor);
    assert_eq!(e.external_ref.as_deref(), Some("SCRUM-10"));
    assert!(!h.readiness(&sk).await.unwrap().ready); // still computed: no criteria
    assert_eq!(count(&sink, "imported", &sk.to_string()), 1);
    assert_eq!(
        sink.0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.action == "override")
            .count(),
        0
    );
    assert!(
        sink.0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.action == "imported")
            .all(|r| r.payload["external_ref"].is_string())
    );

    // a done story keeps its resolution and its ticked criteria
    let done = h
        .import_story(
            &c,
            ImportStory {
                external_ref: "SCRUM-12".into(),
                title: "Shipped".into(),
                kind: Kind::Bug,
                epic: None,
                description: Some("old".into()),
                criteria: vec![CriterionDto {
                    text: "works".into(),
                    done: true,
                }],
                owner: None,
                labels: vec![],
                delivery: DeliveryState::Done,
                resolution: Some(Resolution::Completed),
            },
        )
        .await
        .unwrap();
    let d = h.get_story(done.key.as_ref().unwrap()).await.unwrap();
    assert_eq!(d.delivery(), DeliveryState::Done);
    assert_eq!(d.resolution(), Some(Resolution::Completed));
    assert!(d.criteria()[0].done);

    // comments keep the Jira author and date and are keyed by external ref
    let same_comment = || ImportComment {
        external_ref: "jira-comment:900".into(),
        story: sk.clone(),
        author: "Alice (alice@example.com)".into(),
        body: "looks good".into(),
        at: Utc.with_ymd_and_hms(2025, 3, 1, 10, 0, 0).unwrap(),
    };
    let cm = h.import_comment(&c, same_comment()).await.unwrap();
    assert!(cm.created);
    assert!(cm.id.is_some());
    let cm2 = h.import_comment(&c, same_comment()).await.unwrap();
    assert!(!cm2.created);
    assert_eq!(cm2.id, cm.id);
    let comments = h.comments_for_story(&sk).await.unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].author, "Alice (alice@example.com)");
    assert_eq!(comments[0].at.year(), 2025);
    let imported_entries = sink
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.action == "imported")
        .count();
    assert_eq!(imported_entries, 5); // initiative, epic, two stories, one comment: no replay logs anything

    // validation
    let story_with = |delivery, resolution: Option<Resolution>, r: &str, epic| ImportStory {
        external_ref: r.into(),
        title: "X".into(),
        kind: Kind::Story,
        epic,
        description: None,
        criteria: vec![],
        owner: None,
        labels: vec![],
        delivery,
        resolution,
    };
    assert!(matches!(
        h.import_story(&c, story_with(DeliveryState::Done, None, "V-1", None))
            .await,
        Err(HandlerError::Validation(_))
    ));
    assert!(matches!(
        h.import_story(
            &c,
            story_with(
                DeliveryState::Testing,
                Some(Resolution::Dropped),
                "V-2",
                None
            )
        )
        .await,
        Err(HandlerError::Validation(_))
    ));
    assert!(matches!(
        h.import_story(&c, story_with(DeliveryState::Todo, None, "  ", None))
            .await,
        Err(HandlerError::Validation(_))
    ));
    assert!(matches!(
        h.import_story(
            &c,
            story_with(DeliveryState::Todo, None, &"R".repeat(201), None)
        )
        .await,
        Err(HandlerError::Validation(_))
    ));
    let mut long = story_with(DeliveryState::Todo, None, "V-3", None);
    long.title = "t".repeat(201);
    assert!(matches!(
        h.import_story(&c, long).await,
        Err(HandlerError::Validation(_))
    ));
    assert!(matches!(
        h.import_story(
            &c,
            story_with(
                DeliveryState::Todo,
                None,
                "V-4",
                Some("ABC-999".parse().unwrap())
            )
        )
        .await,
        Err(HandlerError::NotFound)
    ));
    let mut bad_horizon = same();
    bad_horizon.external_ref = "JPD-6".into();
    bad_horizon.horizon = "Someday".into();
    assert!(matches!(
        h.import_initiative(&c, bad_horizon).await,
        Err(HandlerError::Validation(_))
    ));
    assert!(matches!(
        h.import_epic(
            &c,
            ImportEpic {
                external_ref: "SCRUM-20".into(),
                title: "orphan".into(),
                initiative: Some("ABC-999".parse().unwrap()),
                brief: None,
                discovery_complete: false,
            }
        )
        .await,
        Err(HandlerError::NotFound)
    ));
    let mut no_story = same_comment();
    no_story.external_ref = "jira-comment:901".into();
    no_story.story = "ABC-999".parse().unwrap();
    assert!(matches!(
        h.import_comment(&c, no_story).await,
        Err(HandlerError::NotFound)
    ));
    // nothing from a refused import was written
    assert_eq!(h.find_story_by_external_ref("V-4").await.unwrap(), None);
}

/// Item references follow the `KeyRef` grammar outside the instance's key space, comments need
/// a body and an author, and every path to an existing reference answers `created: false` with one winner.
#[tokio::test]
async fn import_validates_references_comment_bodies_and_resolves_races() {
    use reeve_protocol::{ImportComment, ImportStory};
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool.clone());
    let c = ctx("dan@example.com", false);
    let story = |r: &str| ImportStory {
        external_ref: r.into(),
        title: "X".into(),
        kind: Kind::Story,
        epic: None,
        description: None,
        criteria: vec![],
        owner: None,
        labels: vec![],
        delivery: DeliveryState::Todo,
        resolution: None,
    };
    for bad in ["jpd 77", "jpd-77", "JPD77", "ABC-3"] {
        match h.import_story(&c, story(bad)).await {
            Err(HandlerError::Validation(m)) => assert!(m.contains(bad), "{m}"),
            other => panic!("{bad}: {other:?}"),
        }
    }
    let ok = h.import_story(&c, story("JPD-77")).await.unwrap();
    assert!(ok.created);

    // comments: blank body or author refused; both are stored trimmed
    let sk = ok.key.clone().unwrap();
    let comment = |r: &str, author: &str, body: &str| ImportComment {
        external_ref: r.into(),
        story: sk.clone(),
        author: author.into(),
        body: body.into(),
        at: Utc.with_ymd_and_hms(2025, 3, 1, 10, 0, 0).unwrap(),
    };
    assert!(matches!(
        h.import_comment(&c, comment("jira-comment:1", "Alice", "  "))
            .await,
        Err(HandlerError::Validation(_))
    ));
    assert!(matches!(
        h.import_comment(&c, comment("jira-comment:2", " ", "hi"))
            .await,
        Err(HandlerError::Validation(_))
    ));
    h.import_comment(&c, comment("jira-comment:3", " Alice ", " hi "))
        .await
        .unwrap();
    let comments = h.comments_for_story(&sk).await.unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].author, "Alice");
    assert_eq!(comments[0].body, "hi");

    // the reference is already the idempotency key of an item that was not imported
    h.create_story(
        &c,
        "JPD-5",
        "Native",
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
    match h.import_story(&c, story("JPD-5")).await {
        Err(HandlerError::Validation(m)) => assert!(m.contains("JPD-5"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(h.find_story_by_external_ref("JPD-5").await.unwrap(), None);

    // a caller's comment idempotency key that looks like a Jira reference never stands in for the imported comment
    let native = h
        .comment_story(&c, &sk, "native", Some("jira-comment:900"))
        .await
        .unwrap();
    let imported = h
        .import_comment(&c, comment("jira-comment:900", "Alice", "from jira"))
        .await
        .unwrap();
    assert!(imported.created, "{imported:?}");
    assert_ne!(imported.id, Some(native));
    let again = h
        .import_comment(&c, comment("jira-comment:900", "Alice", "from jira"))
        .await
        .unwrap();
    assert!(!again.created);
    assert_eq!(again.id, imported.id);
    // and the caller's key still replays the caller's comment
    assert_eq!(
        h.comment_story(&c, &sk, "native", Some("jira-comment:900"))
            .await
            .unwrap(),
        native
    );
    let mut bodies: Vec<_> = h
        .comments_for_story(&sk)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.body)
        .collect();
    bodies.sort();
    assert_eq!(bodies, ["from jira", "hi", "native"]);

    // concurrent imports of one reference: one creates, every other answers with the winner's key
    let tasks: Vec<_> = (0..8)
        .map(|_| {
            let (h, c, s) = (h.clone(), c.clone(), story("JPD-50"));
            tokio::spawn(async move { h.import_story(&c, s).await })
        })
        .collect();
    let mut results = Vec::new();
    for t in tasks {
        results.push(t.await.unwrap().unwrap());
    }
    assert_eq!(results.iter().filter(|r| r.created).count(), 1);
    let winner = h.find_story_by_external_ref("JPD-50").await.unwrap();
    assert!(winner.is_some());
    assert!(results.iter().all(|r| r.key == winner));
    assert_eq!(count(&sink, "imported", &winner.unwrap().to_string()), 1);
}

/// An import that loses the insert to a concurrent writer of the same reference answers `created: false` with the
/// winner's key, whether it lost on the `idempotency_key` index (DO NOTHING) or on the `external_ref` index (a unique
/// violation). The winner is a row held uncommitted in another transaction, so the import's lookup misses it and its
/// insert waits on the index until the winner commits.
#[tokio::test]
async fn import_that_loses_the_insert_race_answers_with_the_winner() {
    use reeve_protocol::ImportStory;
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool.clone());
    let c = ctx("dan@example.com", false);
    let story = |r: &str| ImportStory {
        external_ref: r.into(),
        title: "X".into(),
        kind: Kind::Story,
        epic: None,
        description: None,
        criteria: vec![],
        owner: None,
        labels: vec![],
        delivery: DeliveryState::Todo,
        resolution: None,
    };
    // (winner key, its external_ref, its idempotency key): the same idempotency key, then only the same external_ref
    for (winner, r, idem) in [
        ("ABC-900", "JPD-60", "JPD-60"),
        ("ABC-901", "JPD-61", "someone-else"),
    ] {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("INSERT INTO stories (key, title, kind, external_ref, idempotency_key, created_at, updated_at) VALUES ($1, 'W', 'story', $2, $3, now(), now())")
            .bind(winner)
            .bind(r)
            .bind(idem)
            .execute(&mut *tx)
            .await
            .unwrap();
        let (h2, c2, s) = (h.clone(), c.clone(), story(r));
        let import = tokio::spawn(async move { h2.import_story(&c2, s).await });
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        tx.commit().await.unwrap();
        let got = import.await.unwrap().unwrap();
        assert!(!got.created, "{r}");
        assert_eq!(got.key, Some(winner.parse().unwrap()), "{r}");
    }
    assert_eq!(
        sink.0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.action == "imported")
            .count(),
        0
    );
}

/// Codex pass 5: a reference names one Jira issue, so it names at most one item across the three types. The same
/// reference imported again under another type is refused, naming the item it already is, and creates nothing.
#[tokio::test]
async fn import_refuses_a_reference_already_imported_under_another_type() {
    use reeve_protocol::{ImportEpic, ImportInitiative, ImportStory};
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool.clone());
    let c = ctx("dan@example.com", false);
    let s = h
        .import_story(
            &c,
            ImportStory {
                external_ref: "SCRUM-30".into(),
                title: "X".into(),
                kind: Kind::Story,
                epic: None,
                description: None,
                criteria: vec![],
                owner: None,
                labels: vec![],
                delivery: DeliveryState::Todo,
                resolution: None,
            },
        )
        .await
        .unwrap();
    let sk = s.key.unwrap();
    let want = format!("SCRUM-30 is already imported as story {sk}");
    match h
        .import_epic(
            &c,
            ImportEpic {
                external_ref: "SCRUM-30".into(),
                title: "E".into(),
                initiative: None,
                brief: None,
                discovery_complete: false,
            },
        )
        .await
    {
        Err(HandlerError::Validation(m)) => assert_eq!(m, want),
        other => panic!("{other:?}"),
    }
    match h
        .import_initiative(
            &c,
            ImportInitiative {
                external_ref: "SCRUM-30".into(),
                title: "I".into(),
                brief: None,
                horizon: "Now".into(),
                target_month: None,
                owner: None,
                discovery_complete: false,
            },
        )
        .await
    {
        Err(HandlerError::Validation(m)) => assert_eq!(m, want),
        other => panic!("{other:?}"),
    }
    let others: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM epics WHERE external_ref = 'SCRUM-30') + (SELECT count(*) FROM initiatives WHERE external_ref = 'SCRUM-30')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(others, 0);
    assert_eq!(
        sink.0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.action == "imported")
            .count(),
        1
    );
}

/// Codex pass 6: no unique index spans the three item tables, so two imports of one reference under different types
/// racing past each other's lookup would both create an item. The import takes a transaction-scoped lock on the
/// reference before its lookup: an import in flight (here a transaction holding that lock with an uncommitted epic)
/// makes a story import of the same reference wait, and once it commits the story import is refused, naming the epic.
/// Then real concurrent story and epic imports of one reference: exactly one item, the other call names it.
#[tokio::test]
async fn concurrent_imports_of_one_reference_under_two_types_claim_it_once() {
    use reeve_protocol::{ImportEpic, ImportStory};
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool.clone());
    let c = ctx("dan@example.com", false);
    let story = |r: &str| ImportStory {
        external_ref: r.into(),
        title: "X".into(),
        kind: Kind::Story,
        epic: None,
        description: None,
        criteria: vec![],
        owner: None,
        labels: vec![],
        delivery: DeliveryState::Todo,
        resolution: None,
    };
    let epic = |r: &str| ImportEpic {
        external_ref: r.into(),
        title: "E".into(),
        initiative: None,
        brief: None,
        discovery_complete: false,
    };
    let items = |r: String| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT (SELECT count(*) FROM stories WHERE external_ref = $1) + (SELECT count(*) FROM epics WHERE external_ref = $1) + (SELECT count(*) FROM initiatives WHERE external_ref = $1)",
            )
            .bind(r)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };

    // an epic import of SCRUM-30 in flight: the lock held, its row inserted but not committed
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('reeve_import:' || $1))")
        .bind("SCRUM-30")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO epics (key, title, external_ref, idempotency_key, created_at, updated_at) VALUES ('ABC-950', 'W', 'SCRUM-30', 'SCRUM-30', now(), now())")
        .execute(&mut *tx)
        .await
        .unwrap();
    let (h2, c2, s) = (h.clone(), c.clone(), story("SCRUM-30"));
    let import = tokio::spawn(async move { h2.import_story(&c2, s).await });
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(
        !import.is_finished(),
        "the story import waits on the reference lock"
    );
    tx.commit().await.unwrap();
    match import.await.unwrap() {
        Err(HandlerError::Validation(m)) => {
            assert_eq!(m, "SCRUM-30 is already imported as epic ABC-950")
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(items("SCRUM-30".into()).await, 1);

    // real concurrent imports: a story and an epic of one reference, several rounds
    for n in 31..39 {
        let r = format!("SCRUM-{n}");
        let (hs, cs, s) = (h.clone(), c.clone(), story(&r));
        let (he, ce, e) = (h.clone(), c.clone(), epic(&r));
        let st = tokio::spawn(async move { hs.import_story(&cs, s).await });
        let ep = tokio::spawn(async move { he.import_epic(&ce, e).await });
        let (st, ep) = (st.await.unwrap(), ep.await.unwrap());
        let (won, winner, loser_says) = match (st, ep) {
            (Ok(w), Err(HandlerError::Validation(m))) => (w, "story", m),
            (Err(HandlerError::Validation(m)), Ok(w)) => (w, "epic", m),
            other => panic!("{r}: {other:?}"),
        };
        assert!(won.created, "{r}");
        let key = won.key.unwrap();
        assert_eq!(
            loser_says,
            format!("{r} is already imported as {winner} {key}")
        );
        assert_eq!(items(r.clone()).await, 1, "{r}");
        assert_eq!(count(&sink, "imported", &key.to_string()), 1, "{r}");
    }
}

#[tokio::test]
async fn readiness_of_a_missing_story_is_not_found() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let err = h.readiness(&"ABC-999".parse().unwrap()).await.unwrap_err();
    assert!(matches!(err, HandlerError::NotFound), "{err:?}");
}

/// Seeds `n` stories under a new initiative and epic for the statement-count tests: a decision on the epic and on the
/// first story, every story after the first blocked by its predecessor, and one pull request per story. `tag` keeps the
/// idempotency keys of two seedings apart.
async fn seed_board(h: &Handlers, tag: &str, n: usize, first_pr: i64) -> Vec<reeve_protocol::Key> {
    use reeve_protocol::PrStatus;
    let c = ctx("dan@example.com", false);
    let ini = h
        .create_initiative(&c, &format!("{tag}-i"), "I", "", "Now", None, None, None)
        .await
        .unwrap();
    let epic = h
        .create_epic(
            &c,
            &format!("{tag}-e"),
            "E",
            Some(ini.key().clone()),
            "",
            None,
        )
        .await
        .unwrap();
    h.require_decision(
        &c,
        &format!("{tag}-de"),
        epic.key(),
        "Which?",
        "dan@example.com",
        None,
        None,
    )
    .await
    .unwrap();
    let gh = Ctx {
        actor: "github".into(),
        via: Via::System,
        now: c.now,
    };
    let mut keys: Vec<reeve_protocol::Key> = Vec::new();
    for i in 0..n {
        let s = h
            .create_story(
                &c,
                &format!("{tag}-s{i}"),
                "S",
                Kind::Story,
                Some(epic.key().clone()),
                "",
                vec!["works".into()],
                vec![],
                None,
                None,
            )
            .await
            .unwrap();
        if let Some(prev) = keys.last() {
            h.add_block(&c, s.key(), prev).await.unwrap();
        }
        h.record_pull_request(
            &gh,
            s.key(),
            pull_request(first_pr + i as i64, PrStatus::Open, c.now, "x"),
        )
        .await
        .unwrap();
        keys.push(s.key().clone());
    }
    h.require_decision(
        &c,
        &format!("{tag}-ds"),
        &keys[0],
        "How?",
        "dan@example.com",
        None,
        None,
    )
    .await
    .unwrap();
    keys
}

/// Marks `story` urgent as of the wall clock, so the mark is active for the statement-count reads (which read at
/// `Utc::now()`, not at the fixed test clock).
async fn mark_now(h: &Handlers, story: &Key) {
    let c = Ctx {
        now: Utc::now(),
        ..ctx("dan@example.com", false)
    };
    h.mark_urgent(&c, ItemType::Story, story, None)
        .await
        .unwrap();
}

/// The list reads in a fixed number of statements: twelve stories cost what three cost (one transaction and up to
/// seven statements per story before).
#[tokio::test]
async fn list_stories_costs_the_same_for_three_and_twelve_stories() {
    let (_c, pool) = common::pg_counted().await;
    let (h, _sink) = mk(pool.clone());
    let keys = seed_board(&h, "a", 3, 1).await;
    // An active mark: urgency costs its fixed statements (marks, then lineage) in both measurements.
    mark_now(&h, &keys[1]).await;
    let list = || {
        let h = h.clone();
        async move {
            h.list_stories(
                &StoryFilter::default(),
                None,
                None,
                false,
                DoneFilter::All,
                Utc::now(),
            )
            .await
            .unwrap();
        }
    };
    list().await; // warm up: prepared statements, catalog caches
    let three = common::statements(&pool, list).await;
    seed_board(&h, "b", 9, 100).await;
    assert_eq!(
        h.list_stories(
            &StoryFilter::default(),
            None,
            None,
            false,
            DoneFilter::All,
            Utc::now()
        )
        .await
        .unwrap()
        .len(),
        12
    );
    let twelve = common::statements(&pool, list).await;
    assert_eq!(three, twelve);
    let board = h
        .list_stories(
            &StoryFilter::default(),
            None,
            None,
            false,
            DoneFilter::All,
            Utc::now(),
        )
        .await
        .unwrap();
    let marked = board.iter().find(|r| r.story.key == keys[1]).unwrap();
    assert_eq!(marked.story.urgent_sources.len(), 1, "the mark is active");
}

/// The context reads its blockers in a fixed number of statements: five blockers cost what one costs, and they come
/// in key-number order.
#[tokio::test]
async fn story_context_costs_the_same_for_one_and_five_blockers() {
    let (_c, pool) = common::pg_counted().await;
    let (h, _sink) = mk(pool.clone());
    let c = ctx("dan@example.com", false);
    let keys = seed_board(&h, "a", 1, 1).await;
    let x = keys[0].clone();
    mark_now(&h, &x).await;
    h.comment_story(&c, &x, "hello", None).await.unwrap();
    let mut blockers = Vec::new();
    for i in 0..5 {
        blockers.push(
            h.create_story(
                &c,
                &format!("b{i}"),
                "B",
                Kind::Chore,
                None,
                "",
                vec![],
                vec![],
                None,
                None,
            )
            .await
            .unwrap()
            .key()
            .clone(),
        );
    }
    h.add_block(&c, &x, &blockers[0]).await.unwrap();
    let context = || {
        let h = h.clone();
        let x = x.clone();
        async move {
            h.story_context(&x, Utc::now()).await.unwrap();
        }
    };
    context().await; // warm up
    let one = common::statements(&pool, context).await;
    for b in &blockers[1..] {
        h.add_block(&c, &x, b).await.unwrap();
    }
    let five = common::statements(&pool, context).await;
    assert_eq!(one, five);
    let ctx_dto = h.story_context(&x, Utc::now()).await.unwrap();
    assert_eq!(
        ctx_dto
            .blockers
            .iter()
            .map(|b| b.key.clone())
            .collect::<Vec<_>>(),
        blockers
    );
    assert_eq!(ctx_dto.story.blocked_by, blockers);
    assert_eq!(ctx_dto.story.urgent_sources.len(), 1, "the mark is active");
    assert_eq!(ctx_dto.comments.len(), 1);
    assert_eq!(ctx_dto.required_decisions.len(), 2);
}

/// A regression guard on the zip: each listed story carries its own readiness, and a blocker that is done drops the
/// reason but keeps the edge.
#[tokio::test]
async fn list_pairs_each_story_with_its_own_readiness() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let c = ctx("dan@example.com", false);
    let epic = h.create_epic(&c, "e", "E", None, "", None).await.unwrap();
    h.mark_discovery(&c, ItemType::Epic, epic.key(), 1, true)
        .await
        .unwrap();
    let mk_story = |idem: &'static str| {
        let h = h.clone();
        let c = c.clone();
        let e = epic.key().clone();
        async move {
            h.create_story(
                &c,
                idem,
                idem,
                Kind::Story,
                Some(e),
                "",
                vec!["ok".into()],
                vec![],
                None,
                None,
            )
            .await
            .unwrap()
            .key()
            .clone()
        }
    };
    let a = mk_story("a").await;
    let b = mk_story("b").await;
    let s = mk_story("s").await;
    h.add_block(&c, &s, &b).await.unwrap();
    let find = |list: &[reeve_protocol::StoryWithReadiness], k: &reeve_protocol::Key| {
        list.iter().find(|x| &x.story.key == k).unwrap().clone()
    };
    let list = h
        .list_stories(
            &StoryFilter::default(),
            None,
            None,
            false,
            DoneFilter::All,
            Utc::now(),
        )
        .await
        .unwrap();
    assert_ne!(
        list[0].story.key, s,
        "S is not first, so a zip off by one shows"
    );
    let s_row = find(&list, &s);
    assert_eq!(
        s_row.readiness.reasons,
        vec![Reason::BlockedBy {
            story: b.clone(),
            delivery: DeliveryState::Todo,
            owner: None,
        }]
    );
    assert_eq!(s_row.story.blocked_by, vec![b.clone()]);
    assert!(find(&list, &a).readiness.ready);
    assert!(find(&list, &b).readiness.ready);

    h.move_story(
        &c,
        &b,
        1,
        DeliveryState::Done,
        Some(Resolution::Dropped),
        None,
    )
    .await
    .unwrap();
    let list = h
        .list_stories(
            &StoryFilter::default(),
            None,
            None,
            false,
            DoneFilter::All,
            Utc::now(),
        )
        .await
        .unwrap();
    let s_row = find(&list, &s);
    assert!(s_row.readiness.ready, "{:?}", s_row.readiness);
    assert_eq!(s_row.story.blocked_by, vec![b.clone()], "the edge stays");
}

/// The snapshot fixture: story S under an epic with discovery and a criterion, blocked by B (todo); story T (todo,
/// same epic, criterion). Returns (S, B, T).
async fn snapshot_fixture(
    h: &Handlers,
) -> (
    reeve_protocol::Key,
    reeve_protocol::Key,
    reeve_protocol::Key,
) {
    let c = ctx("dan@example.com", false);
    let epic = h.create_epic(&c, "e", "E", None, "", None).await.unwrap();
    h.mark_discovery(&c, ItemType::Epic, epic.key(), 1, true)
        .await
        .unwrap();
    let mut keys = Vec::new();
    for idem in ["s", "b", "t"] {
        keys.push(
            h.create_story(
                &c,
                idem,
                idem,
                Kind::Story,
                Some(epic.key().clone()),
                "",
                vec!["ok".into()],
                vec![],
                None,
                None,
            )
            .await
            .unwrap()
            .key()
            .clone(),
        );
    }
    h.add_block(&c, &keys[0], &keys[1]).await.unwrap();
    (keys[0].clone(), keys[1].clone(), keys[2].clone())
}

const PAUSE: std::time::Duration = std::time::Duration::from_secs(10);

/// Polls `pg_stat_activity` until exactly one backend of this database waits on a lock: the call, which took its
/// snapshot on its first `stories` read and now waits on the table lock the test took before spawning
/// it. A 10 s deadline fails the test.
async fn wait_until_blocked(pool: &sqlx::PgPool) {
    tokio::time::timeout(PAUSE, async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND datname = current_database()",
            )
            .fetch_one(pool)
            .await
            .unwrap();
            if waiting == 1 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the call never blocked on the table lock the test took");
}

/// Commits, while the call waits, changes to tables the lock does not cover: B done, a new edge T -> S, a pull request
/// on S, and (when given) the deletion of a comment. Never an INSERT into `comments`: its foreign key to
/// `required_decisions` would wait on the test's own lock.
async fn commit_during_the_pause(
    pool: &sqlx::PgPool,
    s: &reeve_protocol::Key,
    b: &reeve_protocol::Key,
    t: &reeve_protocol::Key,
    delete_comment: Option<i64>,
) {
    tokio::time::timeout(PAUSE, async {
        let mut w = pool.begin().await.unwrap();
        sqlx::query("UPDATE stories SET delivery = 'done', resolution = 'dropped' WHERE key = $1")
            .bind(b.to_string())
            .execute(&mut *w)
            .await
            .unwrap();
        sqlx::query("INSERT INTO story_blocks (story_key, blocker_key) VALUES ($1, $2)")
            .bind(t.to_string())
            .bind(s.to_string())
            .execute(&mut *w)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO story_prs (story_key, repo, number, url, title, status, updated_at) \
             VALUES ($1, 'acme/portal', 1, 'https://github.com/acme/portal/pull/1', 'x', 'open', now())",
        )
        .bind(s.to_string())
        .execute(&mut *w)
        .await
        .unwrap();
        if let Some(id) = delete_comment {
            sqlx::query("DELETE FROM comments WHERE id = $1")
                .bind(id)
                .execute(&mut *w)
                .await
                .unwrap();
        }
        w.commit().await.unwrap();
    })
    .await
    .expect("the writer waited: it touched a table the lock covers");
}

/// The list reads one snapshot: commits made while it waits (after its first statement) are invisible to every part of
/// the answer, readiness, blockers and pull requests alike. A read on the pool, a transaction per read, or READ
/// COMMITTED would each see them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_stories_reads_one_snapshot() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let (s, b, t) = snapshot_fixture(&h).await;

    let mut lock = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE required_decisions IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let call = tokio::spawn({
        let h = h.clone();
        async move {
            h.list_stories(
                &StoryFilter::default(),
                None,
                None,
                false,
                DoneFilter::All,
                Utc::now(),
            )
            .await
        }
    });
    wait_until_blocked(&pool).await;
    commit_during_the_pause(&pool, &s, &b, &t, None).await;
    lock.rollback().await.unwrap();
    let list = tokio::time::timeout(PAUSE, call)
        .await
        .expect("the list never finished")
        .unwrap()
        .unwrap();

    let row = |k: &reeve_protocol::Key| list.iter().find(|x| &x.story.key == k).unwrap();
    assert_eq!(
        row(&s).readiness.reasons,
        vec![Reason::BlockedBy {
            story: b.clone(),
            delivery: DeliveryState::Todo,
            owner: None,
        }]
    );
    assert!(row(&s).story.prs.is_empty());
    // B's row is read by the first statement, before the pause: a consistency check, not part of the proof. The
    // proof that B's move is invisible after the pause is S's `BlockedBy(B)` above (the readiness facts are read
    // after the pause).
    assert_eq!(row(&b).story.delivery, DeliveryState::Todo);
    assert!(row(&t).story.blocked_by.is_empty());
    assert!(row(&t).readiness.ready);

    let after = h
        .list_stories(
            &StoryFilter::default(),
            None,
            None,
            false,
            DoneFilter::All,
            Utc::now(),
        )
        .await
        .unwrap();
    let row = |k: &reeve_protocol::Key| after.iter().find(|x| &x.story.key == k).unwrap();
    assert!(row(&s).readiness.ready);
    assert_eq!(row(&s).story.prs.len(), 1);
    assert_eq!(row(&b).story.delivery, DeliveryState::Done);
    assert_eq!(row(&t).story.blocked_by, vec![s.clone()]);
    assert_eq!(
        row(&t).readiness.reasons,
        vec![Reason::BlockedBy {
            story: s.clone(),
            delivery: DeliveryState::Todo,
            owner: None,
        }]
    );
}

/// The context reads one snapshot: commits made while it waits are invisible to its readiness, blockers, pull requests,
/// comments and attachments alike.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn story_context_reads_one_snapshot() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let (s, b, t) = snapshot_fixture(&h).await;
    let comment = h
        .comment_story(&ctx("dan@example.com", false), &s, "C", None)
        .await
        .unwrap();
    // An attachment claimed through a second comment; the pause deletes it.
    let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255]));
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let dan = ctx("dan@example.com", false);
    let att = h
        .upload_attachment(&dan, "shot.png", None, bytes::Bytes::from(png.into_inner()))
        .await
        .unwrap()
        .id();
    let with_att = h
        .comment_story_attaching(&dan, &s, "see", None, &[att])
        .await
        .unwrap();

    let mut lock = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE required_decisions IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let call = tokio::spawn({
        let h = h.clone();
        let s = s.clone();
        async move { h.story_context(&s, Utc::now()).await }
    });
    wait_until_blocked(&pool).await;
    commit_during_the_pause(&pool, &s, &b, &t, Some(comment)).await;
    tokio::time::timeout(PAUSE, h.delete_attachment(&dan, att))
        .await
        .expect("the delete waited: it touched a table the lock covers")
        .unwrap();
    lock.rollback().await.unwrap();
    let got = tokio::time::timeout(PAUSE, call)
        .await
        .expect("the context never finished")
        .unwrap()
        .unwrap();

    assert_eq!(
        got.readiness.reasons,
        vec![Reason::BlockedBy {
            story: b.clone(),
            delivery: DeliveryState::Todo,
            owner: None,
        }]
    );
    assert!(got.story.prs.is_empty());
    assert_eq!(
        got.comments.iter().map(|c| c.id).collect::<Vec<_>>(),
        [comment, with_att]
    );
    assert_eq!(
        got.attachments.iter().map(|a| a.id).collect::<Vec<_>>(),
        [att]
    );
    assert_eq!(got.blockers.len(), 1);
    assert_eq!(got.blockers[0].key, b);
    assert_eq!(got.blockers[0].delivery, DeliveryState::Todo);

    let after = h.story_context(&s, Utc::now()).await.unwrap();
    assert!(after.readiness.ready);
    assert_eq!(after.story.prs.len(), 1);
    assert_eq!(
        after.comments.iter().map(|c| c.id).collect::<Vec<_>>(),
        [with_att]
    );
    assert!(after.attachments.is_empty());
    assert_eq!(after.blockers[0].delivery, DeliveryState::Done);
}

/// Every key-resolution path, pinned in one table on an instance with prefix `ABC`. Native keys come
/// first (an external reference never shadows one); a typed lookup never widens to another type; the branch-token
/// lookup reads external references only with `legacy`.
#[tokio::test]
async fn resolution_precedence_is_one_table() {
    use reeve_protocol::{ImportEpic, ImportInitiative, ImportStory, Key};
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let old = Handlers {
        ports: make_planning_ports(pool.clone()),
        activity: Arc::new(MemSink(Mutex::new(vec![]))),
        pool: pool.clone(),
        settings: Settings {
            key_prefix: "OLD".into(),
            ..settings()
        },
    };
    let c = ctx("dan@example.com", false);
    let story = |h: &Handlers, idem: &'static str| {
        let c = c.clone();
        let h = h.clone();
        async move {
            h.create_story(
                &c,
                idem,
                "S",
                Kind::Story,
                None,
                "",
                vec![],
                vec![],
                None,
                None,
            )
            .await
            .unwrap()
            .key()
            .clone()
        }
    };
    let gra1 = story(&h, "s1").await;
    let gra2 = h
        .create_epic(&c, "e1", "E", None, "", None)
        .await
        .unwrap()
        .key()
        .clone();
    let old1 = story(&old, "os").await;
    let old2 = old
        .create_epic(&c, "oe", "OE", None, "", None)
        .await
        .unwrap()
        .key()
        .clone();
    assert_eq!(
        [
            gra1.to_string(),
            gra2.to_string(),
            old1.to_string(),
            old2.to_string()
        ],
        ["ABC-1", "ABC-2", "OLD-1", "OLD-2"]
    );
    let gra_ini = h
        .create_initiative(&c, "i1", "I", "", "Now", None, None, None)
        .await
        .unwrap()
        .key()
        .clone();
    let imp_epic = |r: &str| ImportEpic {
        external_ref: r.into(),
        title: "IE".into(),
        initiative: None,
        brief: None,
        discovery_complete: true,
    };
    let imp_story = |r: &str| ImportStory {
        external_ref: r.into(),
        title: "IS".into(),
        kind: Kind::Story,
        epic: None,
        description: None,
        criteria: vec![],
        owner: None,
        labels: vec![],
        delivery: DeliveryState::Todo,
        resolution: None,
    };
    let scrum11 = h
        .import_story(&c, imp_story("SCRUM-11"))
        .await
        .unwrap()
        .key
        .unwrap();
    let jpd5 = h
        .import_epic(&c, imp_epic("JPD-5"))
        .await
        .unwrap()
        .key
        .unwrap();
    let from_old2 = h
        .import_story(&c, imp_story("OLD-2"))
        .await
        .unwrap()
        .key
        .unwrap();

    let jpd9 = h
        .import_initiative(
            &c,
            ImportInitiative {
                external_ref: "JPD-9".into(),
                title: "II".into(),
                brief: None,
                horizon: "Now".into(),
                target_month: None,
                owner: None,
                discovery_complete: true,
            },
        )
        .await
        .unwrap()
        .key
        .unwrap();
    // An imported story whose external reference equals a native key: it must never shadow it.
    let from_old1 = h
        .import_story(&c, imp_story("OLD-1"))
        .await
        .unwrap()
        .key
        .unwrap();
    assert_ne!(from_old1, old1);

    let k = |s: &str| s.parse::<Key>().unwrap();
    let rr = |raw: &str, t: ItemType| {
        let h = h.clone();
        let raw = raw.to_string();
        async move { h.resolve_ref(&raw, t).await }
    };
    for (raw, t, want) in [
        ("ABC-1", ItemType::Story, Some(gra1.clone())),
        ("ABC-999", ItemType::Story, Some(k("ABC-999"))),
        ("ABC-2", ItemType::Story, Some(gra2.clone())),
        ("OLD-1", ItemType::Story, Some(old1.clone())),
        ("OLD-2", ItemType::Story, Some(from_old2.clone())),
        ("OLD-2", ItemType::Epic, Some(old2.clone())),
        ("SCRUM-11", ItemType::Story, Some(scrum11.clone())),
        ("SCRUM-11", ItemType::Epic, None),
        ("not a key", ItemType::Story, None),
        (
            &gra_ini.to_string(),
            ItemType::Initiative,
            Some(gra_ini.clone()),
        ),
        ("JPD-9", ItemType::Initiative, Some(jpd9.clone())),
    ] {
        match (rr(raw, t).await, want) {
            (Ok(got), Some(want)) => assert_eq!(got, want, "resolve_ref({raw}, {t:?})"),
            (Err(HandlerError::NotFound), None) => {}
            (got, want) => panic!("resolve_ref({raw}, {t:?}) = {got:?}, want {want:?}"),
        }
    }

    for (raw, want) in [
        ("ABC-2", Some((ItemType::Epic, gra2.clone()))),
        ("ABC-999", None),
        ("OLD-2", Some((ItemType::Epic, old2.clone()))),
        ("OLD-1", Some((ItemType::Story, old1.clone()))),
        ("JPD-5", Some((ItemType::Epic, jpd5.clone()))),
        ("JPD-9", Some((ItemType::Initiative, jpd9.clone()))),
        (
            &gra_ini.to_string(),
            Some((ItemType::Initiative, gra_ini.clone())),
        ),
        ("SCRUM-11", Some((ItemType::Story, scrum11.clone()))),
    ] {
        match (h.resolve_any(raw).await, want) {
            (Ok(got), Some(want)) => assert_eq!(got, want, "resolve_any({raw})"),
            (Err(HandlerError::NotFound), None) => {}
            (got, want) => panic!("resolve_any({raw}) = {got:?}, want {want:?}"),
        }
    }

    for (raw, legacy, want) in [
        ("OLD-1", false, Some(old1.clone())),
        ("SCRUM-11", false, None),
        ("SCRUM-11", true, Some(scrum11.clone())),
        ("OLD-2", true, Some(from_old2.clone())),
        ("OLD-1", true, Some(old1.clone())),
        ("ABC-999", true, None),
        ("ABC-1", false, Some(gra1.clone())),
    ] {
        assert_eq!(
            h.find_story_by_branch_token(raw, legacy).await.unwrap(),
            want,
            "find_story_by_branch_token({raw}, {legacy})"
        );
    }

    let d = h
        .require_decision(&c, "d", &old2, "Pick", "dan@example.com", None, None)
        .await
        .unwrap();
    let on_type: String =
        sqlx::query_scalar("SELECT on_type FROM required_decisions WHERE id = $1")
            .bind(d.id())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(on_type, "epic");
}

/// An initiative with an epic holding a completed, a dropped and a todo story.
async fn seed_initiative_with_stories(h: &Handlers, tag: &str) {
    let dan = ctx("dan@example.com", false);
    let ini = h
        .create_initiative(&dan, &format!("{tag}-i"), "I", "", "Now", None, None, None)
        .await
        .unwrap();
    let epic = h
        .create_epic(
            &dan,
            &format!("{tag}-e"),
            "E",
            Some(ini.key().clone()),
            "",
            None,
        )
        .await
        .unwrap();
    let mut keys = Vec::new();
    for n in 0..4 {
        let s = h
            .create_story(
                &dan,
                &format!("{tag}-s{n}"),
                "S",
                Kind::Story,
                Some(epic.key().clone()),
                "",
                vec![],
                vec![],
                None,
                None,
            )
            .await
            .unwrap();
        keys.push(s.key().clone());
    }
    // Two completed, one dropped, one todo: swapping completed and dropped changes the ratio (2/3 vs 1/2).
    for k in [&keys[0], &keys[3]] {
        h.move_story(
            &dan,
            k,
            1,
            DeliveryState::InProgress,
            None,
            Some("seed".into()),
        )
        .await
        .unwrap();
        h.move_story(
            &dan,
            k,
            2,
            DeliveryState::Done,
            Some(Resolution::Completed),
            None,
        )
        .await
        .unwrap();
    }
    h.move_story(
        &dan,
        &keys[1],
        1,
        DeliveryState::Done,
        Some(Resolution::Dropped),
        None,
    )
    .await
    .unwrap();
}

/// The list counts every initiative's stories in a fixed number of statements.
#[tokio::test]
async fn list_initiatives_costs_the_same_for_two_and_six_initiatives() {
    let (_c, pool) = common::pg_counted().await;
    let (h, _sink) = mk(pool.clone());
    seed_initiative_with_stories(&h, "a").await;
    seed_initiative_with_stories(&h, "b").await;
    let list = || {
        let h = h.clone();
        async move {
            h.list_initiatives(Utc::now()).await.unwrap();
        }
    };
    list().await; // warm up
    let two = common::statements(&pool, list).await;
    let dan = ctx("dan@example.com", false);
    seed_initiative_with_stories(&h, "c").await;
    seed_initiative_with_stories(&h, "d").await;
    h.create_initiative(&dan, "e-i", "Bare", "", "Now", None, None, None)
        .await
        .unwrap();
    let f = h
        .create_initiative(&dan, "f-i", "Empty epic", "", "Now", None, None, None)
        .await
        .unwrap();
    h.create_epic(&dan, "f-e", "E", Some(f.key().clone()), "", None)
        .await
        .unwrap();
    assert_eq!(h.list_initiatives(Utc::now()).await.unwrap().len(), 6);
    let six = common::statements(&pool, list).await;
    assert_eq!(two, six);
}

/// The grouped counts agree with the one-at-a-time read, for initiatives without stories too.
#[tokio::test]
async fn list_initiatives_progress_matches_one_at_a_time() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    seed_initiative_with_stories(&h, "a").await;
    h.create_initiative(&dan, "e-i", "Bare", "", "Now", None, None, None)
        .await
        .unwrap();
    let f = h
        .create_initiative(&dan, "f-i", "Empty epic", "", "Now", None, None, None)
        .await
        .unwrap();
    h.create_epic(&dan, "f-e", "E", Some(f.key().clone()), "", None)
        .await
        .unwrap();
    let listed = h.list_initiatives(Utc::now()).await.unwrap();
    assert_eq!(listed.len(), 3);
    for dto in &listed {
        let key = dto.key.clone();
        assert_eq!(dto, &h.initiative_dto(&key, Utc::now()).await.unwrap());
    }
    assert_eq!(listed.iter().filter(|d| d.progress.is_none()).count(), 2);
    assert_eq!(
        listed
            .iter()
            .filter(|d| d.progress == Some(2.0 / 3.0))
            .count(),
        1
    );
}

/// A done window too large to subtract from the clock lists everything instead of panicking.
#[tokio::test]
async fn list_stories_with_an_absurd_done_window_lists_everything() {
    let (_c, pool) = common::pg().await;
    let (mut h, _sink) = mk(pool.clone());
    h.settings.done_window_days = u32::MAX;
    let keys = seed_board(&h, "a", 2, 1).await;
    let dan = ctx("dan@example.com", false);
    h.move_story(
        &dan,
        &keys[0],
        1,
        DeliveryState::Done,
        Some(Resolution::Dropped),
        None,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE stories SET done_at = '1970-01-01T00:00:00Z' WHERE key = $1")
        .bind(keys[0].to_string())
        .execute(&pool)
        .await
        .unwrap();
    let all = h
        .list_stories(
            &StoryFilter::default(),
            None,
            None,
            false,
            DoneFilter::Recent,
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
}

/// kb/ADR-0027 R3: only an admin's check stores `accepted`. A member's is refused before the row is read (so it says
/// nothing about versions, closure or existence) and writes nothing; a member's other results are stored; an admin's
/// automation token stores `accepted` (storing a result is not an override, kb/ADR-0027 R2).
#[tokio::test]
async fn only_an_admins_check_stores_accepted() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let epic = ready_epic(&h, &dan).await;
    let dan_bot = ctx("dan@example.com", true);
    let mia_bot = ctx("mia@example.com", true);
    let s = h
        .create_story(
            &dan,
            "s",
            "S",
            Kind::Chore,
            Some(epic.clone()),
            "",
            vec!["works".into()],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let d = h
        .require_decision(&dan, "d", s.key(), "Pick", "dan@example.com", None, None)
        .await
        .unwrap();
    let d = h
        .link_record(
            &dan,
            d.id(),
            "knowledge-base",
            "knowledge/adrs/0001.md",
            "kb/ADR-0001",
            None,
        )
        .await
        .unwrap();
    let closed = h
        .require_decision(&dan, "c", s.key(), "Old", "dan@example.com", None, None)
        .await
        .unwrap();
    let closed = h
        .link_record(
            &dan,
            closed.id(),
            "knowledge-base",
            "knowledge/adrs/0002.md",
            "kb/ADR-0002",
            None,
        )
        .await
        .unwrap();
    h.close_decision(&dan, closed.id(), ClosureKind::Withdrawn, "not needed")
        .await
        .unwrap();
    let accepted = || CheckResult::Accepted {
        commit: "abc".into(),
        reversibility: None,
    };
    let checked = || count(&sink, "checked", &d.id().to_string());

    // 1. A member's accepted: Forbidden; the decision, its version and the log are untouched.
    let before = h.get_decision(d.id()).await.unwrap();
    let err = h
        .record_check(
            &mia_bot,
            d.id(),
            "kb/ADR-0001",
            before.version(),
            accepted(),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, HandlerError::Forbidden), "{err:?}");
    let after = h.get_decision(d.id()).await.unwrap();
    assert_eq!(
        (after.version(), after.checked_at()),
        (before.version(), before.checked_at())
    );
    assert_eq!(after.check(), before.check());
    assert_eq!(checked(), 0);
    let r = h.readiness(s.key()).await.unwrap().reasons;
    assert!(
        matches!(r[..], [Reason::DecisionUnchecked { .. }]),
        "still waiting for a check: {r:?}"
    );

    // 2. Role first: a stale version, a closed decision and an unknown id all answer Forbidden to a member.
    for (id, record, version) in [
        (d.id(), "kb/ADR-0001", before.version() + 5),
        (closed.id(), "kb/ADR-0002", closed.version() + 1),
        (d.id() + 10_000, "kb/ADR-0001", 1),
    ] {
        let err = h
            .record_check(&mia_bot, id, record, version, accepted(), false)
            .await
            .unwrap_err();
        assert!(matches!(err, HandlerError::Forbidden), "{id}: {err:?}");
    }
    // ...while an admin's stale version is still the conflict it was.
    let err = h
        .record_check(
            &dan,
            d.id(),
            "kb/ADR-0001",
            before.version() + 5,
            accepted(),
            true,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, HandlerError::Conflict { .. }), "{err:?}");

    // 3. A member's not_found and not_accepted are stored, each yielding its own reason.
    h.record_check(
        &mia_bot,
        d.id(),
        "kb/ADR-0001",
        before.version(),
        CheckResult::NotFound,
        false,
    )
    .await
    .unwrap();
    let r = h.readiness(s.key()).await.unwrap().reasons;
    assert!(
        matches!(r[..], [Reason::DecisionRecordNotFound { .. }]),
        "{r:?}"
    );
    let v = h.get_decision(d.id()).await.unwrap().version();
    h.record_check(
        &mia_bot,
        d.id(),
        "kb/ADR-0001",
        v,
        CheckResult::NotAccepted {
            status: "proposed".into(),
            superseded_by: None,
            commit: "abc".into(),
            reversibility: None,
        },
        false,
    )
    .await
    .unwrap();
    let r = h.readiness(s.key()).await.unwrap().reasons;
    assert!(
        matches!(r[..], [Reason::DecisionNotAccepted { .. }]),
        "{r:?}"
    );
    assert_eq!(checked(), 2);

    // 4. An admin's automation token stores accepted, and the story is ready.
    let v = h.get_decision(d.id()).await.unwrap().version();
    h.record_check(&dan_bot, d.id(), "kb/ADR-0001", v, accepted(), true)
        .await
        .unwrap();
    assert!(h.readiness(s.key()).await.unwrap().ready);
    assert_eq!(checked(), 3);

    // 5. A posted unchecked is refused from anyone: a member cannot reset an accepted decision.
    let v = h.get_decision(d.id()).await.unwrap().version();
    let err = h
        .record_check(
            &mia_bot,
            d.id(),
            "kb/ADR-0001",
            v,
            CheckResult::Unchecked,
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, HandlerError::Validation(_)), "{err:?}");
    assert!(h.get_decision(d.id()).await.unwrap().is_satisfied());
    assert_eq!(checked(), 3);
}

/// The scheduled check posts every open, linked decision every few minutes (kb/ADR-0004 R4 covers satisfied ones too).
/// A result equal to the stored one (same kind and outcome; `commit` is left out, since `origin/HEAD` moves on every
/// push) only moves `checked_at`: no `checked` entry, no version bump. A changed result is logged and bumps.
#[tokio::test]
async fn an_unchanged_check_result_only_moves_checked_at() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan@example.com", true);
    let later = |mins: i64| Ctx {
        now: dan.now + chrono::Duration::minutes(mins),
        ..dan.clone()
    };
    let s = h
        .create_story(
            &dan,
            "s",
            "S",
            Kind::Chore,
            None,
            "",
            vec!["works".into()],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let d = h
        .require_decision(&dan, "d", s.key(), "Pick", "dan@example.com", None, None)
        .await
        .unwrap();
    let d = h
        .link_record(
            &dan,
            d.id(),
            "knowledge-base",
            "knowledge/adrs/0001.md",
            "kb/ADR-0001",
            None,
        )
        .await
        .unwrap();
    let checked = || count(&sink, "checked", &d.id().to_string());
    let accepted = |commit: &str| CheckResult::Accepted {
        commit: commit.into(),
        reversibility: None,
    };
    let not_accepted = |status: &str, by: Option<&str>, commit: &str| CheckResult::NotAccepted {
        status: status.into(),
        superseded_by: by.map(Into::into),
        commit: commit.into(),
        reversibility: None,
    };

    // The first result is a change from unchecked: logged, bumped.
    let first = h
        .record_check(
            &dan,
            d.id(),
            "kb/ADR-0001",
            d.version(),
            accepted("aaa"),
            true,
        )
        .await
        .unwrap();
    assert_eq!(checked(), 1);
    assert_eq!(first.version(), d.version() + 1);
    assert_eq!(first.checked_at(), Some(dan.now));

    // The same result at a later commit: only checked_at moves; the stored result is kept as it was.
    let again = h
        .record_check(
            &later(10),
            d.id(),
            "kb/ADR-0001",
            first.version(),
            accepted("bbb"),
            true,
        )
        .await
        .unwrap();
    assert_eq!(checked(), 1, "an unchanged result writes no activity entry");
    assert_eq!(
        again.version(),
        first.version(),
        "and does not bump the version"
    );
    assert_eq!(
        again.checked_at(),
        Some(later(10).now),
        "checked_at advances"
    );
    assert_eq!(again.check(), &accepted("aaa"));
    assert_eq!(h.get_decision(d.id()).await.unwrap(), again);

    // A changed result is logged and bumps, and so is each change of status or superseded_by.
    let mut v = again.version();
    let mut n = 1;
    for (mins, result) in [
        (20, not_accepted("superseded", Some("kb/ADR-0002"), "ccc")),
        (30, not_accepted("superseded", Some("kb/ADR-0003"), "ccc")),
        (40, not_accepted("rejected", Some("kb/ADR-0003"), "ccc")),
        (50, CheckResult::NotFound),
        (60, CheckResult::ProposedPrOpen),
    ] {
        let got = h
            .record_check(&later(mins), d.id(), "kb/ADR-0001", v, result.clone(), true)
            .await
            .unwrap();
        n += 1;
        assert_eq!(checked(), n, "{result:?} is a change");
        assert_eq!(got.version(), v + 1, "{result:?} bumps");
        assert_eq!(got.check(), &result);
        v = got.version();
        // ...and posting it again is not, at a later commit too (the commit is not part of the outcome).
        let again = match &result {
            CheckResult::NotAccepted {
                status,
                superseded_by,
                ..
            } => not_accepted(status, superseded_by.as_deref(), "ddd"),
            r => r.clone(),
        };
        let same = h
            .record_check(
                &later(mins + 5),
                d.id(),
                "kb/ADR-0001",
                v,
                again.clone(),
                true,
            )
            .await
            .unwrap();
        assert_eq!(checked(), n, "{again:?} again is unchanged");
        assert_eq!(same.version(), v);
        assert_eq!(same.checked_at(), Some(later(mins + 5).now));
        assert_eq!(same.check(), &result, "the stored result keeps its commit");
    }

    // The guards still run for an unchanged result: a stale version is a conflict.
    let err = h
        .record_check(
            &later(70),
            d.id(),
            "kb/ADR-0001",
            v - 1,
            CheckResult::ProposedPrOpen,
            true,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, HandlerError::Conflict { .. }), "{err:?}");
}

/// R2 of kb/ADR-0013: the record's reversibility is part of the outcome. A change in it alone is logged and bumps the
/// version; the same value again only moves `checked_at`; a relink drops the stored value with the whole result
/// (kb/ADR-0004.R1: the planner keeps nothing of the record beyond the check). Catches the old comparison that treats
/// every accepted result as equal.
#[tokio::test]
async fn a_change_in_the_records_reversibility_is_a_new_check_outcome() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan@example.com", true);
    let s = h
        .create_story(
            &dan,
            "s",
            "S",
            Kind::Chore,
            None,
            "",
            vec!["works".into()],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let d = h
        .require_decision(&dan, "d", s.key(), "Pick", "dan@example.com", None, None)
        .await
        .unwrap();
    let d = h
        .link_record(
            &dan,
            d.id(),
            "knowledge-base",
            "knowledge/adrs/0001.md",
            "kb/ADR-0001",
            None,
        )
        .await
        .unwrap();
    let checked = || count(&sink, "checked", &d.id().to_string());
    let accepted = |rv| CheckResult::Accepted {
        commit: "abc".into(),
        reversibility: rv,
    };

    let first = h
        .record_check(
            &dan,
            d.id(),
            "kb/ADR-0001",
            d.version(),
            accepted(None),
            true,
        )
        .await
        .unwrap();
    assert_eq!((checked(), first.version()), (1, d.version() + 1));

    let second = h
        .record_check(
            &dan,
            d.id(),
            "kb/ADR-0001",
            first.version(),
            accepted(Some(Reversibility::OneWay)),
            true,
        )
        .await
        .unwrap();
    assert_eq!((checked(), second.version()), (2, first.version() + 1));
    assert_eq!(second.check(), &accepted(Some(Reversibility::OneWay)));

    let third = h
        .record_check(
            &dan,
            d.id(),
            "kb/ADR-0001",
            second.version(),
            accepted(Some(Reversibility::OneWay)),
            true,
        )
        .await
        .unwrap();
    assert_eq!(checked(), 2, "the same value again is not a new outcome");
    assert_eq!(third.version(), second.version());

    let relinked = h
        .link_record(
            &dan,
            d.id(),
            "knowledge-base",
            "knowledge/adrs/0002.md",
            "kb/ADR-0002",
            None,
        )
        .await
        .unwrap();
    assert!(
        matches!(relinked.check(), CheckResult::Unchecked),
        "{:?}",
        relinked.check()
    );
}

const BLOCKERS: &str = "SELECT coalesce(string_agg(blocker_key, ',' ORDER BY blocker_key), '') \
                        FROM story_blocks WHERE story_key = $1";

#[tokio::test]
async fn remove_block_deletes_one_edge_and_logs_unblocked_last() {
    let (_c, pool) = common::pg().await;
    let (h, probe) = mk_probe(pool, BLOCKERS);
    let dan = ctx("dan@example.com", false);
    let epic = ready_epic(&h, &dan).await;
    // Under a discovery-complete epic with one criterion, the only possible reasons are its blockers.
    let s = h
        .create_story(
            &dan,
            "s",
            "Blocked",
            Kind::Bug,
            Some(epic.clone()),
            "",
            vec!["works".into()],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let b = h
        .create_story(
            &dan,
            "b",
            "Blocker B",
            Kind::Chore,
            None,
            "",
            vec![],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let c = h
        .create_story(
            &dan,
            "c",
            "Blocker C",
            Kind::Chore,
            None,
            "",
            vec![],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    h.add_block(&dan, s.key(), b.key()).await.unwrap();
    h.add_block(&dan, s.key(), c.key()).await.unwrap();
    let version = h.get_story(s.key()).await.unwrap().version();

    h.remove_block(&dan, s.key(), b.key()).await.unwrap();

    let dto = h.story_dto(s.key(), Utc::now()).await.unwrap();
    assert_eq!(
        dto.blocked_by,
        vec![c.key().clone()],
        "only the named edge goes"
    );
    assert_eq!(
        dto.version, version,
        "a block is not story content: no version moves"
    );
    assert_eq!(
        h.readiness(s.key()).await.unwrap().reasons,
        vec![Reason::BlockedBy {
            story: c.key().clone(),
            delivery: DeliveryState::Todo,
            owner: None
        }],
        "readiness is recomputed from the facts left"
    );
    let e = probe.entries("unblocked");
    assert_eq!(e.len(), 1);
    let (rec, inside, outside) = &e[0];
    assert_eq!(rec.entity_key, s.key().to_string());
    assert_eq!(rec.payload, serde_json::json!({"blocker": b.key()}));
    assert_eq!(rec.actor, "dan@example.com");
    assert_eq!(rec.at, dan.now, "the entry carries the Ctx's time");
    assert_eq!(
        inside.as_deref(),
        Some(c.key().to_string().as_str()),
        "deleted before the entry"
    );
    assert_eq!(
        outside.as_deref(),
        Some(format!("{},{}", b.key(), c.key()).as_str()),
        "the entry is written before commit"
    );

    // A missing edge (just removed, or a story named as its own blocker) is Ok and writes nothing.
    h.remove_block(&dan, s.key(), b.key()).await.unwrap();
    h.remove_block(&dan, s.key(), s.key()).await.unwrap();
    assert_eq!(probe.entries("unblocked").len(), 1);
    // Both stories must exist, as for add_block.
    let missing: Key = "ABC-999".parse().unwrap();
    assert!(matches!(
        h.remove_block(&dan, s.key(), &missing).await.unwrap_err(),
        HandlerError::NotFound
    ));
    assert!(matches!(
        h.remove_block(&dan, &missing, b.key()).await.unwrap_err(),
        HandlerError::NotFound
    ));
    // The last blocker gone, the story is ready.
    h.remove_block(&dan, s.key(), c.key()).await.unwrap();
    assert!(h.readiness(s.key()).await.unwrap().ready);
}

const DECISION: &str = "SELECT (SELECT title || '|' || decider || '|' || coalesce(due::text, '') \
                        FROM required_decisions WHERE id::text = $1)";

#[tokio::test]
async fn edit_decision_changes_planner_fields_only() {
    let (_c, pool) = common::pg().await;
    let (h, probe) = mk_probe(pool, DECISION);
    let dan = ctx("dan@example.com", false);
    let epic = ready_epic(&h, &dan).await;
    let s = h
        .create_story(
            &dan,
            "s",
            "Gated",
            Kind::Bug,
            Some(epic.clone()),
            "",
            vec!["works".into()],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let due = chrono::NaiveDate::from_ymd_opt(2026, 11, 30).unwrap();
    let d = h
        .require_decision(
            &dan,
            "d",
            s.key(),
            "Pick a queue",
            "dan@example.com",
            Some(due),
            None,
        )
        .await
        .unwrap();
    h.link_record(
        &dan,
        d.id(),
        "kb",
        "adrs/0031-queue.md",
        "kb/ADR-0031",
        Some("https://github.com/x/y/pull/9".into()),
    )
    .await
    .unwrap();
    let v = h.get_decision(d.id()).await.unwrap().version();
    // Only an admin's check stores accepted (m2d); this fixture is the admin's.
    h.record_check(
        &dan,
        d.id(),
        "kb/ADR-0031",
        v,
        CheckResult::Accepted {
            commit: "abc".into(),
            reversibility: None,
        },
        true,
    )
    .await
    .unwrap();
    let before = h.get_decision(d.id()).await.unwrap();
    assert!(h.readiness(s.key()).await.unwrap().ready);

    // One field: only it changes. The reference, PR url and check stay exactly as stored (kb/ADR-0004 R1).
    // The edit runs an hour after the check, so an edit that refreshed `checked_at` from its own clock fails below.
    let later = Ctx {
        now: dan.now + chrono::Duration::hours(1),
        ..dan.clone()
    };
    let e = h
        .edit_decision(
            &later,
            d.id(),
            before.version(),
            None,
            Some(" bob@example.com ".into()),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(e.decider(), "bob@example.com", "trimmed");
    assert_eq!((e.title(), e.due()), (before.title(), before.due()));
    assert_eq!(
        (&e.record(), &e.pr_url(), &e.check(), e.checked_at()),
        (
            &before.record(),
            &before.pr_url(),
            &before.check(),
            before.checked_at()
        )
    );
    assert_eq!(e.version(), before.version() + 1);
    assert!(
        h.readiness(s.key()).await.unwrap().ready,
        "an edit never unsatisfies the decision"
    );
    let edited = probe.entries("edited");
    assert_eq!(edited.len(), 1);
    // The entry names the changed field and carries its old and new value, so a retitle or a decider change of a
    // satisfied decision stays recoverable from the log (the actor is the entry's own column).
    assert_eq!(
        edited[0].0.payload,
        serde_json::json!({"fields": ["decider"], "from": {"decider": "dan@example.com"}, "to": {"decider": "bob@example.com"}})
    );
    assert_eq!(edited[0].0.actor, "dan@example.com");
    assert_eq!(
        edited[0].0.at, later.now,
        "the entry's time is the edit's own (kb/ADR-0010 R4)"
    );
    let id = d.id().to_string();
    assert_eq!(
        (edited[0].0.entity_type, edited[0].0.entity_key.as_str()),
        ("decision", id.as_str()),
        "filed under the decision, so it shows in the decision's history"
    );
    assert_eq!(
        edited[0].1.as_deref(),
        Some("Pick a queue|bob@example.com|2026-11-30"),
        "saved before the entry"
    );
    assert_eq!(
        edited[0].2.as_deref(),
        Some("Pick a queue|dan@example.com|2026-11-30"),
        "entry before commit"
    );

    // Two fields: the title trimmed, the due date cleared with Some(None).
    let e = h
        .edit_decision(
            &dan,
            d.id(),
            e.version(),
            Some("  Pick the queue  ".into()),
            None,
            Some(None),
            None,
        )
        .await
        .unwrap();
    assert_eq!((e.title(), e.due()), ("Pick the queue", None));
    assert_eq!(
        probe.entries("edited")[1].0.payload,
        serde_json::json!({
            "fields": ["title", "due"],
            "from": {"title": "Pick a queue", "due": "2026-11-30"},
            "to": {"title": "Pick the queue", "due": null}
        }),
        "old and new value of exactly the changed fields; a cleared due date is null"
    );

    // Values equal to the stored ones: no write, no entry, the version stays.
    let same = h
        .edit_decision(
            &dan,
            d.id(),
            e.version(),
            Some("Pick the queue".into()),
            Some("bob@example.com".into()),
            Some(None),
            None,
        )
        .await
        .unwrap();
    assert_eq!(same.version(), e.version());
    assert_eq!(probe.entries("edited").len(), 2);

    // Refusals write nothing, and their precedence is fixed: blank input (no row read) before the version, the version
    // before the closed rule. Each case asserts the exact error and that the version and the log did not move.
    const BLANK: &str = "title and decider must not be blank";
    let stale = e.version() - 1;
    assert!(matches!(
        h.edit_decision(&dan, d.id(), stale, Some("x".into()), None, None, None).await.unwrap_err(),
        HandlerError::Conflict { current } if current == e.version()
    ));
    // A stale no-op is still a conflict: the values match, but the caller has not seen the current row.
    assert!(matches!(
        h.edit_decision(&dan, d.id(), stale, Some("Pick the queue".into()), None, None, None).await.unwrap_err(),
        HandlerError::Conflict { current } if current == e.version()
    ));
    for version in [e.version(), stale] {
        for (title, decider) in [(Some("   ".to_string()), None), (None, Some(String::new()))] {
            assert!(
                matches!(
                    h.edit_decision(&dan, d.id(), version, title, decider, None, None).await.unwrap_err(),
                    HandlerError::Validation(ref m) if m == BLANK
                ),
                "blank input is refused before the version is compared (version {version})"
            );
        }
    }
    assert_eq!(
        h.get_decision(d.id()).await.unwrap().version(),
        e.version(),
        "no refusal moved the version"
    );
    assert_eq!(
        probe.entries("edited").len(),
        2,
        "no refusal wrote an entry"
    );
    assert!(matches!(
        h.edit_decision(&dan, 999_999, 1, Some("x".into()), None, None, None)
            .await
            .unwrap_err(),
        HandlerError::NotFound
    ));
    h.close_decision(&dan, d.id(), ClosureKind::Withdrawn, "not needed")
        .await
        .unwrap();
    let closed = h.get_decision(d.id()).await.unwrap();
    assert!(matches!(
        h.edit_decision(&dan, d.id(), closed.version(), Some("x".into()), None, None, None).await.unwrap_err(),
        HandlerError::Validation(m) if m == "decision is closed"
    ));
    // Stale on a closed decision: the version answers first, so the caller reloads and then sees it closed.
    assert!(matches!(
        h.edit_decision(&dan, d.id(), closed.version() - 1, Some("x".into()), None, None, None).await.unwrap_err(),
        HandlerError::Conflict { current } if current == closed.version()
    ));
    assert_eq!(
        h.get_decision(d.id()).await.unwrap().version(),
        closed.version()
    );
    assert_eq!(probe.entries("edited").len(), 2);
}

#[tokio::test]
async fn a_details_edit_keeps_an_unlisted_horizon() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);
    let ini = h
        .create_initiative(&dan, "i1", "Orion", "", "Now", None, None, None)
        .await
        .unwrap();
    // The configured buckets change: "Now" is no longer listed.
    let mut s = settings();
    s.horizon_buckets = vec!["Soon".into(), "Later".into()];
    let h2 = Handlers {
        ports: make_planning_ports(pool.clone()),
        activity: h.activity.clone(),
        pool,
        settings: s,
    };
    let edited = h2
        .edit_initiative(
            &dan,
            ini.key(),
            ini.version(),
            Some("Orion 2".into()),
            None,
            None,
            Some(Some("2026-12".into())),
            Some(Some("o@example.com".into())),
            None,
        )
        .await
        .unwrap();
    assert_eq!(edited.title(), "Orion 2");
    assert_eq!(edited.horizon(), "Now");
    assert_eq!(edited.owner(), Some("o@example.com"));
    // The stored, unlisted horizon sent back explicitly is not a change.
    let same = h2
        .edit_initiative(
            &dan,
            ini.key(),
            edited.version(),
            None,
            None,
            Some("Now".into()),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(same.horizon(), "Now");
    // A different unlisted value is refused and nothing is written.
    assert!(matches!(
        h2.edit_initiative(&dan, ini.key(), same.version(), None, None, Some("Someday".into()), None, None, None)
            .await
            .unwrap_err(),
        HandlerError::Validation(m) if m.contains("Someday")
    ));
    let stored = h2
        .ports
        .initiatives
        .get_initiative(ini.key())
        .await
        .unwrap();
    assert_eq!(stored.horizon(), "Now");
    assert_eq!(stored.version(), same.version());
    // A listed bucket is still accepted.
    let moved = h2
        .edit_initiative(
            &dan,
            ini.key(),
            same.version(),
            None,
            None,
            Some("Soon".into()),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(moved.horizon(), "Soon");
}

/// Rows created under one `Ctx` share `created_at`; the lists then break the tie by key number (ABC-9 before ABC-10).
#[tokio::test]
async fn list_initiatives_orders_ties_by_key_number() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);
    for n in 0..10 {
        h.create_initiative(&dan, &format!("i{n}"), "I", "", "Now", None, None, None)
            .await
            .unwrap();
    }
    // Rewrite the first row so the heap no longer returns insertion order.
    sqlx::query("UPDATE initiatives SET title = title WHERE key = 'ABC-1'")
        .execute(&pool)
        .await
        .unwrap();
    let keys: Vec<String> = h
        .list_initiatives(Utc::now())
        .await
        .unwrap()
        .iter()
        .map(|i| i.key.to_string())
        .collect();
    let want: Vec<String> = (1..=10).map(|n| format!("ABC-{n}")).collect();
    assert_eq!(keys, want, "initiatives");
}

#[tokio::test]
async fn list_epics_orders_ties_by_key_number() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);
    for n in 0..10 {
        h.create_epic(&dan, &format!("e{n}"), "E", None, "", None)
            .await
            .unwrap();
    }
    // Rewrite the first row so the heap no longer returns insertion order.
    sqlx::query("UPDATE epics SET title = title WHERE key = 'ABC-1'")
        .execute(&pool)
        .await
        .unwrap();
    let keys: Vec<String> = h
        .epic_dtos(None, Utc::now())
        .await
        .unwrap()
        .iter()
        .map(|e| e.key.to_string())
        .collect();
    let want: Vec<String> = (1..=10).map(|n| format!("ABC-{n}")).collect();
    assert_eq!(keys, want, "epics");
}

/// A record in a repository the server does not check yet is stored, not refused; readiness says the record is
/// in an unchecked repository, the scheduled check does not list it, and once the repository is configured the next
/// check picks it up without a relink.
#[tokio::test]
async fn a_record_in_an_unconfigured_repository_is_kept_until_the_repository_is_checked() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);
    let epic = ready_epic(&h, &dan).await;
    let s = h
        .create_story(
            &dan,
            "s",
            "Ship",
            Kind::Chore,
            Some(epic.clone()),
            "",
            vec!["ok".into()],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let d = h
        .require_decision(
            &dan,
            "d",
            s.key(),
            "Pick a host",
            "dan@example.com",
            None,
            None,
        )
        .await
        .unwrap();
    let linked = h
        .link_record(
            &dan,
            d.id(),
            "infra",
            "docs/adr/0001-host.md",
            "infra/ADR-0001",
            None,
        )
        .await
        .expect("an unconfigured repository is stored, not refused");
    assert_eq!(linked.record().as_ref().unwrap().repo(), "infra");
    // a bad path is still refused
    assert!(matches!(
        h.link_record(&dan, d.id(), "infra", "../x.md", "infra/ADR-0001", None)
            .await,
        Err(HandlerError::Validation(_))
    ));
    let r = h.readiness(s.key()).await.unwrap();
    assert_eq!(
        r.reasons,
        vec![Reason::DecisionRepoNotConfigured {
            decision_id: d.id(),
            title: "Pick a host".into()
        }]
    );
    assert!(
        h.list_decisions(None, true, false)
            .await
            .unwrap()
            .is_empty()
    );

    // the same database, with infra configured
    let mut configured = settings();
    configured.repos.push("infra".into());
    let h2 = Handlers {
        ports: make_planning_ports(pool.clone()),
        activity: std::sync::Arc::new(MemSink(std::sync::Mutex::new(vec![]))),
        pool,
        settings: configured,
    };
    assert_eq!(
        h2.list_decisions(None, true, false)
            .await
            .unwrap()
            .iter()
            .map(|x| x.id())
            .collect::<Vec<_>>(),
        vec![d.id()]
    );
    assert_eq!(
        h2.readiness(s.key()).await.unwrap().reasons,
        vec![Reason::DecisionUnchecked {
            decision_id: d.id(),
            title: "Pick a host".into()
        }]
    );
}

/// Codex review on #33: two applies of one plan at once both answer, and each item is reported created by exactly one.
#[tokio::test]
async fn concurrent_applies_of_one_plan_report_each_item_created_once() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let plan: reeve_protocol::ApplyPlan = serde_json::from_value(serde_json::json!({
        "id": "race",
        "epics": [{"name": "e", "title": "E"}],
        "stories": [{"name": "a", "title": "A", "epic": "e"}, {"name": "b", "title": "B", "epic": "e"}]
    }))
    .unwrap();
    for _ in 0..5 {
        let (x, y) = tokio::join!(
            h.apply_plan(&dan, plan.clone()),
            h.apply_plan(&dan, plan.clone())
        );
        let (x, y) = (x.unwrap(), y.unwrap());
        for (i, j) in x.items.iter().zip(&y.items) {
            assert_eq!(i.key, j.key);
            assert!(
                !(i.created && j.created),
                "{} reported created twice",
                i.name
            );
        }
    }
}

fn last(sink: &MemSink, action: &str) -> ActivityRecord {
    sink.0
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|e| e.action == action)
        .cloned()
        .unwrap()
}

/// Create logs the classification in `required`; an idempotent replay returns the first row's classification; an
/// edit changes it under the version and logs `edited` with from/to; a closed decision refuses the edit. Catches a
/// classification set silently, overwritten by a replay, or edited without a log entry.
#[tokio::test]
async fn reversibility_is_set_on_create_and_changed_only_by_a_logged_edit() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let s = seed_board(&h, "rv", 1, 900).await.remove(0);
    let d = h
        .require_decision(
            &dan,
            "rv-1",
            &s,
            "Pick a db",
            "dan",
            None,
            Some(Reversibility::OneWay),
        )
        .await
        .unwrap();
    assert_eq!(d.reversibility(), Some(Reversibility::OneWay));
    assert_eq!(last(&sink, "required").payload["reversibility"], "one_way");
    let again = h
        .require_decision(
            &dan,
            "rv-1",
            &s,
            "Pick a db",
            "dan",
            None,
            Some(Reversibility::TwoWay),
        )
        .await
        .unwrap();
    assert_eq!(
        (again.id(), again.reversibility()),
        (d.id(), Some(Reversibility::OneWay))
    );

    let e = h
        .edit_decision(
            &dan,
            d.id(),
            d.version(),
            None,
            None,
            None,
            Some(Some(Reversibility::TwoWay)),
        )
        .await
        .unwrap();
    assert_eq!(
        (e.reversibility(), e.version()),
        (Some(Reversibility::TwoWay), d.version() + 1)
    );
    let edited = last(&sink, "edited");
    assert_eq!(edited.payload["from"]["reversibility"], "one_way");
    assert_eq!(edited.payload["to"]["reversibility"], "two_way");

    h.close_decision(&dan, d.id(), ClosureKind::Decided, "Postgres")
        .await
        .unwrap();
    assert!(matches!(
        h.edit_decision(&dan, d.id(), e.version() + 1, None, None, None, Some(None)).await.unwrap_err(),
        HandlerError::Validation(m) if m == "decision is closed"));
}

/// R3 through the handler: Decide on one-way is refused with the domain's message and writes nothing; Withdraw stores
/// kind and the caller's token kind; two-way Decide stores and logs `decided`; a system caller is refused before any
/// write. Catches a close committed before the rule, a hard-coded via, and a System via reaching the CHECK as a 500.
#[tokio::test]
async fn close_stores_kind_and_via_and_refuses_decided_one_way() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let bot = ctx_via("dan@example.com", Via::Automation);
    let s = seed_board(&h, "cl", 1, 910).await.remove(0);
    let one = h
        .require_decision(
            &dan,
            "c-1",
            &s,
            "Pricing",
            "dan",
            None,
            Some(Reversibility::OneWay),
        )
        .await
        .unwrap();
    let two = h
        .require_decision(
            &dan,
            "c-2",
            &s,
            "Copy",
            "dan",
            None,
            Some(Reversibility::TwoWay),
        )
        .await
        .unwrap();

    let err = h
        .close_decision(&dan, one.id(), ClosureKind::Decided, "flat fee")
        .await
        .unwrap_err();
    assert!(
        matches!(err, HandlerError::Validation(ref m) if m.starts_with("a one-way decision is decided")),
        "{err:?}"
    );
    let after = h.get_decision(one.id()).await.unwrap();
    assert_eq!((after.closed(), after.version()), (None, one.version()));
    assert_eq!(count(&sink, "closed", &one.id().to_string()), 0);

    let sys = ctx_via("reeve", Via::System);
    assert!(matches!(
        h.close_decision(&sys, one.id(), ClosureKind::Withdrawn, "x")
            .await
            .unwrap_err(),
        HandlerError::Forbidden
    ));
    let after = h.get_decision(one.id()).await.unwrap();
    assert_eq!((after.closed(), after.version()), (None, one.version()));
    assert_eq!(count(&sink, "closed", &one.id().to_string()), 0);

    let w = h
        .close_decision(&bot, one.id(), ClosureKind::Withdrawn, "dropped the tier")
        .await
        .unwrap();
    let c = w.closed().unwrap();
    assert_eq!(
        (c.kind, c.via),
        (Some(ClosureKind::Withdrawn), Some(TokenKind::Automation))
    );

    let d = h
        .close_decision(&dan, two.id(), ClosureKind::Decided, "plain words")
        .await
        .unwrap();
    let c = d.closed().unwrap();
    assert_eq!(
        (c.kind, c.via),
        (Some(ClosureKind::Decided), Some(TokenKind::Interactive))
    );
    assert_eq!(last(&sink, "closed").payload["kind"], "decided");
}

/// The queue reads in a fixed number of statements: a board of three stories costs what a board of twelve costs,
/// with more decisions. Catches a per-decision or per-story query.
#[tokio::test]
async fn decision_queue_costs_the_same_for_few_and_many() {
    let (_c, pool) = common::pg_counted().await;
    let (h, _sink) = mk(pool.clone());
    seed_board(&h, "a", 3, 1).await; // three todo stories, a decision on the epic and one on the first story
    let q = || {
        let h = h.clone();
        async move {
            h.decision_queue(
                &DecisionScope::All {
                    include_closed: false,
                },
                Utc::now(),
            )
            .await
            .unwrap();
        }
    };
    q().await; // warm up: prepared statements, catalog caches
    let few = common::statements(&pool, q).await;
    assert!(few > 5, "the counter measured {few} statements");
    seed_board(&h, "b", 9, 100).await;
    seed_board(&h, "c", 4, 200).await;
    let many = common::statements(&pool, q).await;
    assert_eq!(few, many);
}

/// End to end over real readiness. Epic E has todo stories S1, S2 and an in-progress S3; decision D1 is on E and D2 on
/// S1. D1 blocks S1 and S2, not S3; D2 blocks S1; the summary counts S1 once (blocked_stories 2). Accepting D1's
/// record moves it to satisfied with no blocks. Scoped by `On(S1)` the queue lists only D2; scoped by `Initiative(I)`
/// it lists D1 and D2 and a decision on another initiative is absent. Catches counting active stories, a stale group
/// after acceptance and a scope that leaks.
#[tokio::test]
async fn decision_queue_over_real_readiness() {
    use reeve_protocol::QueueGroup;
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let ini = h
        .create_initiative(&dan, "q-i", "I", "", "Now", None, None, None)
        .await
        .unwrap();
    let epic = h
        .create_epic(&dan, "q-e", "E", Some(ini.key().clone()), "", None)
        .await
        .unwrap();
    let mut stories = Vec::new();
    for n in 1..=3 {
        let s = h
            .create_story(
                &dan,
                &format!("q-s{n}"),
                &format!("S{n}"),
                Kind::Story,
                Some(epic.key().clone()),
                "",
                vec!["works".into()],
                vec![],
                None,
                None,
            )
            .await
            .unwrap();
        stories.push(s);
    }
    let (s1, s2, s3) = (&stories[0], &stories[1], &stories[2]);
    let d1 = h
        .require_decision(
            &dan,
            "q-d1",
            epic.key(),
            "D1",
            "dan@example.com",
            None,
            None,
        )
        .await
        .unwrap();
    let d2 = h
        .require_decision(&dan, "q-d2", s1.key(), "D2", "dan@example.com", None, None)
        .await
        .unwrap();
    let moved = h
        .move_story(
            &dan,
            s3.key(),
            s3.version(),
            DeliveryState::InProgress,
            None,
            Some("started before the decision".into()),
        )
        .await
        .unwrap();
    assert_eq!(moved.delivery(), DeliveryState::InProgress);
    let ini2 = h
        .create_initiative(&dan, "q-i2", "I2", "", "Now", None, None, None)
        .await
        .unwrap();
    let d_i2 = h
        .require_decision(
            &dan,
            "q-d3",
            ini2.key(),
            "D3",
            "dan@example.com",
            None,
            None,
        )
        .await
        .unwrap();

    let ids = |q: &[QueueEntry]| q.iter().map(|e| e.decision.id()).collect::<Vec<_>>();
    let (all, summary) = h
        .decision_queue(
            &DecisionScope::All {
                include_closed: false,
            },
            dan.now,
        )
        .await
        .unwrap();
    assert_eq!(
        all.iter()
            .map(|e| (e.group, e.decision.id()))
            .collect::<Vec<_>>(),
        vec![
            (QueueGroup::Blocking, d1.id()),
            (QueueGroup::Blocking, d2.id()),
            (QueueGroup::Open, d_i2.id())
        ]
    );
    let keys = |e: &QueueEntry| e.held.iter().map(|b| b.key.clone()).collect::<Vec<_>>();
    assert_eq!(keys(&all[0]), vec![s1.key().clone(), s2.key().clone()]);
    assert_eq!(all[0].held[0].title, "S1");
    assert_eq!(keys(&all[1]), vec![s1.key().clone()]);
    assert!(all[2].held.is_empty());
    assert_eq!(
        (summary.open, summary.blocking, summary.blocked_stories),
        (3, 2, 2)
    );

    let (on_s1, _) = h
        .decision_queue(&DecisionScope::On(s1.key().clone()), dan.now)
        .await
        .unwrap();
    assert_eq!(ids(&on_s1), vec![d2.id()]);
    let (under_i, _) = h
        .decision_queue(&DecisionScope::Initiative(ini.key().clone()), dan.now)
        .await
        .unwrap();
    assert_eq!(ids(&under_i), vec![d1.id(), d2.id()]);

    let linked = h
        .link_record(
            &dan,
            d1.id(),
            "knowledge-base",
            "knowledge/adrs/0040-queue.md",
            "kb/ADR-0040",
            None,
        )
        .await
        .unwrap();
    h.record_check(
        &dan,
        d1.id(),
        "kb/ADR-0040",
        linked.version(),
        CheckResult::Accepted {
            commit: "abc".into(),
            reversibility: None,
        },
        true,
    )
    .await
    .unwrap();
    let (all, summary) = h
        .decision_queue(
            &DecisionScope::All {
                include_closed: false,
            },
            dan.now,
        )
        .await
        .unwrap();
    let e1 = all.iter().find(|e| e.decision.id() == d1.id()).unwrap();
    assert_eq!(e1.group, QueueGroup::Satisfied);
    assert!(e1.held.is_empty());
    let e2 = all.iter().find(|e| e.decision.id() == d2.id()).unwrap();
    assert_eq!(
        (e2.group, keys(e2)),
        (QueueGroup::Blocking, vec![s1.key().clone()])
    );
    assert_eq!(
        (summary.open, summary.blocking, summary.blocked_stories),
        (2, 1, 1)
    );
}

/// Opens a sprint whose goal serves `active` and leaves a draft whose goal serves `draft` (items may be of any level);
/// returns (active goal id, draft goal id).
async fn sprints_serving(
    h: &Handlers,
    c: &Ctx,
    tag: &str,
    active: &[reeve_protocol::Key],
    draft: &[reeve_protocol::Key],
) -> (i64, i64) {
    let (a, _) = h
        .add_goal(
            c,
            &format!("{tag}-ga"),
            "active outcome",
            "check",
            None,
            "",
            false,
            &[],
        )
        .await
        .unwrap();
    h.link_goal(c, a.id, active).await.unwrap();
    let d = h.sprints(c.now).await.unwrap().draft.unwrap();
    h.open_sprint(c, d.id, d.version, None).await.unwrap();
    let (g, _) = h
        .add_goal(
            c,
            &format!("{tag}-gd"),
            "draft outcome",
            "check",
            None,
            "",
            false,
            &[],
        )
        .await
        .unwrap();
    h.link_goal(c, g.id, draft).await.unwrap();
    (a.id, g.id)
}

/// A discovery-complete initiative and epic with `n` ready-but-for-decisions stories (one criterion each).
async fn marked_epic_with_stories(
    h: &Handlers,
    c: &Ctx,
    tag: &str,
    n: usize,
) -> (reeve_protocol::Key, Vec<Story>) {
    let ini = h
        .create_initiative(c, &format!("{tag}-i"), "I", "", "Now", None, None, None)
        .await
        .unwrap();
    h.mark_discovery(c, ItemType::Initiative, ini.key(), ini.version(), true)
        .await
        .unwrap();
    let epic = h
        .create_epic(
            c,
            &format!("{tag}-e"),
            "E",
            Some(ini.key().clone()),
            "",
            None,
        )
        .await
        .unwrap();
    h.mark_discovery(c, ItemType::Epic, epic.key(), epic.version(), true)
        .await
        .unwrap();
    let mut stories = vec![];
    for i in 0..n {
        stories.push(
            h.create_story(
                c,
                &format!("{tag}-s{i}"),
                &format!("S{i}"),
                Kind::Story,
                Some(epic.key().clone()),
                "",
                vec!["works".into()],
                vec![],
                None,
                None,
            )
            .await
            .unwrap(),
        );
    }
    (epic.key().clone(), stories)
}

async fn queue_all(h: &Handlers, c: &Ctx) -> (Vec<QueueEntry>, QueueSummary) {
    h.decision_queue(
        &DecisionScope::All {
            include_closed: false,
        },
        c.now,
    )
    .await
    .unwrap()
}

/// D1 on S1 (active goal) and D2 on S2 (draft goal) land in this_sprint and next_sprint with the goal as the reason, and
/// the summary counts one each. Catches the draft read not wired and goals of the wrong sprint.
#[tokio::test]
async fn queue_needed_in_from_active_and_draft_goals() {
    use reeve_protocol::{NeededIn, NeededInReason, QueueLane};
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let (_e, ss) = marked_epic_with_stories(&h, &dan, "n", 2).await;
    let d1 = h
        .require_decision(&dan, "d1", ss[0].key(), "D1", "dan@example.com", None, None)
        .await
        .unwrap();
    let d2 = h
        .require_decision(&dan, "d2", ss[1].key(), "D2", "dan@example.com", None, None)
        .await
        .unwrap();
    let (ga, gd) = sprints_serving(
        &h,
        &dan,
        "n",
        std::slice::from_ref(ss[0].key()),
        std::slice::from_ref(ss[1].key()),
    )
    .await;
    let (es, summary) = queue_all(&h, &dan).await;
    let of = |id: i64| es.iter().find(|e| e.decision.id() == id).unwrap();
    let (e1, e2) = (of(d1.id()), of(d2.id()));
    assert_eq!(
        (e1.needed_in, e1.lane, e1.needed_in_reason.clone()),
        (
            NeededIn::ThisSprint,
            QueueLane::ThisSprint,
            Some(NeededInReason::Goal {
                goal_id: ga,
                outcome: "active outcome".into()
            })
        )
    );
    assert_eq!(
        (e2.needed_in, e2.lane, e2.needed_in_reason.clone()),
        (
            NeededIn::NextSprint,
            QueueLane::NextSprint,
            Some(NeededInReason::Goal {
                goal_id: gd,
                outcome: "draft outcome".into()
            })
        )
    );
    assert_eq!((summary.this_sprint, summary.next_sprint), (1, 1));
}

/// A decision on an initiative in discovery under the first bucket, with no stories, lands in next_sprint; under the
/// second bucket it stays open. Catches the counted-but-no-lane case end to end.
#[tokio::test]
async fn queue_discovery_decision_counts_in_next_sprint_lane() {
    use reeve_protocol::{NeededInReason, QueueLane};
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let now_ini = h
        .create_initiative(&dan, "i-now", "I", "", "Now", None, None, None)
        .await
        .unwrap();
    let later_ini = h
        .create_initiative(&dan, "i-later", "I2", "", "Later", None, None, None)
        .await
        .unwrap();
    let d1 = h
        .require_decision(
            &dan,
            "d1",
            now_ini.key(),
            "D1",
            "dan@example.com",
            None,
            None,
        )
        .await
        .unwrap();
    let d2 = h
        .require_decision(
            &dan,
            "d2",
            later_ini.key(),
            "D2",
            "dan@example.com",
            None,
            None,
        )
        .await
        .unwrap();
    let (es, summary) = queue_all(&h, &dan).await;
    let of = |id: i64| es.iter().find(|e| e.decision.id() == id).unwrap();
    assert_eq!(of(d1.id()).lane, QueueLane::NextSprint);
    assert_eq!(
        of(d1.id()).needed_in_reason,
        Some(NeededInReason::Discovery {
            key: now_ini.key().clone(),
            level: reeve_protocol::ItemType::Initiative
        })
    );
    assert_eq!(of(d2.id()).lane, QueueLane::Open);
    assert_eq!(summary.next_sprint, 1);
}

/// A started story still counts: D is on epic E, S (in progress by override) serves an active goal. Catches
/// "unfinished" narrowed to todo.
#[tokio::test]
async fn queue_under_way_story_counts() {
    use reeve_protocol::{NeededIn, QueueGroup};
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let (epic, ss) = marked_epic_with_stories(&h, &dan, "u", 1).await;
    let d = h
        .require_decision(&dan, "d", &epic, "D", "dan@example.com", None, None)
        .await
        .unwrap();
    let s = &ss[0];
    h.move_story(
        &dan,
        s.key(),
        s.version(),
        DeliveryState::InProgress,
        None,
        Some("started before the decision".into()),
    )
    .await
    .unwrap();
    sprints_serving(&h, &dan, "u", std::slice::from_ref(s.key()), &[]).await;
    let (es, _) = queue_all(&h, &dan).await;
    let e = es.iter().find(|e| e.decision.id() == d.id()).unwrap();
    assert!(e.held.is_empty());
    assert_eq!(
        e.under_way
            .iter()
            .map(|u| u.key.clone())
            .collect::<Vec<_>>(),
        vec![s.key().clone()]
    );
    assert_eq!(e.needed_in, NeededIn::ThisSprint);
    assert_eq!(e.group, QueueGroup::Open, "old clients see no change");
}

/// A record in a repository the server does not check needs the repository configured, not a wait. Catches the
/// unconfigured-repo case shown as waiting for the check.
#[tokio::test]
async fn queue_next_step_configure_repo() {
    use reeve_protocol::NextStep;
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let (epic, _) = marked_epic_with_stories(&h, &dan, "r", 1).await;
    let d = h
        .require_decision(&dan, "d", &epic, "D", "dan@example.com", None, None)
        .await
        .unwrap();
    h.link_record(&dan, d.id(), "infra", "adr/1.md", "ADR-1", None)
        .await
        .unwrap();
    let (es, _) = queue_all(&h, &dan).await;
    assert_eq!(
        es[0].next_step,
        Some(NextStep::ConfigureRepo {
            repo: "infra".into()
        })
    );
}

/// A held story is urgent when its epic is marked (lineage, kb/ADR-0017 R3). Catches `Urgency::default()` left in place.
#[tokio::test]
async fn queue_held_story_urgency_is_lineage() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let (epic, ss) = marked_epic_with_stories(&h, &dan, "g", 1).await;
    h.require_decision(&dan, "d", ss[0].key(), "D", "dan@example.com", None, None)
        .await
        .unwrap();
    let (before, _) = queue_all(&h, &dan).await;
    assert!(!before[0].held[0].urgent);
    h.mark_urgent(&dan, ItemType::Epic, &epic, None)
        .await
        .unwrap();
    let (after, _) = queue_all(&h, &dan).await;
    assert!(after[0].held[0].urgent);
}

/// The context is the queue's own entry for that decision. Catches a second implementation of the classifier.
#[tokio::test]
async fn decision_context_returns_one_entry() {
    use reeve_protocol::QueueLane;
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let (_e, ss) = marked_epic_with_stories(&h, &dan, "c", 2).await;
    let d = h
        .require_decision(&dan, "d", ss[0].key(), "D", "dan@example.com", None, None)
        .await
        .unwrap();
    let other = h
        .require_decision(&dan, "d2", ss[1].key(), "D2", "dan@example.com", None, None)
        .await
        .unwrap();
    sprints_serving(&h, &dan, "c", std::slice::from_ref(ss[0].key()), &[]).await;
    let (es, _) = queue_all(&h, &dan).await;
    let want = es.iter().find(|e| e.decision.id() == d.id()).unwrap();
    let got = h.decision_context(d.id(), dan.now).await.unwrap();
    let keys = |e: &QueueEntry| e.held.iter().map(|b| b.key.clone()).collect::<Vec<_>>();
    assert_eq!(
        (got.lane, got.needed_in, got.next_step.clone(), keys(&got)),
        (
            want.lane,
            want.needed_in,
            want.next_step.clone(),
            keys(want)
        )
    );
    assert_eq!(got.lane, QueueLane::ThisSprint);
    let o = h.decision_context(other.id(), dan.now).await.unwrap();
    assert_eq!(o.lane, QueueLane::Beyond);
    h.close_decision(
        &dan,
        d.id(),
        reeve_protocol::ClosureKind::Withdrawn,
        "not needed",
    )
    .await
    .unwrap();
    assert_eq!(
        h.decision_context(d.id(), dan.now).await.unwrap().lane,
        QueueLane::Closed
    );
    assert!(matches!(
        h.decision_context(9999, dan.now).await,
        Err(HandlerError::NotFound)
    ));
}

/// Goals, a draft sprint and an urgent mark add no statements: the queue costs the same with few and with many stories,
/// goals and marks. Catches a per-goal or per-mark read that the goal-less baseline cannot.
#[tokio::test]
async fn decision_queue_costs_the_same_with_goals() {
    let (_c, pool) = common::pg_counted().await;
    let (h, _sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);
    let a = seed_board(&h, "a", 3, 1).await;
    sprints_serving(&h, &dan, "a", &a[..1], &a[1..2]).await;
    h.mark_urgent(&dan, ItemType::Story, &a[2], None)
        .await
        .unwrap();
    let q = || {
        let h = h.clone();
        let c = dan.clone();
        async move {
            queue_all(&h, &c).await;
        }
    };
    q().await;
    let few = common::statements(&pool, q).await;
    assert!(few > 5, "the counter measured {few} statements");
    let b = seed_board(&h, "b", 9, 100).await;
    let c = seed_board(&h, "c", 4, 200).await;
    // Grow the ACTIVE sprint's goals (even i, to_active) and the draft's (odd i), each linked to seeded stories.
    for (i, k) in b.iter().chain(c.iter()).enumerate() {
        let (g, _) = h
            .add_goal(
                &dan,
                &format!("more-{i}"),
                "o",
                "c",
                None,
                "",
                i % 2 == 0,
                &[],
            )
            .await
            .unwrap();
        h.link_goal(&dan, g.id, std::slice::from_ref(k))
            .await
            .unwrap();
        if i % 3 == 0 {
            h.mark_urgent(&dan, ItemType::Story, k, None).await.unwrap();
        }
    }
    let many = common::statements(&pool, q).await;
    assert_eq!(few, many);
}

/// The queue reads only unfinished stories (delivery not done): done stories under a decision's epic are neither held
/// nor under way, and adding them changes no row the queue reads. The domain drops done stories too, so the queue's
/// answer is the same either way: only this rows-read comparison catches a regression to `DoneWindow::All` (every done
/// story in the history fetched, then thrown away). A future queue read whose rows legitimately grow with done
/// stories must update this test.
#[tokio::test]
async fn decision_queue_reads_no_done_story() {
    let (_c, pool) = common::pg_counted().await;
    let (h, _sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);
    let (epic, ss) = marked_epic_with_stories(&h, &dan, "f", 2).await;
    let d = h
        .require_decision(&dan, "d", &epic, "D", "dan@example.com", None, None)
        .await
        .unwrap();
    let q = || {
        let (h, c) = (h.clone(), dan.clone());
        async move {
            queue_all(&h, &c).await;
        }
    };
    q().await; // warm up: prepared statements, catalog caches
    let before = common::rows(&pool, q).await;
    assert!(before > 0, "the counter measured {before} rows");
    let mut done = Vec::new();
    for i in 0..5 {
        let s = h
            .create_story(
                &dan,
                &format!("f-done{i}"),
                "Done",
                Kind::Story,
                Some(epic.clone()),
                "",
                vec!["works".into()],
                vec![],
                None,
                None,
            )
            .await
            .unwrap();
        h.move_story(
            &dan,
            s.key(),
            s.version(),
            DeliveryState::Done,
            Some(Resolution::Dropped),
            Some("dropped before the decision".into()),
        )
        .await
        .unwrap();
        done.push(s.key().clone());
    }
    let (es, _) = queue_all(&h, &dan).await;
    let e = es.iter().find(|e| e.decision.id() == d.id()).unwrap();
    let affected: Vec<Key> = e
        .held
        .iter()
        .chain(&e.under_way)
        .map(|s| s.key.clone())
        .collect();
    assert_eq!(
        affected,
        ss.iter().map(|s| s.key().clone()).collect::<Vec<_>>()
    );
    let after = common::rows(&pool, q).await;
    assert_eq!(
        before, after,
        "the queue read rows of the done stories {done:?}"
    );
}

/// The queue reads one snapshot: a goal link committed while the read waits does not change the lane the read reports;
/// a fresh call sees it. Catches reads taken on separate snapshots.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn decision_queue_reads_one_snapshot() {
    use reeve_protocol::QueueLane;
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);
    let (_e, ss) = marked_epic_with_stories(&h, &dan, "z", 1).await;
    let d = h
        .require_decision(&dan, "d", ss[0].key(), "D", "dan@example.com", None, None)
        .await
        .unwrap();
    // A draft goal exists but serves nothing yet.
    let (_ga, gd) = sprints_serving(&h, &dan, "z", &[], &[]).await;
    assert_eq!(queue_all(&h, &dan).await.0[0].lane, QueueLane::Beyond);

    // The queue's statements run on one snapshot; the call waits on `initiatives` (read after its first statements),
    // while the test commits a goal link, which touches no `initiatives` row.
    let mut lock = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE initiatives IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let call = tokio::spawn({
        let (h, c) = (h.clone(), dan.clone());
        async move { queue_all(&h, &c).await }
    });
    wait_until_blocked(&pool).await;
    tokio::time::timeout(PAUSE, async {
        let mut w = pool.begin().await.unwrap();
        h.ports
            .sprints
            .insert_goal_link(&mut w, gd, ss[0].key(), "dan@example.com", dan.now)
            .await
            .unwrap();
        w.commit().await.unwrap();
    })
    .await
    .expect("the commit never finished");
    lock.rollback().await.unwrap();
    let (es, _) = tokio::time::timeout(PAUSE, call)
        .await
        .expect("the queue never finished")
        .unwrap();
    assert_eq!(es[0].decision.id(), d.id());
    assert_eq!(es[0].lane, QueueLane::Beyond, "the old lane");
    assert_eq!(queue_all(&h, &dan).await.0[0].lane, QueueLane::NextSprint);
}

/// Marking and clearing: interactive only, logged under their own actions, renew replaces, clear is idempotent.
#[tokio::test]
async fn urgent_marks_are_set_renewed_and_cleared_by_people_only() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let bot = ctx("dan@example.com", true);
    let ini = h
        .create_initiative(&dan, "i", "Init", "", "Now", None, None, None)
        .await
        .unwrap();
    let k = ini.key().clone();
    assert!(matches!(
        h.mark_urgent(&bot, ItemType::Initiative, &k, None)
            .await
            .unwrap_err(),
        HandlerError::Refused(r) if r == vec![Reason::InteractiveOnly]
    ));
    assert!(matches!(
        h.mark_urgent(&dan, ItemType::Initiative, &k, Some("x".repeat(201)))
            .await
            .unwrap_err(),
        HandlerError::Validation(_)
    ));
    assert!(matches!(
        h.mark_urgent(&dan, ItemType::Epic, &k, None)
            .await
            .unwrap_err(),
        HandlerError::NotFound
    ));
    h.mark_urgent(
        &dan,
        ItemType::Initiative,
        &k,
        Some("  Customer X  ".into()),
    )
    .await
    .unwrap();
    let payload = |sink: &MemSink, n: usize| {
        sink.0
            .lock()
            .unwrap()
            .iter()
            .filter(|a| a.action == "urgent_marked")
            .nth(n)
            .unwrap()
            .payload
            .clone()
    };
    assert_eq!(
        payload(&sink, 0),
        serde_json::json!({"item_type": "initiative", "reason": "Customer X"})
    );
    // characters, not bytes; and a blank reason is no reason
    h.mark_urgent(&dan, ItemType::Initiative, &k, Some("é".repeat(200)))
        .await
        .unwrap();
    assert!(matches!(
        h.mark_urgent(&dan, ItemType::Initiative, &k, Some("é".repeat(201)))
            .await
            .unwrap_err(),
        HandlerError::Validation(_)
    ));
    h.mark_urgent(&dan, ItemType::Initiative, &k, Some("   ".into()))
        .await
        .unwrap();
    assert_eq!(
        h.war_room(dan.now).await.unwrap()[0].state.mark.reason,
        None
    );
    h.mark_urgent(
        &dan,
        ItemType::Initiative,
        &k,
        Some("  Customer X  ".into()),
    )
    .await
    .unwrap();
    // wrong type and unknown key on clear
    assert!(matches!(
        h.clear_urgent(&dan, ItemType::Epic, &k).await.unwrap_err(),
        HandlerError::NotFound
    ));
    let ghost: Key = "ABC-9999".parse().unwrap();
    assert!(matches!(
        h.clear_urgent(&dan, ItemType::Initiative, &ghost)
            .await
            .unwrap_err(),
        HandlerError::NotFound
    ));
    let first = h.war_room(dan.now).await.unwrap();
    assert_eq!(first[0].state.mark.reason.as_deref(), Some("Customer X"));
    let later = Ctx {
        now: dan.now + chrono::Duration::days(3),
        ..dan.clone()
    };
    h.mark_urgent(&later, ItemType::Initiative, &k, None)
        .await
        .unwrap();
    assert_eq!(
        count(&sink, "urgent_marked", &k.to_string()),
        5,
        "every mark and renewal logs"
    );
    let room = h.war_room(later.now).await.unwrap();
    assert_eq!(room.len(), 1);
    assert_eq!(room[0].state.mark.at, later.now, "renew replaces the time");
    assert_eq!(room[0].state.mark.reason, None, "and the reason");
    assert!(matches!(
        h.clear_urgent(&bot, ItemType::Initiative, &k)
            .await
            .unwrap_err(),
        HandlerError::Refused(r) if r == vec![Reason::InteractiveOnly]
    ));
    h.clear_urgent(&dan, ItemType::Initiative, &k)
        .await
        .unwrap();
    h.clear_urgent(&dan, ItemType::Initiative, &k)
        .await
        .unwrap();
    assert_eq!(
        count(&sink, "urgent_cleared", &k.to_string()),
        1,
        "a second clear logs nothing"
    );
    assert!(h.war_room(later.now).await.unwrap().is_empty());
    assert_eq!(
        h.initiative_dto(&k, Utc::now()).await.unwrap().version,
        ini.version(),
        "no version bump"
    );
}

/// The war room lists active marks first, expired ones after, with the marked item's initiative.
#[tokio::test]
async fn the_war_room_lists_active_then_expired_marks() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let ini = h
        .create_initiative(&dan, "i1", "Orion", "", "Now", None, None, None)
        .await
        .unwrap();
    let epic = h
        .create_epic(&dan, "e1", "Orion v2", Some(ini.key().clone()), "", None)
        .await
        .unwrap();
    let story = h
        .create_story(
            &dan,
            "s1",
            "Booking flow",
            Kind::Story,
            Some(epic.key().clone()),
            "",
            vec!["works".into()],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    assert!(h.war_room(dan.now).await.unwrap().is_empty());
    h.mark_urgent(&dan, ItemType::Story, story.key(), Some("blocker".into()))
        .await
        .unwrap();
    let later = Ctx {
        now: dan.now + chrono::Duration::days(15),
        ..dan.clone()
    };
    h.mark_urgent(&later, ItemType::Initiative, ini.key(), None)
        .await
        .unwrap();
    // sprint length is 14 days: the story's mark is 15 days old, the initiative's is fresh.
    let room = h.war_room(later.now).await.unwrap();
    assert_eq!(room.len(), 2);
    assert_eq!(
        (room[0].item, &room[0].key),
        (ItemType::Initiative, ini.key())
    );
    assert!(room[0].state.active);
    assert_eq!(room[0].title, "Orion");
    assert_eq!(
        room[0].initiative,
        Some(WarRoomInitiative {
            key: ini.key().clone(),
            title: "Orion".into(),
            horizon: "Now".into()
        }),
        "an initiative is its own initiative"
    );
    assert_eq!((room[1].item, &room[1].key), (ItemType::Story, story.key()));
    assert!(!room[1].state.active, "expired sorts after active");
    assert_eq!(room[1].title, "Booking flow");
    assert_eq!(room[1].state.mark.reason.as_deref(), Some("blocker"));
    assert_eq!(
        room[1].initiative,
        Some(WarRoomInitiative {
            key: ini.key().clone(),
            title: "Orion".into(),
            horizon: "Now".into()
        }),
        "a nested item carries its initiative"
    );
    // at the mark's own time both are active, oldest first
    let early = h.war_room(dan.now).await.unwrap();
    assert!(early.iter().all(|e| e.state.active));
    assert_eq!(early[0].key, *story.key());
}

/// Bulk clear and renew touch exactly the listed items and roll back as a whole.
#[tokio::test]
async fn bulk_clear_and_renew_touch_exactly_the_listed_items() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let bot = ctx("dan@example.com", true);
    let mut keys = vec![];
    for n in 0..3 {
        let i = h
            .create_initiative(&dan, &format!("i{n}"), "Init", "", "Now", None, None, None)
            .await
            .unwrap();
        h.mark_urgent(
            &dan,
            ItemType::Initiative,
            i.key(),
            Some(format!("why {n}")),
        )
        .await
        .unwrap();
        keys.push(i.key().clone());
    }
    let (a, b, c) = (keys[0].clone(), keys[1].clone(), keys[2].clone());
    h.war_room_clear(
        &dan,
        &[
            (ItemType::Initiative, a.clone()),
            (ItemType::Initiative, b.clone()),
        ],
    )
    .await
    .unwrap();
    let room = h.war_room(dan.now).await.unwrap();
    assert_eq!(room.len(), 1, "only the listed marks are cleared");
    assert_eq!(room[0].key, c);
    assert_eq!(count(&sink, "urgent_cleared", &a.to_string()), 1);
    assert_eq!(count(&sink, "urgent_cleared", &b.to_string()), 1);
    assert_eq!(count(&sink, "urgent_cleared", &c.to_string()), 0);
    // a wrong type fails the whole request: nothing is cleared, nothing is logged
    assert!(matches!(
        h.war_room_clear(
            &dan,
            &[
                (ItemType::Initiative, c.clone()),
                (ItemType::Epic, c.clone())
            ]
        )
        .await
        .unwrap_err(),
        HandlerError::Validation(_)
    ));
    // an unknown key fails it too
    let ghost: Key = "ABC-9999".parse().unwrap();
    assert!(matches!(
        h.war_room_clear(
            &dan,
            &[
                (ItemType::Initiative, c.clone()),
                (ItemType::Initiative, ghost)
            ]
        )
        .await
        .unwrap_err(),
        HandlerError::Validation(_)
    ));
    assert_eq!(h.war_room(dan.now).await.unwrap().len(), 1, "rolled back");
    assert_eq!(count(&sink, "urgent_cleared", &c.to_string()), 0);
    // renew keeps the reason and replaces the time; an unmarked listed item is skipped
    let later = Ctx {
        now: dan.now + chrono::Duration::days(20),
        ..dan.clone()
    };
    h.war_room_renew(
        &later,
        &[
            (ItemType::Initiative, c.clone()),
            (ItemType::Initiative, a.clone()),
        ],
    )
    .await
    .unwrap();
    let room = h.war_room(later.now).await.unwrap();
    assert_eq!(room.len(), 1, "the unmarked item stays unmarked");
    assert_eq!(room[0].state.mark.at, later.now);
    assert_eq!(room[0].state.mark.reason.as_deref(), Some("why 2"));
    assert!(room[0].state.active);
    assert_eq!(count(&sink, "urgent_marked", &c.to_string()), 2);
    assert_eq!(
        count(&sink, "urgent_marked", &a.to_string()),
        1,
        "a was not renewed"
    );
    // limits and roles
    let many: Vec<(ItemType, Key)> = (0..101)
        .map(|_| (ItemType::Initiative, c.clone()))
        .collect();
    assert!(matches!(
        h.war_room_clear(&dan, &many).await.unwrap_err(),
        HandlerError::Validation(_)
    ));
    for r in [
        h.war_room_clear(&bot, &[(ItemType::Initiative, c.clone())])
            .await,
        h.war_room_renew(&bot, &[(ItemType::Initiative, c.clone())])
            .await,
    ] {
        assert!(matches!(
            r.unwrap_err(),
            HandlerError::Refused(r) if r == vec![Reason::InteractiveOnly]
        ));
    }
    assert_eq!(h.war_room(later.now).await.unwrap().len(), 1);
}

/// The entry is written last and inside the mark's transaction (kb/ADR-0010 R5).
#[tokio::test]
async fn an_urgent_entry_sees_the_mark_inside_and_not_outside() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk_probe(
        pool,
        "SELECT (SELECT urgent_by FROM initiatives WHERE key = $1)",
    );
    let dan = ctx("dan@example.com", false);
    let ini = h
        .create_initiative(&dan, "i", "Init", "", "Now", None, None, None)
        .await
        .unwrap();
    h.mark_urgent(&dan, ItemType::Initiative, ini.key(), None)
        .await
        .unwrap();
    let seen = sink.entries("urgent_marked");
    assert_eq!(seen.len(), 1);
    assert_eq!(
        (seen[0].1.as_deref(), seen[0].2.as_deref()),
        (Some("dan@example.com"), None)
    );
    h.clear_urgent(&dan, ItemType::Initiative, ini.key())
        .await
        .unwrap();
    let seen = sink.entries("urgent_cleared");
    assert_eq!(seen.len(), 1);
    assert_eq!(
        (seen[0].1.as_deref(), seen[0].2.as_deref()),
        (None, Some("dan@example.com")),
        "the clear is made before the entry and visible outside only after commit"
    );
}

/// A repeated item in a bulk request writes one entry; the renewing write skips a row that is no longer marked.
#[tokio::test]
async fn renew_dedupes_and_never_revives_a_cleared_mark() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);
    let ini = h
        .create_initiative(&dan, "i", "Init", "", "Now", None, None, None)
        .await
        .unwrap();
    let k = ini.key().clone();
    h.mark_urgent(&dan, ItemType::Initiative, &k, None)
        .await
        .unwrap();
    let twice = [
        (ItemType::Initiative, k.clone()),
        (ItemType::Initiative, k.clone()),
    ];
    h.war_room_renew(&dan, &twice).await.unwrap();
    assert_eq!(
        count(&sink, "urgent_marked", &k.to_string()),
        2,
        "mark + one renewal"
    );
    h.war_room_clear(&dan, &twice).await.unwrap();
    assert_eq!(count(&sink, "urgent_cleared", &k.to_string()), 1);
    // the repo variant on an unmarked row: None, and no change
    let mut tx = pool.begin().await.unwrap();
    assert_eq!(
        h.ports
            .urgency
            .renew_urgent_mark(
                &mut tx,
                ItemType::Initiative,
                &k,
                dan.now,
                "dan@example.com"
            )
            .await
            .unwrap(),
        None
    );
    assert!(
        h.ports
            .urgency
            .urgent_marks_in(&mut tx)
            .await
            .unwrap()
            .is_empty()
    );
    tx.commit().await.unwrap();
    // and a renewal of an unmarked item through the handler writes nothing
    h.war_room_renew(&dan, &twice[..1]).await.unwrap();
    assert_eq!(count(&sink, "urgent_marked", &k.to_string()), 2);
}

/// A renewal keeps the reason the row holds when it writes, never one it read earlier: a reason committed while the
/// renewal waits on the row survives it, and the `urgent_marked` entry carries that reason.
#[tokio::test]
async fn renew_keeps_a_reason_it_never_read() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);
    let ini = h
        .create_initiative(&dan, "i", "Init", "", "Now", None, None, None)
        .await
        .unwrap();
    let k = ini.key().clone();
    h.mark_urgent(&dan, ItemType::Initiative, &k, Some("old".into()))
        .await
        .unwrap();
    // Another writer holds the row with a new reason while the renewal starts.
    let mut w = pool.begin().await.unwrap();
    sqlx::query("UPDATE initiatives SET urgent_reason = 'fresh' WHERE key = $1")
        .bind(k.to_string())
        .execute(&mut *w)
        .await
        .unwrap();
    let later = Ctx {
        now: dan.now + chrono::Duration::days(20),
        ..dan.clone()
    };
    let renew = tokio::spawn({
        let (h, k, later) = (h.clone(), k.clone(), later.clone());
        async move { h.war_room_renew(&later, &[(ItemType::Initiative, k)]).await }
    });
    wait_until_blocked(&pool).await;
    w.commit().await.unwrap();
    renew.await.unwrap().unwrap();
    let room = h.war_room(later.now).await.unwrap();
    assert_eq!(room[0].state.mark.at, later.now, "renewed");
    assert_eq!(room[0].state.mark.reason.as_deref(), Some("fresh"));
    let entries: Vec<serde_json::Value> = sink
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|a| a.action == "urgent_marked" && a.entity_key == k.to_string())
        .map(|a| a.payload.clone())
        .collect();
    assert_eq!(entries.len(), 2, "mark + renewal");
    assert_eq!(entries[1]["reason"], "fresh");
}

fn source_keys(v: &[reeve_protocol::ItemRefDto]) -> Vec<String> {
    v.iter().map(|r| r.key.to_string()).collect()
}

/// Every assembler fills urgency the same way (endpoint parity, handler level).
#[tokio::test]
async fn every_item_dto_carries_its_urgency() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);
    let i = h
        .create_initiative(&dan, "i", "I", "", "Now", None, None, None)
        .await
        .unwrap();
    let e = h
        .create_epic(&dan, "e", "E", Some(i.key().clone()), "", None)
        .await
        .unwrap();
    let s = h
        .create_story(
            &dan,
            "s",
            "S",
            Kind::Story,
            Some(e.key().clone()),
            "",
            vec![],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let sib = h
        .create_story(
            &dan,
            "t",
            "T",
            Kind::Story,
            Some(e.key().clone()),
            "",
            vec![],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    h.mark_urgent(&dan, ItemType::Story, s.key(), Some("why".into()))
        .await
        .unwrap();
    let now = dan.now;
    let sk = vec![s.key().to_string()];
    assert_eq!(
        source_keys(&h.initiative_dto(i.key(), now).await.unwrap().urgent_sources),
        sk
    );
    assert_eq!(
        source_keys(&h.list_initiatives(now).await.unwrap()[0].urgent_sources),
        sk
    );
    assert_eq!(
        source_keys(&h.epic_dto(e.key(), now).await.unwrap().urgent_sources),
        sk
    );
    assert_eq!(
        source_keys(&h.epic_dtos(None, now).await.unwrap()[0].urgent_sources),
        sk
    );
    assert_eq!(
        source_keys(&h.epic_dtos(Some(i.key()), now).await.unwrap()[0].urgent_sources),
        sk
    );
    let board = h
        .list_stories(
            &StoryFilter::default(),
            None,
            None,
            false,
            DoneFilter::All,
            now,
        )
        .await
        .unwrap();
    let row = |k: &Key| {
        board
            .iter()
            .find(|r| &r.story.key == k)
            .unwrap()
            .story
            .clone()
    };
    assert_eq!(source_keys(&row(s.key()).urgent_sources), sk);
    assert!(
        row(sib.key()).urgent_sources.is_empty(),
        "a sibling stays calm"
    );
    assert!(row(sib.key()).urgent_mark.is_none());
    assert_eq!(
        row(s.key()).urgent_mark.unwrap().reason.as_deref(),
        Some("why")
    );
    let ctxd = h.story_context(sib.key(), now).await.unwrap();
    assert_eq!(
        source_keys(&ctxd.epic.unwrap().urgent_sources),
        sk,
        "nested epic in the context"
    );
    assert_eq!(
        source_keys(&ctxd.initiative.unwrap().urgent_sources),
        sk,
        "nested initiative in the context"
    );
    assert!(
        ctxd.story.urgent_sources.is_empty(),
        "the context's own story"
    );
    let ctxs = h.story_context(s.key(), now).await.unwrap();
    assert_eq!(source_keys(&ctxs.story.urgent_sources), sk);
    let dto = h.story_dto(s.key(), now).await.unwrap();
    assert_eq!(source_keys(&dto.urgent_sources), sk);
    assert_eq!(dto.urgent_mark.unwrap().reason.as_deref(), Some("why"));
    // The export readers, on one transaction, with the urgency computed once on it.
    let mut tx = pool
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .unwrap();
    let urg = h.urgency_in(&mut tx, now).await.unwrap();
    let clock = h.promise_clock_in(&mut tx, now).await.unwrap();
    let ex_i = h.export_initiatives(&mut tx, &urg).await.unwrap();
    let ex_e = h.export_epics(&mut tx, &urg, &clock).await.unwrap();
    let ex_s = h.export_stories(&mut tx, &urg, &clock).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(source_keys(&ex_i[0].urgent_sources), sk);
    assert_eq!(source_keys(&ex_e[0].urgent_sources), sk);
    let ex_row = |k: &Key| ex_s.iter().find(|x| &x.key == k).unwrap().clone();
    assert_eq!(source_keys(&ex_row(s.key()).urgent_sources), sk);
    assert!(ex_row(s.key()).urgent_mark.is_some());
    assert!(ex_row(sib.key()).urgent_sources.is_empty());
    // An expired mark is still shown on its item but makes nothing urgent.
    let later = now + chrono::Duration::days(15);
    let dto = h.story_dto(s.key(), later).await.unwrap();
    assert!(dto.urgent_sources.is_empty());
    assert!(!dto.urgent_mark.unwrap().active);
    assert!(
        h.initiative_dto(i.key(), later)
            .await
            .unwrap()
            .urgent_sources
            .is_empty()
    );
}

/// A mark made between someone's read and their edit or move does not make the edit fail, and the edit keeps it.
#[tokio::test]
async fn a_mark_never_conflicts_with_an_edit_or_a_move_and_survives_both() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let now = dan.now;
    let epic = ready_epic(&h, &dan).await;
    // A story: read, then marked by someone else, then edited and moved with the version that was read.
    let s = h
        .create_story(
            &dan,
            "s",
            "S",
            Kind::Chore,
            Some(epic),
            "",
            vec!["works".into()],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let read = h.get_story(s.key()).await.unwrap().version();
    h.mark_urgent(&dan, ItemType::Story, s.key(), Some("hot".into()))
        .await
        .unwrap();
    let edited = h
        .edit_story(
            &dan,
            s.key(),
            read,
            Some("S2".into()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("the mark did not bump the version");
    let moved = h
        .move_story(
            &dan,
            s.key(),
            edited.version(),
            DeliveryState::InProgress,
            None,
            None,
        )
        .await
        .expect("a mark never refuses a move");
    assert_eq!(moved.delivery(), DeliveryState::InProgress);
    let dto = h.story_dto(s.key(), now).await.unwrap();
    assert_eq!(dto.title, "S2");
    assert_eq!(dto.urgent_mark.unwrap().reason.as_deref(), Some("hot"));
    assert_eq!(source_keys(&dto.urgent_sources), [s.key().to_string()]);

    // An initiative and an epic: marked after the read; an edit and a discovery mark keep it.
    let i = h
        .create_initiative(&dan, "i", "I", "", "Now", None, None, None)
        .await
        .unwrap();
    let e = h
        .create_epic(&dan, "e", "E", Some(i.key().clone()), "", None)
        .await
        .unwrap();
    h.mark_urgent(&dan, ItemType::Initiative, i.key(), Some("cust".into()))
        .await
        .unwrap();
    h.mark_urgent(&dan, ItemType::Epic, e.key(), None)
        .await
        .unwrap();
    let i2 = h
        .edit_initiative(
            &dan,
            i.key(),
            i.version(),
            Some("I2".into()),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("the mark did not bump the initiative's version");
    h.mark_discovery(&dan, ItemType::Initiative, i.key(), i2.version(), true)
        .await
        .unwrap();
    let e2 = h
        .edit_epic(
            &dan,
            e.key(),
            e.version(),
            Some("E2".into()),
            None,
            None,
            None,
        )
        .await
        .expect("the mark did not bump the epic's version");
    h.mark_discovery(&dan, ItemType::Epic, e.key(), e2.version(), true)
        .await
        .unwrap();
    let idto = h.initiative_dto(i.key(), now).await.unwrap();
    assert_eq!(idto.title, "I2");
    assert!(idto.discovery_complete.is_some());
    assert_eq!(idto.urgent_mark.unwrap().reason.as_deref(), Some("cust"));
    assert_eq!(
        source_keys(&idto.urgent_sources),
        [i.key().to_string(), e.key().to_string()]
    );
    let edto = h.epic_dto(e.key(), now).await.unwrap();
    assert_eq!(edto.title, "E2");
    assert!(edto.discovery_complete.is_some());
    assert!(edto.urgent_mark.is_some());
    assert_eq!(
        source_keys(&edto.urgent_sources),
        [e.key().to_string(), i.key().to_string()]
    );
}

/// Urgency never feeds readiness, delivery or progress (kb/ADR-0017 R5).
#[tokio::test]
async fn urgency_changes_no_readiness_delivery_or_progress() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let now = dan.now;
    let i = h
        .create_initiative(&dan, "i", "I", "", "Now", None, None, None)
        .await
        .unwrap();
    let e = h
        .create_epic(&dan, "e", "E", Some(i.key().clone()), "", None)
        .await
        .unwrap();
    h.mark_discovery(&dan, ItemType::Initiative, i.key(), 1, true)
        .await
        .unwrap();
    h.mark_discovery(&dan, ItemType::Epic, e.key(), 1, true)
        .await
        .unwrap();
    let mut keys = Vec::new();
    for (n, criteria) in [
        ("ready", vec!["ok".to_string()]),
        ("bare", vec![]),
        ("done", vec!["ok".to_string()]),
    ] {
        keys.push(
            h.create_story(
                &dan,
                n,
                n,
                Kind::Story,
                Some(e.key().clone()),
                "",
                criteria,
                vec![],
                None,
                None,
            )
            .await
            .unwrap()
            .key()
            .clone(),
        );
    }
    let override_reason = Some("ship it".to_string());
    h.move_story(
        &dan,
        &keys[2],
        1,
        DeliveryState::InProgress,
        None,
        override_reason,
    )
    .await
    .unwrap();
    h.move_story(
        &dan,
        &keys[2],
        2,
        DeliveryState::Done,
        Some(Resolution::Completed),
        None,
    )
    .await
    .unwrap();
    type Facts = (
        Vec<(
            Key,
            reeve_protocol::Readiness,
            DeliveryState,
            Option<Resolution>,
        )>,
        Vec<(Key, Option<f32>)>,
        Vec<reeve_protocol::Readiness>,
    );
    let facts = || async {
        let board = h
            .list_stories(
                &StoryFilter::default(),
                None,
                None,
                false,
                DoneFilter::All,
                now,
            )
            .await
            .unwrap();
        let stories = board
            .into_iter()
            .map(|r| {
                (
                    r.story.key,
                    r.readiness,
                    r.story.delivery,
                    r.story.resolution,
                )
            })
            .collect();
        let progress = h
            .list_initiatives(now)
            .await
            .unwrap()
            .into_iter()
            .map(|i| (i.key, i.progress))
            .collect();
        let mut one_at_a_time = Vec::new();
        for k in &keys {
            one_at_a_time.push(h.readiness(k).await.unwrap());
        }
        let f: Facts = (stories, progress, one_at_a_time);
        f
    };
    let before = facts().await;
    assert!(
        before.0.iter().any(|(_, r, ..)| r.ready),
        "one story is ready"
    );
    assert!(before.0.iter().any(|(_, r, ..)| !r.ready), "one is not");
    assert!(before.1[0].1.is_some(), "the initiative has progress");
    h.mark_urgent(&dan, ItemType::Initiative, i.key(), Some("now".into()))
        .await
        .unwrap();
    let marked = h.story_dto(&keys[0], now).await.unwrap();
    assert_eq!(
        source_keys(&marked.urgent_sources),
        [i.key().to_string()],
        "the mark is active and reaches the stories"
    );
    assert_eq!(facts().await, before, "marked");
    h.clear_urgent(&dan, ItemType::Initiative, i.key())
        .await
        .unwrap();
    assert_eq!(facts().await, before, "cleared");
}

/// Every item read takes one connection once, for its one `REPEATABLE READ` transaction: its rows and its urgency
/// are one snapshot. On a one-connection pool, a read through the pool while the snapshot is open could not get a
/// connection (and fails), and a read through the pool before or after it is a second acquisition.
#[tokio::test]
async fn every_item_read_is_one_transaction_on_one_connection() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);
    let i = h
        .create_initiative(&dan, "i", "I", "", "Now", None, None, None)
        .await
        .unwrap();
    let e = h
        .create_epic(&dan, "e", "E", Some(i.key().clone()), "", None)
        .await
        .unwrap();
    let s = h
        .create_story(
            &dan,
            "s",
            "S",
            Kind::Story,
            Some(e.key().clone()),
            "",
            vec![],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    h.mark_urgent(&dan, ItemType::Story, s.key(), None)
        .await
        .unwrap();

    let acquired = Arc::new(AtomicUsize::new(0));
    let one = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .min_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(3))
        .before_acquire({
            let acquired = acquired.clone();
            move |_, _| {
                acquired.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Ok(true) })
            }
        })
        .connect_with((*pool.connect_options()).clone())
        .await
        .unwrap();
    sqlx::query("SELECT 1").execute(&one).await.unwrap(); // the connection exists and is idle
    let h = Handlers {
        ports: make_planning_ports(one.clone()),
        pool: one,
        ..h
    };
    let now = dan.now;
    let sk = s.key().to_string();
    let once = || acquired.swap(0, Ordering::SeqCst);
    once();
    let urgent = h.initiative_dto(i.key(), now).await.unwrap().urgent_sources;
    assert_eq!(
        (once(), source_keys(&urgent)),
        (1, vec![sk.clone()]),
        "initiative_dto"
    );
    let urgent = h.list_initiatives(now).await.unwrap()[0]
        .urgent_sources
        .clone();
    assert_eq!(
        (once(), source_keys(&urgent)),
        (1, vec![sk.clone()]),
        "list_initiatives"
    );
    let urgent = h.epic_dto(e.key(), now).await.unwrap().urgent_sources;
    assert_eq!(
        (once(), source_keys(&urgent)),
        (1, vec![sk.clone()]),
        "epic_dto"
    );
    let urgent = h.epic_dtos(None, now).await.unwrap()[0]
        .urgent_sources
        .clone();
    assert_eq!(
        (once(), source_keys(&urgent)),
        (1, vec![sk.clone()]),
        "epic_dtos"
    );
    let urgent = h.story_dto(s.key(), now).await.unwrap().urgent_sources;
    assert_eq!(
        (once(), source_keys(&urgent)),
        (1, vec![sk.clone()]),
        "story_dto"
    );
    let urgent = h
        .list_stories(
            &StoryFilter::default(),
            None,
            None,
            false,
            DoneFilter::All,
            now,
        )
        .await
        .unwrap()[0]
        .story
        .urgent_sources
        .clone();
    assert_eq!(
        (once(), source_keys(&urgent)),
        (1, vec![sk.clone()]),
        "list_stories"
    );
    let urgent = h
        .story_context(s.key(), now)
        .await
        .unwrap()
        .initiative
        .unwrap()
        .urgent_sources;
    assert_eq!(
        (once(), source_keys(&urgent)),
        (1, vec![sk]),
        "story_context"
    );
}

/// A story with no epic and no criteria, created at `c.now`.
async fn plain_story(h: &Handlers, c: &Ctx, idem: &str, title: &str, epic: Option<&Key>) -> Key {
    h.create_story(
        c,
        idem,
        title,
        Kind::Story,
        epic.cloned(),
        "",
        vec![],
        vec![],
        None,
        None,
    )
    .await
    .unwrap()
    .key()
    .clone()
}

/// The keys of the story list for `f`, `text` and `initiative`, every done story included, in order.
async fn list_keys(
    h: &Handlers,
    f: &StoryFilter,
    text: Option<&str>,
    initiative: Option<&Key>,
    now: chrono::DateTime<Utc>,
) -> Vec<Key> {
    h.list_stories(f, text, initiative, false, DoneFilter::All, now)
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.story.key)
        .collect()
}

/// Text is 1..=200 characters after trimming, counted as characters, not bytes; relevance needs text. Validation
/// runs before the empty-result shortcut of an epic set that names nothing.
#[tokio::test]
async fn story_list_validates_text_and_sort() {
    let (_c, pool) = common::pg().await;
    let (h, _) = mk(pool);
    let c = ctx("dan@example.com", false);
    let s = plain_story(&h, &c, "s", &"é".repeat(200), None).await;
    let refused = |f: StoryFilter, text: Option<String>| {
        let h = &h;
        async move {
            let r = h
                .list_stories(&f, text.as_deref(), None, false, DoneFilter::All, c.now)
                .await;
            assert!(
                matches!(r, Err(HandlerError::Validation(_))),
                "{f:?} {text:?}: {r:?}"
            );
        }
    };
    let none = StoryFilter::default;
    refused(none(), Some("".into())).await;
    refused(none(), Some("   ".into())).await;
    refused(none(), Some("a".repeat(201))).await;
    refused(
        StoryFilter {
            epics: Some(vec![]),
            ..none()
        },
        Some(" ".into()),
    )
    .await;
    refused(
        StoryFilter {
            sort: reeve_protocol::StorySort::Relevance,
            ..none()
        },
        None,
    )
    .await;
    refused(
        StoryFilter {
            sort: reeve_protocol::StorySort::Relevance,
            epics: Some(vec![]),
            ..none()
        },
        None,
    )
    .await;
    assert_eq!(
        list_keys(&h, &none(), Some(&"é".repeat(200)), None, c.now).await,
        vec![s.clone()],
        "200 two-byte characters are within the limit"
    );
    assert_eq!(
        list_keys(
            &h,
            &StoryFilter {
                sort: reeve_protocol::StorySort::Relevance,
                ..none()
            },
            Some(&format!("  {}  ", "é".repeat(200))),
            None,
            c.now
        )
        .await,
        vec![s],
        "the text is trimmed before it is counted and matched"
    );
}

/// The handler derives the key from the text it is given and replaces whatever `f.text` held: a stale key there never
/// matches (text `zzz` lists nothing though the stale key names a story), and with no text there is no text filter.
#[tokio::test]
async fn story_list_ignores_a_callers_story_text() {
    let (_c, pool) = common::pg().await;
    let (h, _) = mk(pool);
    let c = ctx("dan@example.com", false);
    let one = plain_story(&h, &c, "1", "First", None).await;
    let two = plain_story(&h, &c, "2", "Second", None).await;
    let stale = StoryFilter {
        text: Some(StoryText {
            text: "zzz".into(),
            key: Some(one.clone()),
        }),
        ..Default::default()
    };
    assert_eq!(
        list_keys(&h, &stale, Some("zzz"), None, c.now).await,
        Vec::<Key>::new(),
        "zzz names no key, whatever f.text said"
    );
    assert_eq!(
        list_keys(&h, &stale, None, None, c.now).await,
        vec![one, two],
        "no text, no text filter"
    );
}

/// An initiative keeps the stories of its epics; an initiative with no epics keeps none.
#[tokio::test]
async fn story_list_initiative_expands_to_its_epics() {
    let (_c, pool) = common::pg().await;
    let (h, _) = mk(pool);
    let c = ctx("dan@example.com", false);
    let i = h
        .create_initiative(&c, "i", "I", "", "Now", None, None, None)
        .await
        .unwrap();
    let j = h
        .create_initiative(&c, "j", "J", "", "Now", None, None, None)
        .await
        .unwrap();
    let empty = h
        .create_initiative(&c, "k", "K", "", "Now", None, None, None)
        .await
        .unwrap();
    let e1 = h
        .create_epic(&c, "e1", "E1", Some(i.key().clone()), "", None)
        .await
        .unwrap();
    let e2 = h
        .create_epic(&c, "e2", "E2", Some(i.key().clone()), "", None)
        .await
        .unwrap();
    let e3 = h
        .create_epic(&c, "e3", "E3", Some(j.key().clone()), "", None)
        .await
        .unwrap();
    let s1 = plain_story(&h, &c, "s1", "S1", Some(e1.key())).await;
    let s2 = plain_story(&h, &c, "s2", "S2", Some(e2.key())).await;
    let s3 = plain_story(&h, &c, "s3", "S3", Some(e3.key())).await;
    let loose = plain_story(&h, &c, "s4", "S4", None).await;
    let f = StoryFilter::default();
    let got = list_keys(&h, &f, None, Some(i.key()), c.now).await;
    assert_eq!(got, vec![s1.clone(), s2], "not {s3} nor {loose}");
    assert_eq!(
        list_keys(&h, &f, None, Some(empty.key()), c.now).await,
        Vec::<Key>::new()
    );
    // With an epic set already given, the initiative narrows it.
    assert_eq!(
        list_keys(
            &h,
            &StoryFilter {
                epics: Some(vec![e1.key().clone(), e3.key().clone()]),
                ..Default::default()
            },
            None,
            Some(i.key()),
            c.now
        )
        .await,
        vec![s1]
    );
}

/// `urgent` keeps exactly the rows shown urgent: a marked initiative's descendants and a directly marked story, not
/// its unmarked sibling, not a done story with a mark, not a mark past its cycle or exactly at its end.
#[tokio::test]
async fn story_list_urgent_selects_the_rows_shown_urgent() {
    let (_c, pool) = common::pg().await;
    let (h, _) = mk(pool);
    let c = ctx("dan@example.com", false);
    let now = c.now;
    let cycle = chrono::Duration::days(settings().sprint_length_days.into());
    let at = |t: chrono::DateTime<Utc>| Ctx {
        now: t,
        ..c.clone()
    };
    let i = h
        .create_initiative(&c, "i", "I", "", "Now", None, None, None)
        .await
        .unwrap();
    let ei = h
        .create_epic(&c, "ei", "EI", Some(i.key().clone()), "", None)
        .await
        .unwrap();
    let e2 = h.create_epic(&c, "e2", "E2", None, "", None).await.unwrap();
    let a1 = plain_story(&h, &c, "a1", "A1", Some(ei.key())).await;
    let a2 = plain_story(&h, &c, "a2", "A2", Some(ei.key())).await;
    let marked = plain_story(&h, &c, "m", "Marked", Some(e2.key())).await;
    let sibling = plain_story(&h, &c, "sib", "Sibling", Some(e2.key())).await;
    let done = plain_story(&h, &c, "d", "Done", None).await;
    let expired = plain_story(&h, &c, "x", "Expired", None).await;
    let boundary = plain_story(&h, &c, "b", "Boundary", None).await;
    let calm = plain_story(&h, &c, "p", "Calm", None).await;
    h.mark_urgent(&c, ItemType::Initiative, i.key(), None)
        .await
        .unwrap();
    h.mark_urgent(&c, ItemType::Story, &marked, None)
        .await
        .unwrap();
    h.mark_urgent(&c, ItemType::Story, &done, None)
        .await
        .unwrap();
    h.mark_urgent(
        &at(now - cycle - chrono::Duration::seconds(1)),
        ItemType::Story,
        &expired,
        None,
    )
    .await
    .unwrap();
    h.mark_urgent(&at(now - cycle), ItemType::Story, &boundary, None)
        .await
        .unwrap();
    let v = h.ports.stories.get_story(&done).await.unwrap().version();
    h.move_story(
        &c,
        &done,
        v,
        DeliveryState::Done,
        Some(Resolution::Dropped),
        None,
    )
    .await
    .unwrap();

    let urgent = StoryFilter {
        urgent: true,
        ..Default::default()
    };
    let got = list_keys(&h, &urgent, None, None, now).await;
    let expected = vec![a1, a2, marked];
    assert_eq!(
        got, expected,
        "not {sibling}, {done}, {expired}, {boundary} nor {calm}"
    );
    let shown_urgent: Vec<Key> = h
        .list_stories(
            &StoryFilter::default(),
            None,
            None,
            false,
            DoneFilter::All,
            now,
        )
        .await
        .unwrap()
        .into_iter()
        .filter(|s| !s.story.urgent_sources.is_empty())
        .map(|s| s.story.key)
        .collect();
    assert_eq!(
        got, shown_urgent,
        "the filter agrees with the rows the response shows urgent"
    );
}

/// A bare number or `#n` names story n, which the list finds though no field of it contains the text.
#[tokio::test]
async fn story_list_text_names_a_key() {
    let (_c, pool) = common::pg().await;
    let (h, _) = mk(pool);
    let c = ctx("dan@example.com", false);
    let _one = plain_story(&h, &c, "1", "First", None).await;
    let _two = plain_story(&h, &c, "2", "Second", None).await;
    let three = plain_story(&h, &c, "3", "Third", None).await;
    assert_eq!(three.to_string(), "ABC-3");
    for t in ["#3", "3", "abc-3"] {
        assert_eq!(
            list_keys(&h, &StoryFilter::default(), Some(t), None, c.now).await,
            vec![three.clone()],
            "{t}"
        );
    }
}

/// 12 Nov 10:00Z: the fixed request time of the promise tests.
fn promise_ctx() -> Ctx {
    Ctx {
        now: Utc.with_ymd_and_hms(2026, 11, 12, 10, 0, 0).unwrap(),
        ..ctx("dan@example.com", false)
    }
}

fn nov(d: u32) -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
}

/// An active sprint planned to end 24 Nov and a draft planned to end 8 Dec, made through the sprint handler.
async fn promise_sprints(h: &Handlers, c: &Ctx) {
    h.add_goal(c, "promise-g", "outcome", "check", None, "", false, &[])
        .await
        .unwrap();
    let d = h.sprints(c.now).await.unwrap().draft.unwrap();
    h.open_sprint(c, d.id, d.version, Some(nov(24)))
        .await
        .unwrap();
    let updated = sqlx::query("UPDATE sprints SET ends_on = '2026-12-08' WHERE state = 'draft'")
        .execute(&h.pool)
        .await
        .unwrap();
    assert_eq!(updated.rows_affected(), 1, "opening left exactly one draft");
}

async fn set_promise(h: &Handlers, item: ItemType, key: &Key, p: &Promise) {
    let mut tx = h.pool.begin().await.unwrap();
    assert!(
        h.ports
            .promises
            .set_promise(
                &mut tx,
                reeve_planning::PromiseItem::of(item).unwrap(),
                key,
                p
            )
            .await
            .unwrap()
    );
    tx.commit().await.unwrap();
}

async fn mark_done_at(h: &Handlers, key: &Key, at: chrono::DateTime<Utc>) {
    sqlx::query(
        "UPDATE stories SET delivery = 'done', resolution = 'completed', done_at = $2 WHERE key = $1",
    )
    .bind(key.to_string())
    .bind(at)
    .execute(&h.pool)
    .await
    .unwrap();
}

/// Every story and epic reader fills `promise` through the one rule: A shows its epic's earlier date with its own still
/// visible; B, done after the epic's promise was set, inherits it and kept it; C, done the same day but before the
/// epic's promise was set, has none (inheritance is keyed by `promised_at`, not its date). The epic shows its own
/// promise with its due period and no state. Every reader agrees.
#[tokio::test]
async fn every_item_dto_carries_its_promise() {
    use reeve_protocol::{PromiseDue, PromiseHolder, PromiseState};
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let c = promise_ctx();
    let now = c.now;
    promise_sprints(&h, &c).await;
    let e = h.create_epic(&c, "e", "E", None, "", None).await.unwrap();
    let a = plain_story(&h, &c, "A", "A", Some(e.key())).await;
    let b = plain_story(&h, &c, "B", "B", Some(e.key())).await;
    let cc = plain_story(&h, &c, "C", "C", Some(e.key())).await;
    let epic_p = Promise::from_persisted(
        nov(18),
        "lead@x".into(),
        Utc.with_ymd_and_hms(2026, 11, 12, 9, 0, 0).unwrap(),
        None,
    );
    set_promise(&h, ItemType::Epic, e.key(), &epic_p).await;
    let own_p = Promise::from_persisted(nov(30), "pm@x".into(), now, Some("Beta".into()));
    set_promise(&h, ItemType::Story, &a, &own_p).await;
    mark_done_at(
        &h,
        &b,
        Utc.with_ymd_and_hms(2026, 11, 12, 9, 30, 0).unwrap(),
    )
    .await;
    mark_done_at(
        &h,
        &cc,
        Utc.with_ymd_and_hms(2026, 11, 12, 8, 0, 0).unwrap(),
    )
    .await;

    let a_dto = h.story_dto(&a, now).await.unwrap();
    let ap = a_dto.promise.clone().expect("A is promised");
    assert_eq!(
        (ap.on, ap.holder, ap.state, ap.due, ap.days),
        (
            nov(18),
            PromiseHolder::Epic,
            Some(PromiseState::Open),
            Some(PromiseDue::ThisSprint),
            None
        )
    );
    let own = ap.own.as_ref().expect("A's own promise stays visible");
    assert_eq!(
        (own.on, own.by.as_str(), own.to.as_deref()),
        (nov(30), "pm@x", Some("Beta"))
    );
    let inherited = ap.epic.as_ref().expect("A inherits its epic's promise");
    assert_eq!((&inherited.key, inherited.fact.on), (e.key(), nov(18)));
    let b_dto = h.story_dto(&b, now).await.unwrap();
    let bp = b_dto.promise.clone().expect("B inherits");
    assert_eq!(
        (bp.holder, bp.state, bp.on),
        (PromiseHolder::Epic, Some(PromiseState::Kept), nov(18))
    );
    let c_dto = h.story_dto(&cc, now).await.unwrap();
    assert_eq!(
        c_dto.promise, None,
        "done before the epic's promise was set"
    );

    let ep = h
        .epic_dto(e.key(), now)
        .await
        .unwrap()
        .promise
        .expect("the epic's promise");
    assert_eq!(
        (ep.fact.on, ep.fact.by.as_str(), ep.due, ep.past),
        (nov(18), "lead@x", Some(PromiseDue::ThisSprint), false)
    );

    // Every reader agrees.
    let list = h
        .list_stories(
            &StoryFilter::default(),
            None,
            None,
            false,
            DoneFilter::All,
            now,
        )
        .await
        .unwrap();
    let row = |k: &Key| {
        list.iter()
            .find(|r| &r.story.key == k)
            .unwrap()
            .story
            .promise
            .clone()
    };
    assert_eq!(row(&a), a_dto.promise);
    assert_eq!(row(&b), b_dto.promise);
    assert_eq!(row(&cc), None);
    let ctxd = h.story_context(&a, now).await.unwrap();
    assert_eq!(ctxd.story.promise, a_dto.promise);
    assert_eq!(ctxd.epic.unwrap().promise, Some(ep.clone()));
    assert_eq!(
        h.epic_dtos(None, now).await.unwrap()[0].promise,
        Some(ep.clone())
    );
    // The export readers, on one transaction, with the clock computed once on it.
    let mut tx = pool
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .unwrap();
    let urg = h.urgency_in(&mut tx, now).await.unwrap();
    let clock = h.promise_clock_in(&mut tx, now).await.unwrap();
    let ex_e = h.export_epics(&mut tx, &urg, &clock).await.unwrap();
    let ex_s = h.export_stories(&mut tx, &urg, &clock).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(ex_e[0].promise, Some(ep));
    let ex_row = |k: &Key| ex_s.iter().find(|x| &x.key == k).unwrap().promise.clone();
    assert_eq!(ex_row(&a), a_dto.promise);
    assert_eq!(ex_row(&b), b_dto.promise);
    assert_eq!(ex_row(&cc), None);
}

/// The promise fields read one snapshot: the clock (the sprints' planned ends) and the promise facts are read on the
/// list's own transaction. The call takes its snapshot on its first statement, then waits on the `sprints` lock; the
/// lock's holder moves the active sprint's end (A from this sprint to next) and A's own date, and commits as it
/// releases the lock. A clock or promise read on another connection, or in a second snapshot, would see the new values.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_promise_fields_read_one_snapshot() {
    use reeve_protocol::PromiseDue;
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let c = promise_ctx();
    let now = c.now;
    promise_sprints(&h, &c).await;
    let a = plain_story(&h, &c, "A", "A", None).await;
    set_promise(
        &h,
        ItemType::Story,
        &a,
        &Promise::from_persisted(nov(20), "pm@x".into(), now, None),
    )
    .await;
    let list = |h: Handlers| async move {
        h.list_stories(
            &StoryFilter::default(),
            None,
            None,
            false,
            DoneFilter::All,
            now,
        )
        .await
        .unwrap()
        .pop()
        .unwrap()
        .story
        .promise
        .unwrap()
    };

    let mut lock = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE sprints IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let call = tokio::spawn(list(h.clone()));
    wait_until_blocked(&pool).await;
    sqlx::query("UPDATE sprints SET ends_on = '2026-11-19' WHERE state = 'active'")
        .execute(&mut *lock)
        .await
        .unwrap();
    sqlx::query("UPDATE stories SET promised_on = '2026-11-22' WHERE key = $1")
        .bind(a.to_string())
        .execute(&mut *lock)
        .await
        .unwrap();
    lock.commit().await.unwrap();
    let before = tokio::time::timeout(PAUSE, call)
        .await
        .expect("the list never finished")
        .unwrap();
    assert_eq!(
        (before.on, before.due, before.own.unwrap().on),
        (nov(20), Some(PromiseDue::ThisSprint), nov(20))
    );

    let after = list(h.clone()).await;
    assert_eq!(
        (after.on, after.due, after.own.unwrap().on),
        (nov(22), Some(PromiseDue::NextSprint), nov(22))
    );
}

/// Writes each entry into `test_activity` on the caller's transaction (model: `tests/attachment_handlers.rs`), so the
/// assertions read what was persisted with the write, and a rolled-back write leaves no row.
struct DbSink;
#[async_trait]
impl ActivitySink for DbSink {
    async fn record(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        rec: ActivityRecord,
    ) -> Result<(), RepoError> {
        insert_test_activity(tx, &rec).await
    }
}

async fn insert_test_activity(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    rec: &ActivityRecord,
) -> Result<(), RepoError> {
    sqlx::query(
        "INSERT INTO test_activity (entity_type, entity_key, action, actor, payload) VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(rec.entity_type)
    .bind(&rec.entity_key)
    .bind(rec.action)
    .bind(&rec.actor)
    .bind(&rec.payload)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Handlers whose entries land in `test_activity` (created here).
async fn mk_db(pool: sqlx::PgPool) -> Handlers {
    sqlx::query("CREATE TABLE test_activity (id BIGSERIAL PRIMARY KEY, entity_type TEXT, entity_key TEXT, action TEXT, actor TEXT, payload JSONB)")
        .execute(&pool)
        .await
        .unwrap();
    Handlers {
        ports: make_planning_ports(pool.clone()),
        activity: Arc::new(DbSink),
        pool,
        settings: settings(),
    }
}

/// The persisted promise entries of one item, in order: (entity_type, action, payload).
async fn promise_entries(
    pool: &sqlx::PgPool,
    key: &Key,
) -> Vec<(String, String, serde_json::Value)> {
    sqlx::query_as(
        "SELECT entity_type, action, payload FROM test_activity WHERE entity_key = $1 AND action IN ('promised', 'promise_cleared') ORDER BY id",
    )
    .bind(key.to_string())
    .fetch_all(pool)
    .await
    .unwrap()
}

fn promise_table(item: ItemType) -> &'static str {
    match item {
        ItemType::Story => "stories",
        ItemType::Epic => "epics",
        ItemType::Initiative => unreachable!("never promised"),
    }
}

type Stored = (
    chrono::NaiveDate,
    String,
    chrono::DateTime<Utc>,
    Option<String>,
);
/// The stored promise columns, read fresh from the table.
async fn stored(pool: &sqlx::PgPool, item: ItemType, key: &Key) -> Option<Stored> {
    let (on, by, at, to): (
        Option<chrono::NaiveDate>,
        Option<String>,
        Option<chrono::DateTime<Utc>>,
        Option<String>,
    ) = sqlx::query_as(&format!(
        "SELECT promised_on, promised_by, promised_at, promised_to FROM {} WHERE key = $1",
        promise_table(item)
    ))
    .bind(key.to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    on.map(|on| (on, by.unwrap(), at.unwrap(), to))
}

fn promise_body(
    on: Option<chrono::NaiveDate>,
    shortcut: Option<reeve_protocol::PromiseShortcut>,
    to: Option<&str>,
) -> reeve_protocol::SetPromise {
    reeve_protocol::SetPromise {
        on,
        shortcut,
        to: to.map(str::to_string),
    }
}

fn validation_text(r: Result<(), HandlerError>) -> String {
    match r {
        Err(HandlerError::Validation(m)) => m,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

fn at_utc(m: u32, d: u32, h: u32, min: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, m, d, h, min, 0).unwrap()
}

/// kb/ADR-0025 R2: setting, changing and clearing are interactive-only, refused with the domain's texts, and logged
/// under `promised` / `promise_cleared` with what was there before. A kept date keeps `promised_at` (and so may be past);
/// `to` left out keeps the recipient and `""` clears it; clearing nothing logs nothing.
#[tokio::test]
async fn promises_are_set_changed_and_cleared_by_people_only() {
    use reeve_protocol::PromiseShortcut::{NextSprint, ThisSprint};
    use serde_json::json;
    let (_c, pool) = common::pg().await;
    let h = mk_db(pool.clone()).await;
    let c = promise_ctx();
    let bot = Ctx {
        via: Via::Automation,
        ..c.clone()
    };
    let e = h
        .create_epic(&c, "e", "E", None, "", None)
        .await
        .unwrap()
        .key()
        .clone();
    let s = plain_story(&h, &c, "s", "S", None).await;
    let done = plain_story(&h, &c, "d", "D", None).await;
    let dropped = plain_story(&h, &c, "x", "X", None).await;
    mark_done_at(&h, &done, c.now).await;
    sqlx::query(
        "UPDATE stories SET delivery = 'done', resolution = 'dropped', done_at = $2 WHERE key = $1",
    )
    .bind(dropped.to_string())
    .bind(c.now)
    .execute(&pool)
    .await
    .unwrap();

    // A done story, dropped included, cannot be promised; clearing one is allowed and, with nothing there, a no-op.
    for k in [&done, &dropped] {
        assert_eq!(
            validation_text(
                h.set_promise(
                    &c,
                    ItemType::Story,
                    k,
                    promise_body(Some(nov(20)), None, None)
                )
                .await
            ),
            format!("{k} is done: a done story cannot be promised")
        );
        assert!(!h.clear_promise(&c, ItemType::Story, k).await.unwrap());
        assert_eq!(stored(&pool, ItemType::Story, k).await, None);
        assert!(promise_entries(&pool, k).await.is_empty());
    }

    // A done story's promise can still be cleared (R2 refuses only setting).
    set_promise(
        &h,
        ItemType::Story,
        &done,
        &Promise::from_persisted(nov(20), "pm@x".into(), c.now, None),
    )
    .await;
    assert!(h.clear_promise(&c, ItemType::Story, &done).await.unwrap());
    assert_eq!(stored(&pool, ItemType::Story, &done).await, None);
    assert_eq!(
        promise_entries(&pool, &done).await,
        vec![(
            "story".to_string(),
            "promise_cleared".to_string(),
            json!({"item_type": "story", "previous": {"on": "2026-11-20", "to": null}})
        )]
    );

    // Initiatives cannot be promised (R1): automation is refused first, a person gets the rule; nothing is logged.
    let ini = h
        .create_initiative(&c, "i", "I", "", "Now", None, None, None)
        .await
        .unwrap()
        .key()
        .clone();
    assert!(matches!(
        h.set_promise(&bot, ItemType::Initiative, &ini, promise_body(Some(nov(20)), None, None))
            .await
            .unwrap_err(),
        HandlerError::Refused(r) if r == vec![Reason::InteractiveOnly]
    ));
    assert!(matches!(
        h.clear_promise(&bot, ItemType::Initiative, &ini).await.unwrap_err(),
        HandlerError::Refused(r) if r == vec![Reason::InteractiveOnly]
    ));
    assert_eq!(
        validation_text(
            h.set_promise(
                &c,
                ItemType::Initiative,
                &ini,
                promise_body(Some(nov(20)), None, None)
            )
            .await
        ),
        "an initiative cannot be promised"
    );
    assert_eq!(
        validation_text(
            h.clear_promise(&c, ItemType::Initiative, &ini)
                .await
                .map(|_| ())
        ),
        "an initiative cannot be promised"
    );
    assert!(promise_entries(&pool, &ini).await.is_empty());

    // No sprint exists yet: both shortcuts are refused.
    assert_eq!(
        validation_text(
            h.set_promise(
                &c,
                ItemType::Story,
                &s,
                promise_body(None, Some(ThisSprint), None)
            )
            .await
        ),
        "there is no active sprint: give a date"
    );
    assert_eq!(
        validation_text(
            h.set_promise(
                &c,
                ItemType::Epic,
                &e,
                promise_body(None, Some(NextSprint), None)
            )
            .await
        ),
        "there is no next sprint yet: give a date"
    );
    promise_sprints(&h, &c).await;

    for (item, k, name) in [
        (ItemType::Story, s.clone(), "story"),
        (ItemType::Epic, e.clone(), "epic"),
    ] {
        let set = |cx: Ctx, b: reeve_protocol::SetPromise| {
            let h = h.clone();
            let k = k.clone();
            async move { h.set_promise(&cx, item, &k, b).await }
        };
        let no_promise = format!("{k} has no promise to change: give a date");
        // Refusals: nothing stored, nothing logged.
        assert!(matches!(
            set(bot.clone(), promise_body(Some(nov(20)), None, None)).await.unwrap_err(),
            HandlerError::Refused(r) if r == vec![Reason::InteractiveOnly]
        ));
        let texts = [
            (
                c.clone(),
                promise_body(Some(nov(11)), None, None),
                "the date 2026-11-11 is past: promise today or later",
            ),
            (
                c.clone(),
                promise_body(Some(nov(20)), None, Some(&"é".repeat(201))),
                "the recipient is longer than 200 characters",
            ),
            (
                Ctx {
                    now: at_utc(11, 25, 8, 0),
                    ..c.clone()
                },
                promise_body(None, Some(ThisSprint), None),
                "the active sprint is planned to end 2026-11-24, which is past: give a date",
            ),
            (
                Ctx {
                    now: at_utc(12, 9, 8, 0),
                    ..c.clone()
                },
                promise_body(None, Some(NextSprint), None),
                "the next sprint is planned to end 2026-12-08, which is past: give a date",
            ),
            (
                c.clone(),
                promise_body(Some(nov(20)), None, Some("Ac\0me")),
                "the recipient cannot contain a NUL character",
            ),
            (
                c.clone(),
                promise_body(Some(nov(20)), Some(ThisSprint), None),
                "give a date or a sprint shortcut, not both",
            ),
            (
                c.clone(),
                promise_body(None, None, None),
                no_promise.as_str(),
            ),
            (
                c.clone(),
                promise_body(None, None, Some("Acme")),
                no_promise.as_str(),
            ),
        ];
        for (cx, b, want) in texts {
            assert_eq!(validation_text(set(cx, b).await), want, "{name}");
        }
        let ghost: Key = "ABC-9999".parse().unwrap();
        let wrong = if item == ItemType::Story { &e } else { &s };
        for missing in [&ghost, wrong] {
            assert!(matches!(
                h.set_promise(&c, item, missing, promise_body(Some(nov(20)), None, None))
                    .await
                    .unwrap_err(),
                HandlerError::NotFound
            ));
            assert!(matches!(
                h.clear_promise(&c, item, missing).await.unwrap_err(),
                HandlerError::NotFound
            ));
        }
        assert_eq!(stored(&pool, item, &k).await, None, "{name}");
        assert!(promise_entries(&pool, &k).await.is_empty(), "{name}");

        // Set: no previous promise.
        set(c.clone(), promise_body(Some(nov(20)), None, Some(" Acme ")))
            .await
            .unwrap();
        assert_eq!(
            stored(&pool, item, &k).await,
            Some((
                nov(20),
                "dan@example.com".into(),
                c.now,
                Some("Acme".into())
            ))
        );
        // A new date the next morning: a new commitment (new `at`, new `by`), the recipient kept. The request's UTC
        // date is the clock's today: 12 Nov is past at 13 Nov 08:00.
        let later = Ctx {
            actor: "pm@x".into(),
            now: at_utc(11, 13, 8, 0),
            ..c.clone()
        };
        assert_eq!(
            validation_text(set(later.clone(), promise_body(Some(nov(12)), None, None)).await),
            "the date 2026-11-12 is past: promise today or later"
        );
        set(later.clone(), promise_body(Some(nov(21)), None, None))
            .await
            .unwrap();
        assert_eq!(
            stored(&pool, item, &k).await,
            Some((nov(21), "pm@x".into(), later.now, Some("Acme".into())))
        );
        // Past the date: keeping it is allowed (only the recipient changes), `at` stays, `by` is the editor.
        let past = Ctx {
            actor: "lead@x".into(),
            now: at_utc(11, 25, 8, 0),
            ..c.clone()
        };
        assert_eq!(
            validation_text(set(past.clone(), promise_body(Some(nov(22)), None, None)).await),
            "the date 2026-11-22 is past: promise today or later",
            "a new past date is still refused"
        );
        set(
            past.clone(),
            promise_body(Some(nov(21)), None, Some("Beta")),
        )
        .await
        .unwrap();
        assert_eq!(
            stored(&pool, item, &k).await,
            Some((nov(21), "lead@x".into(), later.now, Some("Beta".into())))
        );
        // The same date and recipient again (a retry or a double submit), by someone else: no write, no entry.
        for b in [
            promise_body(Some(nov(21)), None, Some("Beta")),
            promise_body(Some(nov(21)), None, None),
            promise_body(None, None, Some(" Beta ")),
        ] {
            set(
                Ctx {
                    actor: "other@x".into(),
                    ..past.clone()
                },
                b,
            )
            .await
            .unwrap();
        }
        assert_eq!(
            stored(&pool, item, &k).await,
            Some((nov(21), "lead@x".into(), later.now, Some("Beta".into()))),
            "{name}: nothing changed, so nothing was written"
        );
        // Neither date nor shortcut, `to: ""`: the recipient cleared, the date kept.
        set(past.clone(), promise_body(None, None, Some("")))
            .await
            .unwrap();
        assert_eq!(
            stored(&pool, item, &k).await,
            Some((nov(21), "lead@x".into(), later.now, None))
        );
        // Clear: automation refused, then cleared once; clearing again logs nothing.
        assert!(matches!(
            h.clear_promise(&bot, item, &k).await.unwrap_err(),
            HandlerError::Refused(r) if r == vec![Reason::InteractiveOnly]
        ));
        assert!(stored(&pool, item, &k).await.is_some(), "{name}");
        assert!(
            h.clear_promise(&c, item, &k).await.unwrap(),
            "{name}: cleared"
        );
        assert_eq!(stored(&pool, item, &k).await, None);
        assert!(
            !h.clear_promise(&c, item, &k).await.unwrap(),
            "{name}: nothing left to clear"
        );

        let want = |action: &str, payload: serde_json::Value| {
            (name.to_string(), action.to_string(), payload)
        };
        assert_eq!(
            promise_entries(&pool, &k).await,
            vec![
                want(
                    "promised",
                    json!({"item_type": name, "on": "2026-11-20", "to": "Acme", "previous": null})
                ),
                want(
                    "promised",
                    json!({"item_type": name, "on": "2026-11-21", "to": "Acme",
                           "previous": {"on": "2026-11-20", "to": "Acme"}})
                ),
                want(
                    "promised",
                    json!({"item_type": name, "on": "2026-11-21", "to": "Beta",
                           "previous": {"on": "2026-11-21", "to": "Acme"}})
                ),
                want(
                    "promised",
                    json!({"item_type": name, "on": "2026-11-21", "to": null,
                           "previous": {"on": "2026-11-21", "to": "Beta"}})
                ),
                want(
                    "promise_cleared",
                    json!({"item_type": name, "previous": {"on": "2026-11-21", "to": null}})
                ),
            ],
            "{name}"
        );
        let actors: Vec<String> = sqlx::query_scalar(
            "SELECT actor FROM test_activity WHERE entity_key = $1 AND action IN ('promised', 'promise_cleared') ORDER BY id",
        )
        .bind(k.to_string())
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            actors,
            [
                "dan@example.com",
                "pm@x",
                "lead@x",
                "lead@x",
                "dan@example.com"
            ]
        );
    }
}

/// R6: a shortcut stores the sprint's planned end as it is when saved, never a length projection (from either sprint's
/// start or from the active sprint's end), and never follows a later edit of the sprint; the due period is then
/// classified against the sprints as they are now (R5).
#[tokio::test]
async fn shortcuts_store_the_planned_end_and_never_follow_later_edits() {
    use reeve_protocol::PromiseDue;
    use reeve_protocol::PromiseShortcut::{NextSprint, ThisSprint};
    let (_c, pool) = common::pg().await;
    let h = mk_db(pool.clone()).await;
    let c = promise_ctx();
    promise_sprints(&h, &c).await;
    for (state, ends) in [("active", "2026-11-25"), ("draft", "2026-12-11")] {
        sqlx::query("UPDATE sprints SET ends_on = $2::date WHERE state = $1")
            .bind(state)
            .bind(ends)
            .execute(&pool)
            .await
            .unwrap();
    }
    let projected: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sprints WHERE state IN ('active','draft') AND (ends_on = starts_on + 14 \
         OR ends_on = (SELECT ends_on FROM sprints WHERE state = 'active') + 14)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        projected, 0,
        "the planned ends differ from a length projection"
    );
    let a = plain_story(&h, &c, "a", "A", None).await;
    let b = plain_story(&h, &c, "b", "B", None).await;
    h.set_promise(
        &c,
        ItemType::Story,
        &a,
        promise_body(None, Some(ThisSprint), None),
    )
    .await
    .unwrap();
    h.set_promise(
        &c,
        ItemType::Story,
        &b,
        promise_body(None, Some(NextSprint), None),
    )
    .await
    .unwrap();
    let on = |k: Key| {
        let pool = pool.clone();
        async move { stored(&pool, ItemType::Story, &k).await.unwrap().0 }
    };
    assert_eq!(on(a.clone()).await, nov(25));
    assert_eq!(
        on(b.clone()).await,
        chrono::NaiveDate::from_ymd_opt(2026, 12, 11).unwrap()
    );

    for (state, ends) in [("active", "2026-11-20"), ("draft", "2026-12-01")] {
        sqlx::query("UPDATE sprints SET ends_on = $2::date WHERE state = $1")
            .bind(state)
            .bind(ends)
            .execute(&pool)
            .await
            .unwrap();
    }
    assert_eq!(on(a.clone()).await, nov(25), "the stored date stays");
    assert_eq!(
        on(b.clone()).await,
        chrono::NaiveDate::from_ymd_opt(2026, 12, 11).unwrap()
    );
    let due = |k: Key| {
        let h = h.clone();
        async move { h.story_dto(&k, c.now).await.unwrap().promise.unwrap().due }
    };
    assert_eq!(due(a).await, Some(PromiseDue::NextSprint));
    assert_eq!(due(b).await, Some(PromiseDue::Later));
}

/// Records what the write's own transaction shows as the item's `promised_on`, writes the entry on that transaction,
/// then fails: the entry must see the write already made, and the failure must roll both back.
struct CheckSink(Mutex<Vec<Option<chrono::NaiveDate>>>);
#[async_trait]
impl ActivitySink for CheckSink {
    async fn record(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        rec: ActivityRecord,
    ) -> Result<(), RepoError> {
        let table = if rec.entity_type == "epic" {
            "epics"
        } else {
            "stories"
        };
        let on: Option<chrono::NaiveDate> =
            sqlx::query_scalar(&format!("SELECT promised_on FROM {table} WHERE key = $1"))
                .bind(&rec.entity_key)
                .fetch_one(&mut **tx)
                .await?;
        self.0.lock().unwrap().push(on);
        insert_test_activity(tx, &rec).await?;
        Err(RepoError::NotFound)
    }
}

/// kb/ADR-0010 R5: the entry is the write's last work, inside its transaction. The sink sees the new date on set and
/// NULL on clear; its failure leaves the old promise and no entry.
#[tokio::test]
async fn promise_writes_record_their_entry_last_and_roll_back_with_it() {
    let (_c, pool) = common::pg().await;
    let h = mk_db(pool.clone()).await;
    let sink = Arc::new(CheckSink(Mutex::new(vec![])));
    let check = Handlers {
        activity: sink.clone(),
        ..h.clone()
    };
    let c = promise_ctx();
    let e = h
        .create_epic(&c, "e", "E", None, "", None)
        .await
        .unwrap()
        .key()
        .clone();
    let s = plain_story(&h, &c, "s", "S", None).await;
    let old = Promise::from_persisted(nov(20), "pm@x".into(), c.now, Some("Acme".into()));
    let old_cols = Some((nov(20), "pm@x".to_string(), c.now, Some("Acme".to_string())));
    for (item, k) in [(ItemType::Story, s), (ItemType::Epic, e)] {
        // From nothing: the sink saw the new date; the failure stored nothing.
        sink.0.lock().unwrap().clear();
        check
            .set_promise(&c, item, &k, promise_body(Some(nov(27)), None, None))
            .await
            .unwrap_err();
        assert_eq!(*sink.0.lock().unwrap(), vec![Some(nov(27))]);
        assert_eq!(stored(&pool, item, &k).await, None);
        // Over an existing promise.
        set_promise(&h, item, &k, &old).await;
        sink.0.lock().unwrap().clear();
        check
            .set_promise(&c, item, &k, promise_body(Some(nov(27)), None, None))
            .await
            .unwrap_err();
        assert_eq!(*sink.0.lock().unwrap(), vec![Some(nov(27))]);
        assert_eq!(stored(&pool, item, &k).await, old_cols);
        sink.0.lock().unwrap().clear();
        check.clear_promise(&c, item, &k).await.unwrap_err();
        assert_eq!(*sink.0.lock().unwrap(), vec![None]);
        assert_eq!(stored(&pool, item, &k).await, old_cols);
        assert!(promise_entries(&pool, &k).await.is_empty());
    }
}

/// `previous` is read under the row lock: a set or clear that starts while another writer holds the row waits for it,
/// and logs that writer's promise as `previous`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_set_waits_for_a_concurrent_writer_and_logs_its_promise_as_previous() {
    let (_c, pool) = common::pg().await;
    let h = mk_db(pool.clone()).await;
    let c = promise_ctx();
    let mut cases = Vec::new();
    for (n, set) in [(0, true), (1, false)] {
        cases.push((
            ItemType::Story,
            plain_story(&h, &c, &format!("s{n}"), "S", None).await,
            set,
        ));
        cases.push((
            ItemType::Epic,
            h.create_epic(&c, &format!("e{n}"), "E", None, "", None)
                .await
                .unwrap()
                .key()
                .clone(),
            set,
        ));
    }
    for (item, k, set) in cases {
        let table = promise_table(item);
        let mut w = pool.begin().await.unwrap();
        sqlx::query(&format!("SELECT 1 FROM {table} WHERE key = $1 FOR UPDATE"))
            .bind(k.to_string())
            .execute(&mut *w)
            .await
            .unwrap();
        sqlx::query(&format!(
            "UPDATE {table} SET promised_on = '2026-11-20', promised_by = 'x', promised_at = now() WHERE key = $1"
        ))
        .bind(k.to_string())
        .execute(&mut *w)
        .await
        .unwrap();
        let (hh, cc, kk) = (h.clone(), c.clone(), k.clone());
        let call = tokio::spawn(async move {
            if set {
                hh.set_promise(&cc, item, &kk, promise_body(Some(nov(27)), None, None))
                    .await
            } else {
                hh.clear_promise(&cc, item, &kk).await.map(|_| ())
            }
        });
        wait_until_blocked(&pool).await;
        w.commit().await.unwrap();
        tokio::time::timeout(PAUSE, call)
            .await
            .expect("the write never finished")
            .unwrap()
            .unwrap();
        let entries = promise_entries(&pool, &k).await;
        assert_eq!(entries.len(), 1, "{item:?} set={set}: {entries:?}");
        assert_eq!(
            entries[0].2["previous"]["on"], "2026-11-20",
            "{item:?} set={set}"
        );
    }
}

/// R2: a promise takes no version and changes none, so an edit with a version read before it succeeds, and edits,
/// moves and the discovery mark leave it as it was.
#[tokio::test]
async fn a_promise_survives_edits_moves_and_the_discovery_mark() {
    let (_c, pool) = common::pg().await;
    let h = mk_db(pool.clone()).await;
    let c = promise_ctx();
    let epic = ready_epic(&h, &c).await;
    let s = h
        .create_story(
            &c,
            "s",
            "S",
            Kind::Chore,
            Some(epic),
            "",
            vec!["works".into()],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let read = h.get_story(s.key()).await.unwrap().version();
    h.set_promise(
        &c,
        ItemType::Story,
        s.key(),
        promise_body(Some(nov(20)), None, Some("Acme")),
    )
    .await
    .unwrap();
    let promised = stored(&pool, ItemType::Story, s.key()).await;
    assert!(promised.is_some());
    assert_eq!(h.get_story(s.key()).await.unwrap().version(), read);
    let edited = h
        .edit_story(
            &c,
            s.key(),
            read,
            Some("S2".into()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("the promise did not bump the version");
    assert_eq!(edited.version(), read + 1);
    let moved = h
        .move_story(
            &c,
            s.key(),
            edited.version(),
            DeliveryState::InProgress,
            None,
            None,
        )
        .await
        .expect("a promise never refuses a move");
    assert_eq!(moved.version(), read + 2);
    assert_eq!(stored(&pool, ItemType::Story, s.key()).await, promised);

    let e = h.create_epic(&c, "e2", "E", None, "", None).await.unwrap();
    h.set_promise(
        &c,
        ItemType::Epic,
        e.key(),
        promise_body(Some(nov(21)), None, None),
    )
    .await
    .unwrap();
    let promised = stored(&pool, ItemType::Epic, e.key()).await;
    assert!(promised.is_some());
    assert_eq!(
        h.epic_dto(e.key(), c.now).await.unwrap().version,
        e.version()
    );
    let e2 = h
        .edit_epic(
            &c,
            e.key(),
            e.version(),
            Some("E2".into()),
            None,
            None,
            None,
        )
        .await
        .expect("the promise did not bump the epic's version");
    assert_eq!(e2.version(), e.version() + 1);
    h.mark_discovery(&c, ItemType::Epic, e.key(), e2.version(), true)
        .await
        .unwrap();
    let dto = h.epic_dto(e.key(), c.now).await.unwrap();
    assert!(dto.discovery_complete.is_some());
    assert_eq!(stored(&pool, ItemType::Epic, e.key()).await, promised);
}

/// R7: readiness, delivery, progress and rank never depend on promises.
#[tokio::test]
async fn promises_change_no_readiness_delivery_progress_or_rank() {
    let (_c, pool) = common::pg().await;
    let h = mk_db(pool.clone()).await;
    let c = promise_ctx();
    promise_sprints(&h, &c).await;
    let i = h
        .create_initiative(&c, "i", "I", "", "Now", None, None, None)
        .await
        .unwrap();
    let e = h
        .create_epic(&c, "e", "E", Some(i.key().clone()), "", None)
        .await
        .unwrap();
    h.mark_discovery(&c, ItemType::Initiative, i.key(), 1, true)
        .await
        .unwrap();
    h.mark_discovery(&c, ItemType::Epic, e.key(), 1, true)
        .await
        .unwrap();
    let mut keys = Vec::new();
    for (n, criteria) in [
        ("ready", vec!["ok".to_string()]),
        ("bare", vec![]),
        ("blocked", vec!["ok".to_string()]),
        ("decided", vec!["ok".to_string()]),
        ("done", vec!["ok".to_string()]),
    ] {
        keys.push(
            h.create_story(
                &c,
                n,
                n,
                Kind::Story,
                Some(e.key().clone()),
                "",
                criteria,
                vec![],
                None,
                None,
            )
            .await
            .unwrap()
            .key()
            .clone(),
        );
    }
    h.add_block(&c, &keys[2], &keys[0]).await.unwrap();
    h.require_decision(&c, "d", &keys[3], "Pick", "dan@example.com", None, None)
        .await
        .unwrap();
    mark_done_at(&h, &keys[4], c.now).await;
    type Snapshot = (
        Vec<(
            Key,
            reeve_protocol::Readiness,
            i32,
            DeliveryState,
            Option<Resolution>,
        )>,
        Vec<(Key, Option<f32>)>,
        Vec<reeve_protocol::Readiness>,
    );
    // The snapshots read on 20 Nov, after the promised dates: overdue promises are covered too.
    let then = at_utc(11, 20, 10, 0);
    let snapshot = || async {
        let board = h
            .list_stories(
                &StoryFilter::default(),
                None,
                None,
                false,
                DoneFilter::All,
                then,
            )
            .await
            .unwrap()
            .into_iter()
            .map(|r| {
                (
                    r.story.key,
                    r.readiness,
                    r.story.rank,
                    r.story.delivery,
                    r.story.resolution,
                )
            })
            .collect();
        let progress = h
            .list_initiatives(then)
            .await
            .unwrap()
            .into_iter()
            .map(|i| (i.key, i.progress))
            .collect();
        let mut one_at_a_time = Vec::new();
        for k in &keys {
            one_at_a_time.push(h.readiness(k).await.unwrap());
        }
        let s: Snapshot = (board, progress, one_at_a_time);
        s
    };
    let before = snapshot().await;
    assert_eq!(before.0.len(), 5, "every story is in the snapshot");
    assert!(before.0.iter().any(|(_, r, ..)| r.ready), "one is ready");
    assert!(before.0.iter().any(|(_, r, ..)| !r.ready), "one is not");
    assert!(before.1[0].1.is_some(), "the initiative has progress");
    for k in [&keys[2], &keys[3]] {
        h.set_promise(
            &c,
            ItemType::Story,
            k,
            promise_body(Some(nov(14)), None, None),
        )
        .await
        .unwrap();
    }
    h.set_promise(
        &c,
        ItemType::Epic,
        e.key(),
        promise_body(Some(nov(13)), None, None),
    )
    .await
    .unwrap();
    for k in &keys[..4] {
        assert_eq!(
            h.story_dto(k, then)
                .await
                .unwrap()
                .promise
                .and_then(|p| p.state),
            Some(reeve_protocol::PromiseState::Overdue),
            "{k}: the promises are in place and overdue"
        );
    }
    assert_eq!(snapshot().await, before);
}

/// `promised` keeps exactly the stories whose derived promise passes `promise::Filter::matches`, ANDed with the other
/// filters. The clock is 12 Nov; the active sprint ends 24 Nov and the draft 8 Dec. O is overdue (10 Nov, promised on
/// 9 Nov) and in progress; T, N and L are open on 20 Nov, 1 Dec and 20 Dec; E inherits its epic's 22 Nov; K kept, D
/// dropped; P has no promise. F, done before its epic promised, is a control: it never matches. N alone is ready, so a
/// kept row whose readiness came from a dropped row (the filter applied before rows meet their readiness) shows.
/// Distinguishes `this_sprint` without the overdue, done stories leaking in, a missing AND with the other filters,
/// and rows misaligned with their readiness.
#[tokio::test]
async fn promised_selects_exact_sets() {
    use std::collections::HashSet;
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let c = promise_ctx();
    let now = c.now;
    promise_sprints(&h, &c).await;
    let own = |on, at| Promise::from_persisted(on, "pm@x".into(), at, None);
    let early = Utc.with_ymd_and_hms(2026, 11, 12, 9, 0, 0).unwrap();

    let o = plain_story(&h, &c, "O", "O", None).await;
    set_promise(
        &h,
        ItemType::Story,
        &o,
        &own(
            nov(10),
            Utc.with_ymd_and_hms(2026, 11, 9, 10, 0, 0).unwrap(),
        ),
    )
    .await;
    sqlx::query(
        "UPDATE stories SET delivery = 'in_progress', owner = 'olive@example.com' WHERE key = $1",
    )
    .bind(o.to_string())
    .execute(&h.pool)
    .await
    .unwrap();
    let t = plain_story(&h, &c, "T", "T", None).await;
    set_promise(&h, ItemType::Story, &t, &own(nov(20), early)).await;
    let ready = ready_epic(&h, &c).await;
    let n = h
        .create_story(
            &c,
            "N",
            "N",
            Kind::Chore,
            Some(ready),
            "",
            vec!["ok".into()],
            vec![],
            None,
            None,
        )
        .await
        .unwrap()
        .key()
        .clone();
    let dec = |d| chrono::NaiveDate::from_ymd_opt(2026, 12, d).unwrap();
    set_promise(&h, ItemType::Story, &n, &own(dec(1), early)).await;
    let l = plain_story(&h, &c, "L", "L", None).await;
    set_promise(&h, ItemType::Story, &l, &own(dec(20), early)).await;
    let ep = h.create_epic(&c, "pe", "PE", None, "", None).await.unwrap();
    set_promise(&h, ItemType::Epic, ep.key(), &own(nov(22), early)).await;
    let e = plain_story(&h, &c, "E", "E", Some(ep.key())).await;
    let f = plain_story(&h, &c, "F", "F", Some(ep.key())).await;
    mark_done_at(&h, &f, Utc.with_ymd_and_hms(2026, 11, 12, 8, 0, 0).unwrap()).await;
    let k = plain_story(&h, &c, "K", "K", None).await;
    set_promise(&h, ItemType::Story, &k, &own(nov(20), early)).await;
    mark_done_at(&h, &k, early).await;
    let d = plain_story(&h, &c, "D", "D", None).await;
    set_promise(&h, ItemType::Story, &d, &own(nov(20), early)).await;
    mark_done_at(&h, &d, early).await;
    sqlx::query("UPDATE stories SET resolution = 'dropped' WHERE key = $1")
        .bind(d.to_string())
        .execute(&h.pool)
        .await
        .unwrap();
    let p = plain_story(&h, &c, "P", "P", None).await;

    let set = |v: Vec<Key>| v.into_iter().collect::<HashSet<_>>();
    let of = |ks: &[&Key]| ks.iter().map(|k| (*k).clone()).collect::<HashSet<_>>();
    let promised = |p| StoryFilter {
        promised: Some(p),
        ..Default::default()
    };
    use reeve_protocol::PromisedFilter as P;
    let cases = [
        ("this_sprint", promised(P::ThisSprint), of(&[&o, &t, &e])),
        ("next_sprint", promised(P::NextSprint), of(&[&n])),
        ("overdue", promised(P::Overdue), of(&[&o])),
        ("any", promised(P::Any), of(&[&o, &t, &n, &l, &e])),
        (
            "any with an owner",
            StoryFilter {
                owner: Some("olive@example.com".into()),
                ..promised(P::Any)
            },
            of(&[&o]),
        ),
        (
            "this_sprint with delivery=todo",
            StoryFilter {
                delivery: Some(DeliveryState::Todo),
                ..promised(P::ThisSprint)
            },
            of(&[&t, &e]),
        ),
    ];
    let all = h
        .list_stories(
            &StoryFilter::default(),
            None,
            None,
            false,
            DoneFilter::All,
            now,
        )
        .await
        .unwrap();
    assert_eq!(
        set(all.iter().map(|s| s.story.key.clone()).collect()),
        of(&[&o, &t, &n, &l, &e, &f, &k, &d, &p]),
        "the baseline lists all nine"
    );
    let ready_of = |rows: &[reeve_protocol::StoryWithReadiness]| -> Vec<(Key, bool)> {
        rows.iter()
            .map(|s| (s.story.key.clone(), s.readiness.ready))
            .collect()
    };
    assert_eq!(
        ready_of(&all).iter().filter(|(_, r)| *r).count(),
        1,
        "only N is ready"
    );
    assert_ne!(all[0].story.key, n, "a dropped row comes before N");
    for (label, ready_only) in [("next_sprint", false), ("any, ready only", true)] {
        let which = if ready_only { P::Any } else { P::NextSprint };
        let rows = h
            .list_stories(
                &promised(which),
                None,
                None,
                ready_only,
                DoneFilter::All,
                now,
            )
            .await
            .unwrap();
        assert_eq!(ready_of(&rows), vec![(n.clone(), true)], "{label}");
    }
    for (label, filter, want) in cases {
        assert_eq!(
            set(list_keys(&h, &filter, None, None, now).await),
            want,
            "{label}"
        );
    }
}

/// One decision on an epic blocks four stories: a control promised 10 Nov and unfinished (overdue on 12 Nov), one
/// promised the same day and done on 9 Nov (kept), one dropped, and one without a promise. Only the unfinished promise
/// pulls the decision: it is needed this sprint with the control as the reason. Once the control is done the decision
/// falls to beyond (the unpromised story keeps it affected); an overdue promise on the epic, which that story inherits,
/// pulls it back with the epic as the reason. The decision's due date is never written (kb/ADR-0025 R7). Catches done
/// promises pulling decisions, the reason naming the wrong item, and the queue writing a due date.
#[tokio::test]
async fn only_unfinished_promises_pull_decisions() {
    use reeve_protocol::{NeededIn, NeededInReason, PromiseState, QueueLane};
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let c = promise_ctx();
    let (epic, ss) = marked_epic_with_stories(&h, &c, "pp", 4).await;
    let (control, kept, dropped) = (ss[0].key(), ss[1].key(), ss[2].key());
    let due = Some(nov(30));
    let d = h
        .require_decision(&c, "pp-d", &epic, "D", "dan@example.com", due, None)
        .await
        .unwrap();
    for k in [control, kept, dropped] {
        let p = Promise::from_persisted(nov(10), "pm@x".into(), c.now, None);
        set_promise(&h, ItemType::Story, k, &p).await;
    }
    mark_done_at(
        &h,
        kept,
        Utc.with_ymd_and_hms(2026, 11, 9, 9, 0, 0).unwrap(),
    )
    .await;
    mark_done_at(
        &h,
        dropped,
        Utc.with_ymd_and_hms(2026, 11, 11, 9, 0, 0).unwrap(),
    )
    .await;
    sqlx::query("UPDATE stories SET resolution = 'dropped' WHERE key = $1")
        .bind(dropped.to_string())
        .execute(&h.pool)
        .await
        .unwrap();
    for (k, want) in [(kept, PromiseState::Kept), (dropped, PromiseState::Dropped)] {
        let p = h.story_dto(k, c.now).await.unwrap().promise.unwrap();
        assert_eq!(p.state, Some(want), "{k}");
    }

    let (es, _) = queue_all(&h, &c).await;
    let e = es.iter().find(|e| e.decision.id() == d.id()).unwrap();
    assert_eq!(
        (e.needed_in, e.lane, e.needed_in_reason.clone()),
        (
            NeededIn::ThisSprint,
            QueueLane::ThisSprint,
            Some(NeededInReason::Promise {
                key: control.clone(),
                level: ItemType::Story,
                on: nov(10),
                overdue: true
            })
        )
    );
    assert_eq!(e.decision.due(), due);

    mark_done_at(&h, control, c.now).await;
    let (es, _) = queue_all(&h, &c).await;
    let e = es.iter().find(|e| e.decision.id() == d.id()).unwrap();
    assert_eq!(
        (e.needed_in, e.lane, e.needed_in_reason.clone()),
        (NeededIn::Beyond, QueueLane::Beyond, None)
    );
    assert_eq!(e.decision.due(), due);

    // The epic's promise, inherited by the unpromised story, pulls it back and the reason names the epic.
    let p = Promise::from_persisted(nov(11), "lead@x".into(), c.now, None);
    set_promise(&h, ItemType::Epic, &epic, &p).await;
    let (es, _) = queue_all(&h, &c).await;
    let e = es.iter().find(|e| e.decision.id() == d.id()).unwrap();
    assert_eq!(
        (e.needed_in, e.needed_in_reason.clone()),
        (
            NeededIn::ThisSprint,
            Some(NeededInReason::Promise {
                key: epic.clone(),
                level: ItemType::Epic,
                on: nov(11),
                overdue: true
            })
        )
    );
    assert_eq!(e.decision.due(), due);
}

/// Open promises through the real story list and `PromiseWindow::of`: with the active sprint planned to end 24 Nov and the
/// draft 8 Dec, a decision on a story promised 20 Nov is needed this sprint, one promised 1 Dec next sprint, and one
/// promised 20 Dec beyond, with no reason. Catches the due period lost or swapped between the DTO and the domain.
#[tokio::test]
async fn open_promises_pull_decisions_by_due_period() {
    use reeve_protocol::{NeededIn, NeededInReason};
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let c = promise_ctx();
    promise_sprints(&h, &c).await;
    let (_epic, ss) = marked_epic_with_stories(&h, &c, "op", 3).await;
    let dec = chrono::NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();
    let late = chrono::NaiveDate::from_ymd_opt(2026, 12, 20).unwrap();
    let mut ids = vec![];
    for (i, (s, on)) in ss.iter().zip([nov(20), dec, late]).enumerate() {
        let p = Promise::from_persisted(on, "pm@x".into(), c.now, None);
        set_promise(&h, ItemType::Story, s.key(), &p).await;
        let d = h
            .require_decision(
                &c,
                &format!("op-d{i}"),
                s.key(),
                "D",
                "dan@example.com",
                None,
                None,
            )
            .await
            .unwrap();
        ids.push(d.id());
    }
    let (es, _) = queue_all(&h, &c).await;
    let of = |id: i64| {
        let e = es.iter().find(|e| e.decision.id() == id).unwrap();
        (e.needed_in, e.needed_in_reason.clone())
    };
    let reason = |i: usize, on| {
        Some(NeededInReason::Promise {
            key: ss[i].key().clone(),
            level: ItemType::Story,
            on,
            overdue: false,
        })
    };
    assert_eq!(of(ids[0]), (NeededIn::ThisSprint, reason(0, nov(20))));
    assert_eq!(of(ids[1]), (NeededIn::NextSprint, reason(1, dec)));
    assert_eq!(of(ids[2]), (NeededIn::Beyond, None));
}

/// Pauses the first entry it records once armed, inside the write that records it and so while that write still holds
/// its row lock, until the test releases it; every other entry goes straight through. Entries land in `test_activity`
/// as `DbSink`'s do. The race tests order their writes with it instead of relying on PostgreSQL's lock queue.
#[derive(Default)]
struct GateSink {
    armed: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[async_trait]
impl ActivitySink for GateSink {
    async fn record(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        rec: ActivityRecord,
    ) -> Result<(), RepoError> {
        insert_test_activity(tx, &rec).await?;
        if self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        Ok(())
    }
}
impl GateSink {
    /// Arms the gate: the next entry recorded pauses.
    fn arm(&self) {
        self.armed.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    /// Waits until a write paused in the gate.
    async fn paused(&self) {
        tokio::time::timeout(PAUSE, self.entered.notified())
            .await
            .expect("the first write never reached its entry");
    }
    /// Lets the paused write go on to commit.
    fn open(&self) {
        self.release.notify_one();
    }
}

/// Handlers whose entries go through a `GateSink` into `test_activity` (created by `mk_db`).
async fn mk_gated(pool: sqlx::PgPool) -> (Handlers, Arc<GateSink>) {
    let gate = Arc::new(GateSink::default());
    let h = Handlers {
        activity: gate.clone(),
        ..mk_db(pool).await
    };
    (h, gate)
}

/// Two real promise writes on one row (a set then a set, a set then a clear, on a story and an epic): the first pauses
/// in its entry while it holds the row, the second is started and seen blocked on that row, then the first is released.
/// Both commit, each logs the promise the other left as its `previous`, and the stored promise is what the last entry
/// left.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queued_promise_writes_chain_their_entries() {
    use serde_json::{Value, json};
    let (_c, pool) = common::pg().await;
    let (h, gate) = mk_gated(pool.clone()).await;
    let c = promise_ctx();
    let mut cases = Vec::new();
    for (n, second_sets) in [(0, true), (1, false)] {
        cases.push((
            ItemType::Story,
            plain_story(&h, &c, &format!("s{n}"), "S", None).await,
            second_sets,
        ));
        cases.push((
            ItemType::Epic,
            h.create_epic(&c, &format!("e{n}"), "E", None, "", None)
                .await
                .unwrap()
                .key()
                .clone(),
            second_sets,
        ));
    }
    for (item, k, second_sets) in cases {
        let name = format!(
            "{item:?} then {}",
            if second_sets { "set" } else { "clear" }
        );
        h.set_promise(
            &c,
            item,
            &k,
            promise_body(Some(nov(20)), None, Some("Acme")),
        )
        .await
        .unwrap();
        gate.arm();
        let (hh, cc, kk) = (h.clone(), c.clone(), k.clone());
        let first = tokio::spawn(async move {
            hh.set_promise(
                &cc,
                item,
                &kk,
                promise_body(Some(nov(27)), None, Some("Bolt")),
            )
            .await
        });
        gate.paused().await;
        let (hh, cc, kk) = (h.clone(), c.clone(), k.clone());
        let second = tokio::spawn(async move {
            if second_sets {
                hh.set_promise(&cc, item, &kk, promise_body(Some(nov(30)), None, None))
                    .await
            } else {
                hh.clear_promise(&cc, item, &kk).await.map(|_| ())
            }
        });
        wait_until_blocked(&pool).await;
        assert!(!second.is_finished(), "{name}: the second write waits");
        gate.open();
        for call in [first, second] {
            tokio::time::timeout(PAUSE, call)
                .await
                .expect("a write never finished")
                .unwrap()
                .unwrap();
        }
        let entries = promise_entries(&pool, &k).await;
        assert_eq!(entries.len(), 3, "{name}: {entries:?}");
        // Each entry's `previous` is what the entry before it left; the stored promise is what the last one left.
        let mut left = Value::Null;
        for (_, action, payload) in &entries {
            assert_eq!(payload["previous"], left, "{name}: {entries:?}");
            left = match action.as_str() {
                "promised" => json!({"on": payload["on"], "to": payload["to"]}),
                "promise_cleared" => Value::Null,
                other => panic!("{name}: unexpected action {other}"),
            };
        }
        let now = stored(&pool, item, &k)
            .await
            .map_or(Value::Null, |(on, _, _, to)| json!({"on": on, "to": to}));
        assert_eq!(now, left, "{name}");
        // The first write went first: its entry names the first promise, the second's names it.
        let it = if item == ItemType::Story {
            "story"
        } else {
            "epic"
        };
        let bolt = json!({"on": "2026-11-27", "to": "Bolt"});
        assert_eq!(
            entries[1].2,
            json!({"item_type": it, "on": "2026-11-27", "to": "Bolt",
                   "previous": {"on": "2026-11-20", "to": "Acme"}}),
            "{name}"
        );
        let second_entry = if second_sets {
            json!({"item_type": it, "on": "2026-11-30", "to": "Bolt", "previous": bolt})
        } else {
            json!({"item_type": it, "previous": bolt})
        };
        assert_eq!(entries[2].2, second_entry, "{name}");
    }
}

/// Completion first (kb/ADR-0025 R2): a move to done pauses in its entry while it holds the story's row, a promise is
/// started and seen blocked on that row, then the move is released and commits; the promise, judged on the row it then
/// locks, is refused as done. Nothing is stored or logged.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_promise_queued_behind_a_move_to_done_is_refused() {
    let (_c, pool) = common::pg().await;
    let (h, gate) = mk_gated(pool.clone()).await;
    let c = promise_ctx();
    let s = plain_story(&h, &c, "s", "S", None).await;
    let version = h.story_dto(&s, c.now).await.unwrap().version;
    gate.arm();
    let (hh, cc, kk) = (h.clone(), c.clone(), s.clone());
    let mv = tokio::spawn(async move {
        hh.move_story(
            &cc,
            &kk,
            version,
            DeliveryState::Done,
            Some(Resolution::Dropped),
            Some("dropped while promised".into()),
        )
        .await
    });
    gate.paused().await;
    let (hh, cc, kk) = (h.clone(), c.clone(), s.clone());
    let promise = tokio::spawn(async move {
        hh.set_promise(
            &cc,
            ItemType::Story,
            &kk,
            promise_body(Some(nov(20)), None, None),
        )
        .await
    });
    wait_until_blocked(&pool).await;
    assert!(!promise.is_finished(), "the promise waits on the move");
    gate.open();
    tokio::time::timeout(PAUSE, mv)
        .await
        .expect("the move never finished")
        .unwrap()
        .unwrap();
    let refused = tokio::time::timeout(PAUSE, promise)
        .await
        .expect("the promise never finished")
        .unwrap();
    assert_eq!(
        validation_text(refused),
        format!("{s} is done: a done story cannot be promised")
    );
    assert_eq!(stored(&pool, ItemType::Story, &s).await, None);
    assert!(promise_entries(&pool, &s).await.is_empty());
}

/// kb/ADR-0021 R4 for `promised`: the filter's membership comes from the snapshot the rows come from. Promises moved
/// while the list waits do not change the set it returns; the next request sees the new set.
#[tokio::test]
async fn the_promised_filter_reads_one_snapshot() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let c = promise_ctx();
    let now = c.now;
    promise_sprints(&h, &c).await;
    let a = plain_story(&h, &c, "A", "A", None).await;
    let b = plain_story(&h, &c, "B", "B", None).await;
    let _x = plain_story(&h, &c, "X", "X", None).await;
    for (k, on) in [
        (&a, nov(20)),
        (&b, chrono::NaiveDate::from_ymd_opt(2026, 12, 1).unwrap()),
    ] {
        set_promise(
            &h,
            ItemType::Story,
            k,
            &Promise::from_persisted(on, "pm@x".into(), now, None),
        )
        .await;
    }
    let list = |h: Handlers| async move {
        let f = StoryFilter {
            promised: Some(reeve_protocol::PromisedFilter::ThisSprint),
            ..Default::default()
        };
        h.list_stories(&f, None, None, false, DoneFilter::All, now)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.story.key)
            .collect::<Vec<_>>()
    };
    assert_eq!(list(h.clone()).await, vec![a.clone()]);

    let mut lock = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE sprints IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let call = tokio::spawn(list(h.clone()));
    wait_until_blocked(&pool).await;
    for (k, on) in [(&a, "2026-12-01"), (&b, "2026-11-22")] {
        sqlx::query("UPDATE stories SET promised_on = $2::date WHERE key = $1")
            .bind(k.to_string())
            .bind(on)
            .execute(&mut *lock)
            .await
            .unwrap();
    }
    lock.commit().await.unwrap();
    let before = tokio::time::timeout(PAUSE, call)
        .await
        .expect("the list never finished")
        .unwrap();
    assert_eq!(
        before,
        vec![a.clone()],
        "the set of the snapshot the rows came from"
    );
    assert_eq!(
        list(h.clone()).await,
        vec![b],
        "the next request sees the new set"
    );
}

/// kb/ADR-0025 R5: `promised` keeps only unfinished stories, so done ones are never read. A done promised story is not
/// listed, and adding done promised stories changes neither the statements nor the rows the list reads, with the
/// default `done=all`.
#[tokio::test]
async fn the_promised_filter_reads_no_done_story() {
    let (_c, pool) = common::pg_counted().await;
    let (h, _sink) = mk(pool.clone());
    let c = promise_ctx();
    let now = c.now;
    let promise = Promise::from_persisted(nov(20), "pm@x".into(), now, None);
    let open = plain_story(&h, &c, "o", "O", None).await;
    set_promise(&h, ItemType::Story, &open, &promise).await;
    let add_done = |n: usize| {
        let (h, c, promise) = (h.clone(), c.clone(), promise.clone());
        async move {
            for i in 0..n {
                let k = plain_story(&h, &c, &format!("d{n}-{i}"), "D", None).await;
                set_promise(&h, ItemType::Story, &k, &promise).await;
                mark_done_at(&h, &k, now).await;
            }
        }
    };
    add_done(1).await;
    let list = || {
        let h = h.clone();
        async move {
            let f = StoryFilter {
                promised: Some(reeve_protocol::PromisedFilter::Any),
                ..Default::default()
            };
            let got: Vec<Key> = h
                .list_stories(&f, None, None, false, DoneFilter::All, now)
                .await
                .unwrap()
                .into_iter()
                .map(|s| s.story.key)
                .collect();
            got
        }
    };
    assert_eq!(
        list().await,
        vec![open.clone()],
        "the done promised story is not listed"
    );
    let run = || async {
        list().await;
    };
    let rows = common::rows(&pool, run).await;
    let statements = common::statements(&pool, run).await;
    assert!(rows > 0, "the counter measured {rows} rows");
    add_done(5).await;
    assert_eq!(list().await, vec![open]);
    assert_eq!(
        common::rows(&pool, run).await,
        rows,
        "rows of done stories were read"
    );
    assert_eq!(common::statements(&pool, run).await, statements);
}

/// Seeds `n` stories with blockers, pull requests and decisions (`seed_board`), promises the first story itself
/// (overdue on `today`) and the epic (in a week, inherited by every story). Returns the stories and the epic.
async fn promised_board(
    h: &Handlers,
    tag: &str,
    n: usize,
    first_pr: i64,
    today: chrono::NaiveDate,
) -> (Vec<Key>, Key) {
    let keys = seed_board(h, tag, n, first_pr).await;
    let epic = h
        .story_dto(&keys[0], Utc::now())
        .await
        .unwrap()
        .epic
        .unwrap();
    let at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let own = Promise::from_persisted(today - chrono::Days::new(2), "pm@x".into(), at, None);
    set_promise(h, ItemType::Story, &keys[0], &own).await;
    let theirs = Promise::from_persisted(today + chrono::Days::new(7), "lead@x".into(), at, None);
    set_promise(h, ItemType::Epic, &epic, &theirs).await;
    (keys, epic)
}

/// Own and inherited promises cost no statements: the story list and the decision queue read the same number with a
/// few promised stories as with many, and both answers carry the promises.
#[tokio::test]
async fn promises_add_no_statements_to_the_list_or_the_queue() {
    use reeve_protocol::{NeededInReason, PromiseHolder, PromiseState};
    let (_c, pool) = common::pg_counted().await;
    let (h, _sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);
    let now = Utc::now();
    let (a, epic_a) = promised_board(&h, "a", 3, 1, now.date_naive()).await;
    let list = || {
        let h = h.clone();
        async move {
            h.list_stories(
                &StoryFilter::default(),
                None,
                None,
                false,
                DoneFilter::All,
                now,
            )
            .await
            .unwrap()
        }
    };
    let queue = || {
        let (h, c) = (h.clone(), Ctx { now, ..dan.clone() });
        async move { queue_all(&h, &c).await }
    };
    // Populated: the first story is overdue on its own date, the others inherit the epic's; the overdue promise
    // gives the decision on the first story its reason.
    let rows = list().await;
    for s in &rows {
        let p = s.story.promise.as_ref().expect("every story is promised");
        let want = if s.story.key == a[0] {
            (PromiseHolder::Own, Some(PromiseState::Overdue))
        } else {
            (PromiseHolder::Epic, Some(PromiseState::Open))
        };
        assert_eq!((p.holder, p.state), want, "{}", s.story.key);
    }
    assert_eq!(rows.len(), 3);
    let (es, _) = queue().await;
    assert!(
        es.iter().any(|e| e.needed_in_reason
            == Some(NeededInReason::Promise {
                key: a[0].clone(),
                level: ItemType::Story,
                on: now.date_naive() - chrono::Days::new(2),
                overdue: true
            })),
        "{es:?}"
    );
    let _ = epic_a;
    let run_list = || async {
        list().await;
    };
    let run_queue = || async {
        queue().await;
    };
    let few = (
        common::statements(&pool, run_list).await,
        common::statements(&pool, run_queue).await,
    );
    promised_board(&h, "b", 9, 100, now.date_naive()).await;
    promised_board(&h, "c", 4, 200, now.date_naive()).await;
    assert_eq!(list().await.len(), 16);
    assert!(list().await.iter().all(|s| s.story.promise.is_some()));
    let many = (
        common::statements(&pool, run_list).await,
        common::statements(&pool, run_queue).await,
    );
    assert_eq!(few, many);
}

/// For a fixed set of stories (one epic's), promised stories and epics elsewhere add no rows to what the list reads:
/// the promise facts are read for the listed keys only.
#[tokio::test]
async fn unrelated_promises_add_no_rows_to_a_fixed_list() {
    let (_c, pool) = common::pg_counted().await;
    let (h, _sink) = mk(pool.clone());
    let now = Utc::now();
    let (a, epic_a) = promised_board(&h, "a", 3, 1, now.date_naive()).await;
    let list = || {
        let (h, epic_a) = (h.clone(), epic_a.clone());
        async move {
            let f = StoryFilter {
                epics: Some(vec![epic_a]),
                ..Default::default()
            };
            h.list_stories(&f, None, None, false, DoneFilter::All, now)
                .await
                .unwrap()
        }
    };
    let keys = |rows: Vec<reeve_protocol::StoryWithReadiness>| -> Vec<(Key, bool)> {
        rows.into_iter()
            .map(|s| (s.story.key, s.story.promise.is_some()))
            .collect()
    };
    let want: Vec<(Key, bool)> = a.iter().map(|k| (k.clone(), true)).collect();
    assert_eq!(keys(list().await), want);
    let run = || async {
        list().await;
    };
    let before = common::rows(&pool, run).await;
    assert!(before > 0, "the counter measured {before} rows");
    let (b, _) = promised_board(&h, "b", 9, 100, now.date_naive()).await;
    // Promised history elsewhere: some of it done.
    for k in &b[..4] {
        mark_done_at(&h, k, now).await;
    }
    assert_eq!(keys(list().await), want);
    assert_eq!(common::rows(&pool, run).await, before);
}

/// kb/ADR-0027 R5 and R6 through the handlers: the owner of an in-review blocker starts its unowned dependent without
/// an override, under automation, and becomes its owner; another person is refused with a reason naming the
/// blocker's state and owner; the batched list and the single read agree; changing the blocker's owner changes the
/// answer. Catches the mover not being passed to the gate, the list and the gate reading different facts, and an
/// owner change that the loader does not see.
#[tokio::test]
async fn own_unmerged_work_blocks_others_but_not_its_owner() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let dan_bot = ctx("dan@example.com", true);
    let mia = ctx("mia@example.com", false);
    let epic = h.create_epic(&dan, "e", "E", None, "", None).await.unwrap();
    h.mark_discovery(&dan, ItemType::Epic, epic.key(), 1, true)
        .await
        .unwrap();
    let mk_story = |idem: &'static str, owner: Option<&'static str>| {
        let h = h.clone();
        let c = dan.clone();
        let e = epic.key().clone();
        async move {
            h.create_story(
                &c,
                idem,
                idem,
                Kind::Story,
                Some(e),
                "",
                vec!["ok".into()],
                vec![],
                None,
                owner.map(Into::into),
            )
            .await
            .unwrap()
        }
    };
    let b = mk_story("b", None).await;
    let d1 = mk_story("d1", None).await;
    let d2 = mk_story("d2", None).await;
    let d3 = mk_story("d3", Some("dan@example.com")).await;
    let d4 = mk_story("d4", Some("mia@example.com")).await;
    for d in [&d1, &d2, &d3, &d4] {
        h.add_block(&dan, d.key(), b.key()).await.unwrap();
    }
    // Dan starts B (becoming its owner) and puts it in review.
    let b1 = h
        .move_story(
            &dan,
            b.key(),
            b.version(),
            DeliveryState::InProgress,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(b1.owner(), Some("dan@example.com"));
    let b2 = h
        .move_story(
            &dan,
            b.key(),
            b1.version(),
            DeliveryState::InReview,
            None,
            None,
        )
        .await
        .unwrap();

    let expected = vec![Reason::BlockedBy {
        story: b.key().clone(),
        delivery: DeliveryState::InReview,
        owner: Some("dan@example.com".into()),
    }];
    // Unowned, D1 is judged without a mover: blocked, in the list and in the single read alike.
    assert_eq!(h.readiness(d1.key()).await.unwrap().reasons, expected);
    let list = h
        .list_stories(
            &StoryFilter::default(),
            None,
            None,
            false,
            DoneFilter::All,
            Utc::now(),
        )
        .await
        .unwrap();
    let row = |k: &Key| {
        list.iter()
            .find(|x| &x.story.key == k)
            .unwrap()
            .readiness
            .clone()
    };
    assert_eq!(
        row(d1.key()).reasons,
        expected,
        "the batched read agrees with the single one"
    );
    // D3 is Dan's: ready in both reads, because its owner and B's are the same.
    assert!(row(d3.key()).ready, "{:?}", row(d3.key()));
    assert!(h.readiness(d3.key()).await.unwrap().ready);

    // Mia may not start D2: refused with the reason naming B's state and owner.
    match h
        .move_story(
            &mia,
            d2.key(),
            d2.version(),
            DeliveryState::InProgress,
            None,
            None,
        )
        .await
    {
        Err(HandlerError::Refused(reasons)) => assert_eq!(reasons, expected),
        other => panic!("expected a refusal, got {other:?}"),
    }
    // The mover never replaces an existing owner: Dan's agent is refused on Mia's D4, judged as Mia's.
    match h
        .move_story(
            &dan_bot,
            d4.key(),
            d4.version(),
            DeliveryState::InProgress,
            None,
            None,
        )
        .await
    {
        Err(HandlerError::Refused(reasons)) => assert_eq!(reasons, expected),
        other => panic!("expected a refusal, got {other:?}"),
    }
    // Dan's agent starts D1 without an override and becomes its owner.
    let started = h
        .move_story(
            &dan_bot,
            d1.key(),
            d1.version(),
            DeliveryState::InProgress,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(started.delivery(), DeliveryState::InProgress);
    assert_eq!(started.owner(), Some("dan@example.com"));

    // Handing B to Mia blocks Dan's owned dependent D3 (the facts are read live, never stored).
    h.edit_story(
        &dan,
        b.key(),
        b2.version(),
        None,
        None,
        None,
        None,
        None,
        Some(Some("mia@example.com".into())),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        h.readiness(d3.key()).await.unwrap().reasons,
        vec![Reason::BlockedBy {
            story: b.key().clone(),
            delivery: DeliveryState::InReview,
            owner: Some("mia@example.com".into()),
        }]
    );
}
