mod common;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use reeve_planning::*;
use reeve_protocol::{
    BriefTemplateDto, BriefTemplateLevel, CheckResult, ClosureKind, DeliveryState, DoneFilter,
    ItemType, Key, Kind, Reason, Resolution, Reversibility, TokenKind, Via,
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

/// Writes each entry into a table the test made, on the mutation's own transaction, as the real activity log does,
/// and refuses the entry on the item `refuse` names, after writing it. A mutation whose entry is refused has by
/// then written its row, and any earlier entry, on a transaction that its caller never commits.
struct TableSink {
    refuse: Mutex<Option<String>>,
}
#[async_trait]
impl ActivitySink for TableSink {
    async fn record(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        rec: ActivityRecord,
    ) -> Result<(), RepoError> {
        let refused = self.refuse.lock().unwrap().as_deref() == Some(rec.entity_key.as_str());
        sqlx::query("INSERT INTO entries_seen (action, entity_key) VALUES ($1, $2)")
            .bind(rec.action)
            .bind(&rec.entity_key)
            .execute(&mut **tx)
            .await?;
        if refused {
            return Err(RepoError::Db(sqlx::Error::Protocol(
                "the test's sink refuses this entry".into(),
            )));
        }
        Ok(())
    }
}
impl TableSink {
    /// Refuses the next entries on `key`, or none.
    fn refuse(&self, key: Option<&Key>) {
        *self.refuse.lock().unwrap() = key.map(Key::to_string);
    }
}
/// Handlers over a `TableSink`, with its table made.
async fn mk_table_sink(pool: sqlx::PgPool) -> (Handlers, Arc<TableSink>) {
    sqlx::query("CREATE TABLE entries_seen (action TEXT NOT NULL, entity_key TEXT NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    let sink = Arc::new(TableSink {
        refuse: Mutex::new(None),
    });
    let h = Handlers {
        ports: make_planning_ports(pool.clone()),
        activity: sink.clone(),
        pool,
        settings: settings(),
    };
    (h, sink)
}
/// How many entries with this action are committed: counted on another connection than any mutation's.
async fn entries_committed(pool: &sqlx::PgPool, action: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM entries_seen WHERE action = $1")
        .bind(action)
        .fetch_one(pool)
        .await
        .unwrap()
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
        client: None,
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
        client: None,
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
        client: None,
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
        let mut dto = h.story_dto(s.key(), Utc::now()).await.unwrap();
        // The one difference between the two readers, by design (kb/ADR-0031): only a read of one item
        // carries its item links and its web links. None exist here.
        assert_eq!(dto.links.take(), Some(vec![]));
        assert_eq!(dto.web_links.take(), Some(vec![]));
        single.push(dto);
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
        client: None,
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
        .unwrap()
        .0;

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
        let mut single = h.initiative_dto(&key, Utc::now()).await.unwrap();
        // Only a read of one item carries its item links and its web links (kb/ADR-0031); none exist here.
        assert_eq!(single.links.take(), Some(vec![]));
        assert_eq!(single.web_links.take(), Some(vec![]));
        assert_eq!(dto, &single);
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
    // Measured before the readiness queue (GRA-611): discovery rows add no statement (spec §5).
    assert_eq!(few, 18, "the counter measured {few} statements");
    seed_board(&h, "b", 9, 100).await;
    seed_board(&h, "c", 4, 200).await;
    let many = common::statements(&pool, q).await;
    assert_eq!(few, many);
}

/// End to end over real readiness (spec §1, §2, §5). Initiative I (Now) and its epics E and E2 have no discovery mark;
/// E holds todo S1, S2 and in-progress S3, E2 holds todo S4; an active goal serves S1 and S4. D1 is on E, D2 on S1, D3
/// on initiative I2 (no stories). A marked epic EM under a marked initiative holds S5, with DM on EM. Epic EL under a
/// Later initiative holds goal-less S6. Rows: E (this sprint, also I, S3 under way) with D1 nested, then E2; EL is left
/// out; EM and its initiative are no rows. The summary counts S1 once and not S4, which no decision holds. Accepting
/// D1's record moves it to satisfied. Scoped by `On(S1)` the queue lists D2 and no row; by `Initiative(I)` D1 and D2 with
/// E's and E2's rows; D1's context is nested under E in the All queue's lane. Catches counting active stories, a stale
/// group after acceptance, a scope that leaks, a left-out row emitted, a marked item made a row, and a context that
/// disagrees with the page.
#[tokio::test]
async fn decision_queue_over_real_readiness() {
    use reeve_protocol::{NeededInReason, QueueGroup, QueueLane};
    async fn story_in(
        h: &Handlers,
        c: &Ctx,
        idem: &str,
        title: &str,
        epic: &reeve_protocol::Key,
    ) -> Story {
        h.create_story(
            c,
            idem,
            title,
            Kind::Story,
            Some(epic.clone()),
            "",
            vec!["works".into()],
            vec![],
            None,
            None,
        )
        .await
        .unwrap()
    }
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
    let s1 = story_in(&h, &dan, "q-s1", "S1", epic.key()).await;
    let s2 = story_in(&h, &dan, "q-s2", "S2", epic.key()).await;
    let s3 = story_in(&h, &dan, "q-s3", "S3", epic.key()).await;
    let epic2 = h
        .create_epic(&dan, "q-e2", "E2", Some(ini.key().clone()), "", None)
        .await
        .unwrap();
    let s4 = story_in(&h, &dan, "q-s4", "S4", epic2.key()).await;
    // A marked epic under I with one goal-less todo story: its discovery item is I itself.
    let epic3 = h
        .create_epic(&dan, "q-e3", "E3", Some(ini.key().clone()), "", None)
        .await
        .unwrap();
    h.mark_discovery(&dan, ItemType::Epic, epic3.key(), epic3.version(), true)
        .await
        .unwrap();
    let s5 = story_in(&h, &dan, "q-s5", "S5", epic3.key()).await;
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
    let d3 = h
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
    let (marked, ms) = marked_epic_with_stories(&h, &dan, "q-m", 1).await;
    let dm = h
        .require_decision(&dan, "q-dm", &marked, "DM", "dan@example.com", None, None)
        .await
        .unwrap();
    let later = h
        .create_initiative(&dan, "q-il", "IL", "", "Later", None, None, None)
        .await
        .unwrap();
    let el = h
        .create_epic(&dan, "q-el", "EL", Some(later.key().clone()), "", None)
        .await
        .unwrap();
    story_in(&h, &dan, "q-s6", "S6", el.key()).await;
    let (ga, _) = sprints_serving(&h, &dan, "q", &[s1.key().clone(), s4.key().clone()], &[]).await;

    let all = DecisionScope::All {
        include_closed: false,
    };
    let ids = |q: &[QueueEntry]| q.iter().map(|e| e.decision.id()).collect::<Vec<_>>();
    let keys = |ss: &[QueueStory]| ss.iter().map(|s| s.key.clone()).collect::<Vec<_>>();
    let (es, rows, summary) = h.decision_queue(&all, dan.now).await.unwrap();
    assert_eq!(
        es.iter()
            .map(|e| (e.group, e.lane, e.decision.id(), e.under.clone()))
            .collect::<Vec<_>>(),
        vec![
            (
                QueueGroup::Blocking,
                QueueLane::ThisSprint,
                d1.id(),
                Some(epic.key().clone())
            ),
            (QueueGroup::Blocking, QueueLane::ThisSprint, d2.id(), None),
            (QueueGroup::Blocking, QueueLane::Beyond, dm.id(), None),
            (QueueGroup::Open, QueueLane::Open, d3.id(), None),
        ]
    );
    assert_eq!(keys(&es[0].held), vec![s1.key().clone(), s2.key().clone()]);
    assert_eq!(es[0].held[0].title, "S1");
    assert_eq!(keys(&es[1].held), vec![s1.key().clone()]);
    assert_eq!(keys(&es[2].held), vec![ms[0].key().clone()]);
    let goal = Some(NeededInReason::Goal {
        goal_id: ga,
        outcome: "active outcome".into(),
    });
    assert_eq!(
        rows.iter()
            .map(|r| (
                r.key.clone(),
                r.lane,
                r.also.clone(),
                r.needed_in_reason.clone(),
                keys(&r.held),
                keys(&r.under_way),
                r.at
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                epic.key().clone(),
                QueueLane::ThisSprint,
                Some(ini.key().clone()),
                goal.clone(),
                vec![s1.key().clone(), s2.key().clone()],
                vec![s3.key().clone()],
                0
            ),
            (
                epic2.key().clone(),
                QueueLane::ThisSprint,
                Some(ini.key().clone()),
                goal,
                vec![s4.key().clone()],
                vec![],
                1
            ),
            (
                ini.key().clone(),
                QueueLane::Beyond,
                None,
                Some(NeededInReason::Held),
                vec![s5.key().clone()],
                vec![],
                2
            ),
        ]
    );
    assert_eq!(
        (
            summary.open,
            summary.blocking,
            summary.blocked_stories,
            summary.this_sprint,
            summary.next_sprint
        ),
        (4, 3, 3, 2, 0)
    );

    let (on_s1, on_rows, _) = h
        .decision_queue(&DecisionScope::On(s1.key().clone()), dan.now)
        .await
        .unwrap();
    assert_eq!((ids(&on_s1), on_rows.len()), (vec![d2.id()], 0));
    // One item's own row only: E's, with the decisions on E.
    let (on_e, e_rows, _) = h
        .decision_queue(&DecisionScope::On(epic.key().clone()), dan.now)
        .await
        .unwrap();
    assert_eq!(
        (
            ids(&on_e),
            e_rows.iter().map(|r| r.key.clone()).collect::<Vec<_>>()
        ),
        (vec![d1.id()], vec![epic.key().clone()])
    );
    let (under_i, i_rows, _) = h
        .decision_queue(&DecisionScope::Initiative(ini.key().clone()), dan.now)
        .await
        .unwrap();
    assert_eq!(ids(&under_i), vec![d1.id(), d2.id()]);
    assert_eq!(
        i_rows
            .iter()
            .map(|r| (r.key.clone(), r.lane, r.also.clone(), r.at))
            .collect::<Vec<_>>(),
        vec![
            (
                epic.key().clone(),
                QueueLane::ThisSprint,
                Some(ini.key().clone()),
                0
            ),
            (
                epic2.key().clone(),
                QueueLane::ThisSprint,
                Some(ini.key().clone()),
                1
            ),
            (ini.key().clone(), QueueLane::Beyond, None, 2),
        ]
    );
    assert_eq!(i_rows[2].needed_in_reason, Some(NeededInReason::Held));
    let c1 = h.decision_context(d1.id(), dan.now).await.unwrap();
    assert_eq!((c1.under, c1.lane), (Some(epic.key().clone()), es[0].lane));

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
    let (es, _, summary) = h.decision_queue(&all, dan.now).await.unwrap();
    let e1 = es.iter().find(|e| e.decision.id() == d1.id()).unwrap();
    assert_eq!((e1.group, e1.under.clone()), (QueueGroup::Satisfied, None));
    assert!(e1.held.is_empty());
    let e2 = es.iter().find(|e| e.decision.id() == d2.id()).unwrap();
    assert_eq!(
        (e2.group, keys(&e2.held)),
        (QueueGroup::Blocking, vec![s1.key().clone()])
    );
    assert_eq!(
        (summary.open, summary.blocking, summary.blocked_stories),
        (3, 2, 2)
    );
}

/// Why each Now item sits where it does, through the real wiring (spec §3, kb/ADR-0024 R5). All three are in the first
/// bucket with no goal or promise. Epic EU (unmarked) holds S1 and its initiative IU, not the row's own item, is
/// marked urgent: its reason is `urgent` naming IU (lineage). Epic ET's initiative has target month 2026-11 and
/// nothing is urgent under it: `target_month`. A marked epic's story has a decision due within the sprint length (10 days
/// (today 26 Oct, due 2 Nov): that decision's reason is `decision_due`; one due a day past the configured sprint
/// length is only `held`. Order: urgent, month, due, held. Catches the urgency read off the row's own key only, the
/// initiative's month not reaching a row, a due date not read, and a window other than the sprint length.
#[tokio::test]
async fn decision_queue_now_reasons_over_real_readiness() {
    use reeve_protocol::NeededInReason;
    let (_c, pool) = common::pg().await;
    let (mut h, _sink) = mk(pool);
    // A sprint length unlike any other 14-day setting, so the window is the sprint length's and nothing else's.
    h.settings.sprint_length_days = 10;
    let dan = ctx("dan@example.com", false);
    let epic_with_story = |tag: &'static str, month: Option<&'static str>| {
        let (h, dan) = (h.clone(), dan.clone());
        async move {
            let ini = h
                .create_initiative(
                    &dan,
                    &format!("{tag}-i"),
                    "I",
                    "",
                    "Now",
                    month.map(String::from),
                    None,
                    None,
                )
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
            h.create_story(
                &dan,
                &format!("{tag}-s"),
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
            (ini.key().clone(), epic.key().clone())
        }
    };
    let (iu, eu) = epic_with_story("u", None).await;
    let (_it, et) = epic_with_story("t", Some("2026-11")).await;
    h.mark_urgent(&dan, ItemType::Initiative, &iu, None)
        .await
        .unwrap();
    let (_marked, ms) = marked_epic_with_stories(&h, &dan, "d", 2).await;
    let due = h
        .require_decision(
            &dan,
            "d-dec",
            ms[0].key(),
            "D",
            "dan@example.com",
            chrono::NaiveDate::from_ymd_opt(2026, 11, 2),
            None,
        )
        .await
        .unwrap();
    // One day past the configured sprint length: not `decision_due`, only `held`.
    let past =
        dan.now.date_naive() + chrono::Days::new(u64::from(h.settings.sprint_length_days) + 1);
    let held = h
        .require_decision(
            &dan,
            "d-held",
            ms[1].key(),
            "H",
            "dan@example.com",
            Some(past),
            None,
        )
        .await
        .unwrap();
    let all = DecisionScope::All {
        include_closed: false,
    };
    let (es, rows, _) = h.decision_queue(&all, dan.now).await.unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| (r.key.clone(), r.lane, r.needed_in_reason.clone(), r.at))
            .collect::<Vec<_>>(),
        vec![
            (
                eu,
                reeve_protocol::QueueLane::Beyond,
                Some(NeededInReason::Urgent {
                    key: iu,
                    level: ItemType::Initiative
                }),
                0
            ),
            (
                et,
                reeve_protocol::QueueLane::Beyond,
                Some(NeededInReason::TargetMonth {
                    month: "2026-11".into()
                }),
                0
            ),
        ]
    );
    assert_eq!(
        es.iter()
            .map(|e| (e.decision.id(), e.needed_in_reason.clone()))
            .collect::<Vec<_>>(),
        vec![
            (due.id(), Some(NeededInReason::DecisionDue { id: due.id() })),
            (held.id(), Some(NeededInReason::Held)),
        ]
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
    let (es, _, s) = h
        .decision_queue(
            &DecisionScope::All {
                include_closed: false,
            },
            c.now,
        )
        .await
        .unwrap();
    (es, s)
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

/// A decision on an initiative without its discovery mark under the first bucket, with no stories, is open with no
/// reason: only goals and promises put work in a sprint lane, and an item without unfinished stories is no row (spec
/// §2). Catches the old discovery candidate surviving end to end.
#[tokio::test]
async fn queue_decision_on_an_unmarked_item_without_stories_is_open() {
    use reeve_protocol::QueueLane;
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let now_ini = h
        .create_initiative(&dan, "i-now", "I", "", "Now", None, None, None)
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
    let (es, rows, summary) = h
        .decision_queue(
            &DecisionScope::All {
                include_closed: false,
            },
            dan.now,
        )
        .await
        .unwrap();
    let e = es.iter().find(|e| e.decision.id() == d1.id()).unwrap();
    assert_eq!(
        (e.lane, e.needed_in_reason.clone()),
        (QueueLane::Open, None)
    );
    assert_eq!(summary.next_sprint, 0);
    assert!(rows.is_empty());
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
        (
            NeededIn::Beyond,
            QueueLane::Beyond,
            Some(NeededInReason::Held)
        )
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
/// promised 20 Dec in Now, ranked by what it holds. Catches the due period lost or swapped between the DTO and the domain.
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
    assert_eq!(of(ids[2]), (NeededIn::Beyond, Some(NeededInReason::Held)));
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

/// What `created` must mean: this call inserted the row. It is atomic by construction: the flag is the insert's own
/// `ON CONFLICT DO NOTHING` answer inside the handler's transaction, true only from the insert itself. A smoke test:
/// it does not force the race (nothing makes both lookups finish before either write); it checks that of two creates with one
/// key, run concurrently on separate pool connections, exactly one reports that it created, whichever order the
/// database serves them in.
#[tokio::test]
async fn a_create_reports_whether_it_inserted_also_when_two_run_at_once() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let create = || {
        h.create_story_attaching(
            &dan,
            "race-1",
            "Booking flow",
            Kind::Story,
            None,
            "",
            vec![],
            vec![],
            None,
            None,
            &[],
        )
    };
    let (first, second) = tokio::join!(create(), create());
    let (first, second) = (first.unwrap(), second.unwrap());
    assert_eq!(first.0.key(), second.0.key(), "one story");
    assert_eq!(
        [first.1, second.1]
            .iter()
            .filter(|created| **created)
            .count(),
        1,
        "exactly one call inserted it"
    );
    // A later repeat finds it and says so.
    let (again, created) = create().await.unwrap();
    assert_eq!((again.key(), created), (first.0.key(), false));
    assert_eq!(
        sink.0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.action == "created")
            .count(),
        1,
        "one activity entry"
    );
}

#[tokio::test]
async fn a_comment_reports_whether_it_inserted() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let story = h
        .create_story(
            &dan,
            "cr-s1",
            "Booking flow",
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
    let first = h
        .comment_story_attaching(&dan, story.key(), "hello", Some("cr-c1"), &[])
        .await
        .unwrap();
    let repeat = h
        .comment_story_attaching(&dan, story.key(), "hello again", Some("cr-c1"), &[])
        .await
        .unwrap();
    assert_eq!((first.1, repeat), (true, (first.0, false)));
    // Without a key every comment is new.
    let unkeyed = h
        .comment_item_attaching(&dan, ItemType::Story, story.key(), "no key", None, &[])
        .await
        .unwrap();
    assert!(unkeyed.1 && unkeyed.0 != first.0);
    // An epic's and a decision's comment report the same way.
    let epic = ready_epic(&h, &dan).await;
    let on_epic = |body: &'static str| {
        h.comment_item_attaching(&dan, ItemType::Epic, &epic, body, Some("cr-c2"), &[])
    };
    let (first, repeat) = (on_epic("one").await.unwrap(), on_epic("two").await.unwrap());
    assert_eq!((first.1, repeat), (true, (first.0, false)));
}

/// kb/ADR-0029 R9: a write made through a client names it in every entry it logs, and a write made any other
/// way has no such field. Create, comment, edit and move are the four writes a client can make.
#[tokio::test]
async fn every_entry_of_a_write_made_through_a_client_names_the_client() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let plain = ctx("dan@example.com", false);
    let through = Ctx {
        client: Some("claude.ai".into()),
        ..ctx("dan@example.com", false)
    };
    let control = h
        .create_story(
            &plain,
            "cl-s0",
            "Booking flow",
            Kind::Story,
            None,
            "",
            vec!["works".into()],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    let story = h
        .create_story(
            &through,
            "cl-s1",
            "Booking flow",
            Kind::Story,
            None,
            "",
            vec!["works".into()],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    h.comment_story(&through, story.key(), "hello", None)
        .await
        .unwrap();
    let edited = h
        .edit_story(
            &through,
            story.key(),
            story.version(),
            Some("Booking".into()),
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
    h.move_story(
        &through,
        story.key(),
        edited.version(),
        DeliveryState::Done,
        Some(Resolution::Dropped),
        None,
    )
    .await
    .unwrap();
    let records = sink.0.lock().unwrap();
    let of = |key: &Key| -> Vec<(&'static str, serde_json::Value)> {
        records
            .iter()
            .filter(|r| r.entity_key == key.to_string())
            .map(|r| (r.action, r.payload["client"].clone()))
            .collect()
    };
    let named = serde_json::json!("claude.ai");
    assert_eq!(
        of(story.key()),
        vec![
            ("created", named.clone()),
            ("commented", named.clone()),
            ("edited", named.clone()),
            ("moved", named)
        ]
    );
    assert_eq!(
        of(control.key()),
        vec![("created", serde_json::Value::Null)]
    );
    assert!(
        records.iter().all(|r| r.payload.is_object()),
        "a payload that is not an object could not carry the client"
    );
}

/// kb/ADR-0031 R1, R4, R7. An item link joins items of any level, is added once (a repeat writes and logs
/// nothing), is logged on both items with the same payload, reads from each end with that side's wording and the
/// other item's level and title, and is removed only by its forward form; `relates_to` is one link from either
/// end. An automation caller may do all of it.
#[tokio::test]
async fn item_links_are_added_once_logged_on_both_items_and_read_from_either_end() {
    use reeve_protocol::ItemLinkType::{Duplicates, Follows, RelatesTo};
    use serde_json::json;
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);
    let bot = ctx("bot@example.com", true);
    let i = h
        .create_initiative(&dan, "i", "Billing", "", "Now", None, None, None)
        .await
        .unwrap()
        .key()
        .clone();
    let e = h
        .create_epic(&dan, "e", "Invoices", Some(i.clone()), "", None)
        .await
        .unwrap()
        .key()
        .clone();
    let s = h
        .create_story(
            &dan,
            "s",
            "Template",
            Kind::Story,
            Some(e.clone()),
            "",
            vec![],
            vec![],
            None,
            None,
        )
        .await
        .unwrap()
        .key()
        .clone();
    let entries = |action: &str| -> Vec<(String, String, serde_json::Value)> {
        sink.0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.action == action)
            .map(|r| {
                (
                    r.entity_type.to_string(),
                    r.entity_key.clone(),
                    r.payload.clone(),
                )
            })
            .collect()
    };
    let links = |k: &Key| {
        let (h, k) = (h.clone(), k.clone());
        async move { serde_json::to_value(h.link_listing(&k).await.unwrap().links).unwrap() }
    };

    // The story follows the epic.
    assert!(
        h.add_item_link(&bot, &s, Follows, &e).await.unwrap(),
        "added"
    );
    assert!(
        !h.add_item_link(&bot, &s, Follows, &e).await.unwrap(),
        "already there"
    );
    let follows = json!({"type": "follows", "from": s, "to": e});
    assert_eq!(
        entries("item_link_added"),
        vec![
            ("story".to_string(), s.to_string(), follows.clone()),
            ("epic".to_string(), e.to_string(), follows.clone()),
        ],
        "one entry on each item, none for the repeat"
    );
    assert_eq!(
        links(&s).await,
        json!([{"type": "follows", "direction": "out", "label": "follows",
                "item": {"key": e, "level": "epic", "title": "Invoices"}}])
    );
    assert_eq!(
        links(&e).await,
        json!([{"type": "follows", "direction": "in", "label": "is followed by",
                "item": {"key": s, "level": "story", "title": "Template"}}])
    );

    // The reader behind a write's answer gives the same item links as the listing, and nothing else.
    assert_eq!(
        h.item_links_of(&s).await.unwrap(),
        h.link_listing(&s).await.unwrap().links
    );
    assert!(matches!(
        h.item_links_of(&"ABC-999".parse().unwrap()).await,
        Err(HandlerError::NotFound)
    ));

    // relates_to named from either end is one link, `out` on both sides. The initiative has the lower key.
    assert!(h.add_item_link(&bot, &s, RelatesTo, &i).await.unwrap());
    assert!(
        !h.add_item_link(&bot, &i, RelatesTo, &s).await.unwrap(),
        "the same link from the other end"
    );
    assert_eq!(
        links(&i).await,
        json!([{"type": "relates_to", "direction": "out", "label": "relates to",
                "item": {"key": s, "level": "story", "title": "Template"}}])
    );
    let related = json!({"type": "relates_to", "from": i, "to": s});
    assert_eq!(entries("item_link_added").len(), 4);
    assert_eq!(
        entries("item_link_added")[2..],
        [
            ("initiative".to_string(), i.to_string(), related.clone()),
            ("story".to_string(), s.to_string(), related.clone()),
        ],
        "the entry names the ends as stored"
    );

    // Order on one item: by name, `out` before `in`, then the other key.
    assert!(h.add_item_link(&bot, &e, Duplicates, &s).await.unwrap());
    assert!(h.add_item_link(&bot, &e, Follows, &s).await.unwrap());
    let on_story = links(&s).await;
    let shape: Vec<(String, String)> = on_story
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["label"].as_str().unwrap().to_string(),
                l["item"]["key"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        shape,
        vec![
            ("follows".to_string(), e.to_string()),
            ("is followed by".to_string(), e.to_string()),
            ("is duplicated by".to_string(), e.to_string()),
            ("relates to".to_string(), i.to_string()),
        ]
    );

    // Removing: a directed link only by its forward form.
    assert!(
        !h.remove_item_link(&bot, &e, Duplicates, &i).await.unwrap(),
        "no such link"
    );
    assert!(h.remove_item_link(&bot, &s, Follows, &e).await.unwrap());
    assert!(
        !h.remove_item_link(&bot, &s, Follows, &e).await.unwrap(),
        "already gone"
    );
    assert_eq!(
        entries("item_link_removed"),
        vec![
            ("story".to_string(), s.to_string(), follows.clone()),
            ("epic".to_string(), e.to_string(), follows),
        ]
    );
    assert_eq!(
        links(&s).await.as_array().unwrap().len(),
        3,
        "the epic's own `follows` of the story is another link and stays"
    );
    assert!(
        h.remove_item_link(&bot, &s, RelatesTo, &i).await.unwrap(),
        "relates_to from the other end"
    );
    assert_eq!(links(&i).await, json!([]));

    // Refusals write nothing.
    let before = sink.0.lock().unwrap().len();
    let ghost: Key = "ABC-999".parse().unwrap();
    assert!(matches!(
        h.add_item_link(&bot, &s, Follows, &s).await,
        Err(HandlerError::Validation(m)) if m == "an item cannot have an item link to itself"
    ));
    for (from, to) in [(&s, &ghost), (&ghost, &s)] {
        assert!(matches!(
            h.add_item_link(&bot, from, Follows, to).await,
            Err(HandlerError::NotFound)
        ));
        assert!(matches!(
            h.remove_item_link(&bot, from, Follows, to).await,
            Err(HandlerError::NotFound)
        ));
    }
    assert!(matches!(
        h.link_listing(&ghost).await,
        Err(HandlerError::NotFound)
    ));
    assert_eq!(sink.0.lock().unwrap().len(), before);
}

/// kb/ADR-0031 R7. A `relates_to` between an epic and a story, named with the higher key (the story) first: the
/// link is stored with the epic first, and each entry of the add and of the remove is recorded under its own
/// item's level and key, never under the other's.
#[tokio::test]
async fn a_relates_to_named_with_the_higher_key_first_is_logged_under_each_items_own_level() {
    use reeve_protocol::ItemLinkType::RelatesTo;
    use serde_json::json;
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool.clone());
    let c = ctx("dan@example.com", false);
    let e = h
        .create_epic(&c, "e", "Invoices", None, "", None)
        .await
        .unwrap()
        .key()
        .clone();
    let s = h
        .create_story(
            &c,
            "s",
            "Template",
            Kind::Story,
            Some(e.clone()),
            "",
            vec![],
            vec![],
            None,
            None,
        )
        .await
        .unwrap()
        .key()
        .clone();
    assert!(
        e.to_string() < s.to_string(),
        "the control: the story has the higher key, {e} < {s}"
    );
    let entries = |action: &str| -> Vec<(String, String, serde_json::Value)> {
        sink.0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.action == action)
            .map(|r| {
                (
                    r.entity_type.to_string(),
                    r.entity_key.clone(),
                    r.payload.clone(),
                )
            })
            .collect()
    };
    let stored = json!({"type": "relates_to", "from": e, "to": s});
    let want = vec![
        ("epic".to_string(), e.to_string(), stored.clone()),
        ("story".to_string(), s.to_string(), stored.clone()),
    ];
    assert!(h.add_item_link(&c, &s, RelatesTo, &e).await.unwrap());
    assert_eq!(entries("item_link_added"), want);
    assert!(h.remove_item_link(&c, &s, RelatesTo, &e).await.unwrap());
    assert_eq!(entries("item_link_removed"), want);
}

/// kb/ADR-0010 R5: the row and its entries commit together. At the moment each entry is recorded, the mutation's
/// own transaction already shows the change and another connection does not yet.
#[tokio::test]
async fn an_item_link_and_its_entries_commit_together() {
    use reeve_protocol::ItemLinkType::Follows;
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk_probe(
        pool.clone(),
        "SELECT (SELECT link_type FROM item_links WHERE from_key = $1 OR to_key = $1 LIMIT 1)",
    );
    let c = ctx("dan@example.com", false);
    let mut keys = Vec::new();
    for n in ["a", "b"] {
        keys.push(
            h.create_story(&c, n, n, Kind::Chore, None, "", vec![], vec![], None, None)
                .await
                .unwrap()
                .key()
                .clone(),
        );
    }
    assert!(
        h.add_item_link(&c, &keys[0], Follows, &keys[1])
            .await
            .unwrap()
    );
    let added = sink.entries("item_link_added");
    assert_eq!(added.len(), 2);
    for (rec, inside, outside) in &added {
        assert_eq!(
            inside.as_deref(),
            Some("follows"),
            "{}: written before its entry",
            rec.entity_key
        );
        assert_eq!(
            *outside, None,
            "{}: not committed before its entry",
            rec.entity_key
        );
    }
    assert!(
        h.remove_item_link(&c, &keys[0], Follows, &keys[1])
            .await
            .unwrap()
    );
    let removed = sink.entries("item_link_removed");
    assert_eq!(removed.len(), 2);
    for (rec, inside, outside) in &removed {
        assert_eq!(
            *inside, None,
            "{}: deleted before its entry",
            rec.entity_key
        );
        assert_eq!(
            outside.as_deref(),
            Some("follows"),
            "{}: the delete is not committed yet",
            rec.entity_key
        );
    }
}

/// Linked items of one name and side come in key-number order, whatever their prefix. After a prefix change an
/// instance holds native keys under two prefixes, each numbered by its own counter (`next_key` counts per prefix),
/// and `LONG-2` comes before `A-3`, which comes before `A-10`.
#[tokio::test]
async fn item_links_list_linked_items_by_key_number_across_prefixes() {
    use reeve_protocol::ItemLinkType::Follows;
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let c = ctx("dan@example.com", false);
    // The same instance under its earlier prefix and under its later one. `KeyRepo::next_key` keeps one counter
    // per prefix, so the two stories made under `LONG` are LONG-1 and LONG-2 and the ten made under `A` are A-1
    // to A-10, whatever was made under the other prefix. Every create has an idempotency key of its own.
    let under = |prefix: &str| Handlers {
        settings: Settings {
            key_prefix: prefix.into(),
            ..settings()
        },
        ..h.clone()
    };
    let mut made = Vec::new();
    for (prefix, count) in [("LONG", 2), ("A", 10)] {
        let by = under(prefix);
        let mut keys = Vec::new();
        for n in 1..=count {
            keys.push(
                by.create_story(
                    &c,
                    &format!("{prefix}-story-{n}"),
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
                .unwrap()
                .key()
                .clone(),
            );
        }
        made.push(keys);
    }
    let (long, a) = (&made[0], &made[1]);
    // The item that has the links, and the three it links to: `a[2]` is the third key under `A`, `a[9]` the tenth.
    let (from, second, third, tenth) = (&long[0], &long[1], &a[2], &a[9]);
    assert_eq!(
        (
            from.to_string(),
            second.to_string(),
            third.to_string(),
            tenth.to_string()
        ),
        (
            "LONG-1".to_string(),
            "LONG-2".to_string(),
            "A-3".to_string(),
            "A-10".to_string()
        ),
        "the control: the keys this test is about"
    );
    // Added in an order that is none of the candidate orders.
    for other in [tenth, second, third] {
        assert!(
            h.add_item_link(&c, from, Follows, other).await.unwrap(),
            "{other}"
        );
    }
    let order: Vec<String> = h
        .link_listing(from)
        .await
        .unwrap()
        .links
        .iter()
        .map(|l| l.item.key.to_string())
        .collect();
    assert_eq!(order, vec!["LONG-2", "A-3", "A-10"]);
}

/// What plan apply (GRA-879) relies on when it calls the `_in` forms in a transaction it opens per link: they write
/// on the caller's transaction only. When that transaction is not committed after they ran, no row and no entry is
/// left. The caller here is the public form (begin, the `_in` form, commit), and what stops it short of the commit
/// is the sink refusing the second item's entry, by which time the `_in` form has written the row and the first
/// item's entry.
#[tokio::test]
async fn an_item_link_written_in_a_transaction_that_is_rolled_back_leaves_no_row_and_no_entry() {
    use reeve_protocol::ItemLinkType::Follows;
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk_table_sink(pool.clone()).await;
    let c = ctx("dan@example.com", false);
    let mut keys = Vec::new();
    for n in ["a", "b"] {
        keys.push(
            h.create_story(&c, n, n, Kind::Chore, None, "", vec![], vec![], None, None)
                .await
                .unwrap()
                .key()
                .clone(),
        );
    }
    // (rows of item_links, entries with this action), as another connection sees them: what was committed.
    let left = |action: &'static str| {
        let pool = pool.clone();
        async move {
            let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM item_links")
                .fetch_one(&pool)
                .await
                .unwrap();
            (rows, entries_committed(&pool, action).await)
        }
    };

    // The add: the row and the entry on `a` are written, the entry on `b` is refused.
    sink.refuse(Some(&keys[1]));
    assert!(
        h.add_item_link(&c, &keys[0], Follows, &keys[1])
            .await
            .is_err()
    );
    assert_eq!(
        left("item_link_added").await,
        (0, 0),
        "no row and no entry of the add is left"
    );

    // The control: the same call, not refused, leaves the row and both entries, so the zeros above are the
    // rollback's doing and this sink's table does show an entry that was committed.
    sink.refuse(None);
    assert!(
        h.add_item_link(&c, &keys[0], Follows, &keys[1])
            .await
            .unwrap()
    );
    assert_eq!(left("item_link_added").await, (1, 2), "the control");

    // The remove: the delete and the entry on `a` are written, the entry on `b` is refused.
    sink.refuse(Some(&keys[1]));
    assert!(
        h.remove_item_link(&c, &keys[0], Follows, &keys[1])
            .await
            .is_err()
    );
    assert_eq!(
        left("item_link_removed").await,
        (1, 0),
        "the link is still there and no entry says otherwise"
    );
    sink.refuse(None);
    assert!(
        h.remove_item_link(&c, &keys[0], Follows, &keys[1])
            .await
            .unwrap()
    );
    assert_eq!(left("item_link_removed").await, (0, 2), "the control");
}

/// kb/ADR-0031 R3: item links and web links are read by no gate and change no item. Readiness (from the list, and one story at a
/// time), delivery, resolution, rank, progress, the active sprint's goal with its link and its numbers, and every
/// item's version are the same before any link exists, with every name in place between and on stories, an epic and
/// an initiative, and after they are removed. A story that `follows` an unstarted story still starts, and a blocked
/// story is refused for its blocker alone. And at the query level: computing readiness and moving a story send no
/// statement that names `item_links` or `web_links`.
#[tokio::test]
async fn item_links_and_web_links_change_no_readiness_delivery_progress_rank_or_version() {
    use reeve_protocol::ItemLinkType::{Duplicates, Follows, RelatesTo};
    // `pg_counted`: this database has `pg_stat_statements`, which the query-level check at the end reads.
    let (_c, pool) = common::pg_counted().await;
    let (h, _sink) = mk(pool.clone());
    let c = ctx("dan@example.com", false);
    let i = h
        .create_initiative(&c, "i", "I", "", "Now", None, None, None)
        .await
        .unwrap()
        .key()
        .clone();
    let e = h
        .create_epic(&c, "e", "E", Some(i.clone()), "", None)
        .await
        .unwrap()
        .key()
        .clone();
    h.mark_discovery(&c, ItemType::Initiative, &i, 1, true)
        .await
        .unwrap();
    h.mark_discovery(&c, ItemType::Epic, &e, 1, true)
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
                Some(e.clone()),
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
    // `blocked` waits on `bare`, which is never started here, so its blocker reason holds throughout.
    h.add_block(&c, &keys[2], &keys[1]).await.unwrap();
    h.require_decision(&c, "d", &keys[3], "Pick", "dan@example.com", None, None)
        .await
        .unwrap();
    mark_done_at(&h, &keys[4], c.now).await;
    // An active sprint whose one goal serves the epic, which is one of the linked items below (R3 names "a goal").
    let (goal, _) = h
        .add_goal(&c, "g", "Invoices go out", "one did", None, "", false, &[])
        .await
        .unwrap();
    h.link_goal(&c, goal.id, std::slice::from_ref(&e))
        .await
        .unwrap();
    let draft = h.sprints(c.now).await.unwrap().draft.unwrap();
    h.open_sprint(&c, draft.id, draft.version, None)
        .await
        .unwrap();

    type Snapshot = (
        Vec<(
            Key,
            reeve_protocol::Readiness,
            i32,
            DeliveryState,
            Option<Resolution>,
            i64,
        )>,
        Vec<(Key, Option<f32>, i64)>,
        Vec<(Key, i64)>,
        Vec<reeve_protocol::Readiness>,
        // The goal's view: the active sprint, every goal (its confidence, verdict and version), the goal links,
        // and the numbers the sprint view derives from the stories its goals serve.
        (Option<Sprint>, Vec<Goal>, Vec<GoalLink>, Option<Summary>),
    );
    let snapshot = || async {
        let board = h
            .list_stories(
                &StoryFilter::default(),
                None,
                None,
                false,
                DoneFilter::All,
                c.now,
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
                    r.story.version,
                )
            })
            .collect();
        let initiatives = h
            .list_initiatives(c.now)
            .await
            .unwrap()
            .into_iter()
            .map(|i| (i.key, i.progress, i.version))
            .collect();
        let epics = h
            .epic_dtos(None, c.now)
            .await
            .unwrap()
            .into_iter()
            .map(|e| (e.key, e.version))
            .collect();
        let mut one_at_a_time = Vec::new();
        for k in &keys {
            one_at_a_time.push(h.readiness(k).await.unwrap());
        }
        let sprints = h.sprints(c.now).await.unwrap();
        let goals = (
            sprints.active,
            sprints.goals,
            sprints.links,
            sprints.active_summary,
        );
        let s: Snapshot = (board, initiatives, epics, one_at_a_time, goals);
        s
    };
    let before = snapshot().await;
    assert_eq!(before.0.len(), 5, "every story is in the snapshot");
    assert!(before.0.iter().any(|(_, r, ..)| r.ready), "one is ready");
    assert!(before.0.iter().any(|(_, r, ..)| !r.ready), "one is not");
    assert!(before.1[0].1.is_some(), "the initiative has progress");
    assert!(before.4.0.is_some(), "the sprint is active");
    assert_eq!(
        before.4.2,
        vec![GoalLink {
            goal_id: goal.id,
            item_key: e.clone(),
        }],
        "the goal and its link to a linked item are in the snapshot"
    );

    // Every name, in both directions, between stories and on an epic and an initiative.
    let links = [
        (&keys[0], Follows, &keys[1]),
        (&keys[1], Follows, &keys[0]),
        (&keys[2], Duplicates, &keys[0]),
        (&keys[3], RelatesTo, &keys[4]),
        (&keys[0], Follows, &e),
        (&e, Duplicates, &i),
        (&i, RelatesTo, &keys[2]),
        (&e, Follows, &keys[4]),
    ];
    for (from, t, to) in links {
        assert!(
            h.add_item_link(&c, from, t, to).await.unwrap(),
            "{from} {t:?} {to}"
        );
    }
    assert_eq!(
        h.link_listing(&keys[0]).await.unwrap().links.len(),
        4,
        "the control: the links are in place"
    );
    // A web link on stories in every state of readiness, on the epic and on the initiative.
    let linked: Vec<&Key> = keys.iter().chain([&e, &i]).collect();
    for (n, k) in linked.iter().enumerate() {
        assert!(
            h.add_web_link(&c, k, "Page", &format!("https://example.com/{n}"))
                .await
                .unwrap(),
            "{k}"
        );
    }
    assert_eq!(
        h.link_listing(&keys[0]).await.unwrap().web_links.len(),
        1,
        "the control: the web links are in place"
    );
    assert_eq!(snapshot().await, before, "with every link in place");

    for (from, t, to) in links {
        assert!(
            h.remove_item_link(&c, from, t, to).await.unwrap(),
            "{from} {t:?} {to}"
        );
    }
    for k in &linked {
        let id = h.link_listing(k).await.unwrap().web_links[0].id;
        assert!(h.remove_web_link(&c, k, id).await.unwrap(), "{k}");
    }
    assert_eq!(snapshot().await, before, "after removing them");

    // Moving reads none of it: `ready` follows the unstarted `bare` and still starts; `blocked` also follows
    // `ready` and is refused for exactly the reasons it had before any link existed.
    h.add_item_link(&c, &keys[0], Follows, &keys[1])
        .await
        .unwrap();
    h.add_item_link(&c, &keys[2], Follows, &keys[0])
        .await
        .unwrap();
    // Web links are in place through the moves too: on the story that starts and on the one that is refused.
    for (n, k) in [&keys[0], &keys[2]].into_iter().enumerate() {
        assert!(
            h.add_web_link(&c, k, "Page", &format!("https://example.com/moved/{n}"))
                .await
                .unwrap(),
            "{k}"
        );
    }
    let blocked_reasons = before.3[2].reasons.clone();
    assert!(
        !blocked_reasons.is_empty(),
        "the control: `blocked` is not ready"
    );
    let version = |k: &Key| {
        let (h, k) = (h.clone(), k.clone());
        async move { h.get_story(&k).await.unwrap().version() }
    };

    // Query level. `common::statements` resets this database's `pg_stat_statements` entries and runs the block;
    // afterwards the view holds the text of every statement the block sent. `naming` counts those that name a
    // table. The count's own statement names none: the table's name is a bound parameter.
    let naming = |table: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM pg_stat_statements \
                 WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
                 AND query ILIKE '%' || $1 || '%'",
            )
            .bind(table)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    // The control: a read of the table is seen when there is one, so a zero below means "no statement", not "the
    // view records nothing".
    common::statements(&pool, || async {
        h.link_listing(&keys[0]).await.unwrap();
    })
    .await;
    assert!(
        naming("item_links").await > 0,
        "the control: a read of item_links is seen"
    );
    assert!(
        naming("web_links").await > 0,
        "the control: a read of web_links is seen"
    );
    common::statements(&pool, || async {
        // Readiness: one story at a time, and from the list.
        for k in &keys {
            h.readiness(k).await.unwrap();
        }
        h.list_stories(
            &StoryFilter::default(),
            None,
            None,
            false,
            DoneFilter::All,
            c.now,
        )
        .await
        .unwrap();
        // A move that passes the gate, and one the gate refuses.
        let v = version(&keys[0]).await;
        let started = h
            .move_story(&c, &keys[0], v, DeliveryState::InProgress, None, None)
            .await
            .unwrap();
        assert_eq!(started.delivery(), DeliveryState::InProgress);
        let v = version(&keys[2]).await;
        match h
            .move_story(&c, &keys[2], v, DeliveryState::InProgress, None, None)
            .await
        {
            Err(HandlerError::Refused(reasons)) => assert_eq!(reasons, blocked_reasons),
            other => panic!("the blocked story must stay refused: {other:?}"),
        }
    })
    .await;
    assert_eq!(
        naming("item_links").await,
        0,
        "computing readiness and moving a story send no statement that names item_links"
    );
    assert_eq!(
        naming("web_links").await,
        0,
        "computing readiness and moving a story send no statement that names web_links"
    );
}

/// The reads of one item (story, epic, initiative, and the story of a context) always carry `links` and
/// `web_links`, an empty list when there are none; every read of several items, and the items that come along in
/// a context, leave both out, in Rust and in the JSON.
#[tokio::test]
async fn single_item_reads_carry_links_and_web_links_and_list_reads_do_not() {
    use reeve_protocol::ItemLinkType::{Follows, RelatesTo};
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let c = ctx("dan@example.com", false);
    let now = c.now;
    let i = h
        .create_initiative(&c, "i", "I", "", "Now", None, None, None)
        .await
        .unwrap()
        .key()
        .clone();
    let e = h
        .create_epic(&c, "e", "E", Some(i.clone()), "", None)
        .await
        .unwrap()
        .key()
        .clone();
    let mut s = Vec::new();
    for n in ["a", "b", "lonely"] {
        s.push(
            h.create_story(
                &c,
                n,
                n,
                Kind::Story,
                Some(e.clone()),
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
    assert!(h.add_item_link(&c, &s[0], Follows, &s[1]).await.unwrap());
    assert!(h.add_item_link(&c, &e, RelatesTo, &i).await.unwrap());
    h.add_block(&c, &s[0], &s[1]).await.unwrap();
    assert!(
        h.add_web_link(&c, &s[0], "Page", "https://example.com/s")
            .await
            .unwrap()
    );
    assert!(
        h.add_web_link(&c, &e, "Page", "https://example.com/e")
            .await
            .unwrap()
    );
    assert!(
        h.add_web_link(&c, &i, "Page", "https://example.com/i")
            .await
            .unwrap()
    );
    let urls = |links: &Option<Vec<reeve_protocol::WebLinkDto>>| -> Option<Vec<String>> {
        links
            .as_ref()
            .map(|l| l.iter().map(|w| w.url.clone()).collect())
    };

    let shown = |links: &Option<Vec<reeve_protocol::ItemLinkDto>>| -> Option<Vec<String>> {
        links.as_ref().map(|l| {
            l.iter()
                .map(|x| format!("{} {}", x.label, x.item.key))
                .collect()
        })
    };
    assert_eq!(
        shown(&h.story_dto(&s[0], now).await.unwrap().links),
        Some(vec![format!("follows {}", s[1])])
    );
    assert_eq!(
        shown(&h.story_dto(&s[1], now).await.unwrap().links),
        Some(vec![format!("is followed by {}", s[0])])
    );
    assert_eq!(
        shown(&h.story_dto(&s[2], now).await.unwrap().links),
        Some(vec![]),
        "none is an empty list, not an absent one"
    );
    assert_eq!(
        shown(&h.epic_dto(&e, now).await.unwrap().links),
        Some(vec![format!("relates to {i}")])
    );
    assert_eq!(
        shown(&h.initiative_dto(&i, now).await.unwrap().links),
        Some(vec![format!("relates to {e}")])
    );
    assert_eq!(
        urls(&h.story_dto(&s[0], now).await.unwrap().web_links),
        Some(vec!["https://example.com/s".to_string()])
    );
    assert_eq!(
        urls(&h.story_dto(&s[2], now).await.unwrap().web_links),
        Some(vec![]),
        "none is an empty list, not an absent one"
    );
    assert_eq!(
        urls(&h.epic_dto(&e, now).await.unwrap().web_links),
        Some(vec!["https://example.com/e".to_string()])
    );
    assert_eq!(
        urls(&h.initiative_dto(&i, now).await.unwrap().web_links),
        Some(vec!["https://example.com/i".to_string()])
    );
    let context = h.story_context(&s[0], now).await.unwrap();
    assert_eq!(
        shown(&context.story.links),
        Some(vec![format!("follows {}", s[1])])
    );
    assert_eq!(
        urls(&context.story.web_links),
        Some(vec!["https://example.com/s".to_string()])
    );
    assert_eq!(
        context.blockers.len(),
        1,
        "the control: a blocker comes along"
    );
    assert_eq!(context.blockers[0].links, None);
    assert_eq!(context.blockers[0].web_links, None);
    // The epic and the initiative have links and web links of their own (added above), so `None` here means
    // "not loaded", not "none".
    let epic = context.epic.unwrap();
    assert_eq!((epic.links, epic.web_links), (None, None));
    let initiative = context.initiative.unwrap();
    assert_eq!((initiative.links, initiative.web_links), (None, None));

    let listed = h
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
        listed.len(),
        3,
        "the control: the list holds the linked stories"
    );
    assert!(
        listed
            .iter()
            .all(|r| r.story.links.is_none() && r.story.web_links.is_none())
    );
    let row = serde_json::to_value(&listed[0]).unwrap();
    assert!(
        row["story"].get("links").is_none() && row["story"].get("web_links").is_none(),
        "left out of the JSON, not sent as null: {row}"
    );
    assert!(
        h.epic_dtos(None, now)
            .await
            .unwrap()
            .iter()
            .all(|e| e.links.is_none() && e.web_links.is_none())
    );
    assert!(
        h.list_initiatives(now)
            .await
            .unwrap()
            .iter()
            .all(|i| i.links.is_none() && i.web_links.is_none())
    );
}

/// kb/ADR-0031 R5, R7. A web link is on one item, one per URL (compared as stored), listed oldest first, logged
/// on its item on each change and never for a no-op, and removed by its id on that item only.
#[tokio::test]
async fn web_links_are_one_per_url_logged_on_their_item_and_removed_by_id() {
    use serde_json::json;
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);
    let bot = ctx("bot@example.com", true);
    let e = h
        .create_epic(&dan, "e", "Invoices", None, "", None)
        .await
        .unwrap()
        .key()
        .clone();
    let s = h
        .create_story(
            &dan,
            "s",
            "Template",
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
        .clone();
    let entries = |action: &str| -> Vec<(String, String, serde_json::Value)> {
        sink.0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.action == action)
            .map(|r| {
                (
                    r.entity_type.to_string(),
                    r.entity_key.clone(),
                    r.payload.clone(),
                )
            })
            .collect()
    };
    let web = |k: &Key| {
        let (h, k) = (h.clone(), k.clone());
        async move { h.link_listing(&k).await.unwrap().web_links }
    };
    let scenario = "https://example.com/wiki/scenario";

    // Added by an automation caller, stored trimmed and in the parser's form.
    assert!(
        h.add_web_link(
            &bot,
            &e,
            "  Scenario page ",
            "HTTPS://Example.com/wiki/scenario"
        )
        .await
        .unwrap()
    );
    let listed = web(&e).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (listed[0].title.as_str(), listed[0].url.as_str()),
        ("Scenario page", scenario)
    );
    let id = listed[0].id;
    let added = json!({"id": id, "title": "Scenario page", "url": scenario});
    assert_eq!(
        entries("web_link_added"),
        vec![("epic".to_string(), e.to_string(), added.clone())]
    );

    // The same URL in another spelling, same title: already there.
    assert!(
        !h.add_web_link(
            &bot,
            &e,
            "Scenario page",
            "https://EXAMPLE.com/wiki/scenario"
        )
        .await
        .unwrap()
    );
    // The same URL with another title is not an edit: remove it first.
    assert!(matches!(
        h.add_web_link(&bot, &e, "Scenario", scenario).await,
        Err(HandlerError::Validation(m))
            if m == "this item already has a web link to that URL, titled \"Scenario page\": remove it first, then add it with the new title"
    ));
    assert_eq!(entries("web_link_added").len(), 1, "neither wrote an entry");
    assert_eq!(web(&e).await, listed, "and neither changed the list");

    // The same URL on another item, and a second one on the epic, which lists after the first.
    assert!(
        h.add_web_link(&dan, &s, "Scenario page", scenario)
            .await
            .unwrap()
    );
    assert!(
        h.add_web_link(&dan, &e, "Board", "http://example.com/board")
            .await
            .unwrap()
    );
    let titles = |links: &[reeve_protocol::WebLinkDto]| -> Vec<String> {
        links.iter().map(|w| w.title.clone()).collect()
    };
    assert_eq!(titles(&web(&e).await), vec!["Scenario page", "Board"]);
    assert_eq!(entries("web_link_added")[1].0, "story");

    // Removed by id, on its own item only.
    assert!(
        !h.remove_web_link(&bot, &s, id).await.unwrap(),
        "the epic's id on the story"
    );
    assert!(h.remove_web_link(&bot, &e, id).await.unwrap());
    assert!(
        !h.remove_web_link(&bot, &e, id).await.unwrap(),
        "already gone"
    );
    assert_eq!(
        entries("web_link_removed"),
        vec![("epic".to_string(), e.to_string(), added)],
        "the entry keeps what was removed"
    );
    assert_eq!(titles(&web(&e).await), vec!["Board"]);
    assert_eq!(
        web(&s).await.len(),
        1,
        "the story's own web link is untouched"
    );

    // Refusals write nothing: no entry, and no row. A URL with a user name or a password is among them, so a
    // credential typed into one reaches neither the table nor the activity log.
    let before = sink.0.lock().unwrap().len();
    let stored = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM web_links")
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    let rows = stored().await;
    assert_eq!(
        rows, 2,
        "the control: the epic's Board and the story's Scenario page are stored"
    );
    for (title, url) in [
        ("", "https://example.com/"),
        ("ok", "ftp://example.com/x"),
        ("ok", "javascript:alert(1)"),
        ("ok", "example.com/wiki"),
        ("ok", "https://mia:secret@example.com/wiki"),
        ("ok", "https://mia@example.com/wiki"),
    ] {
        assert!(
            matches!(
                h.add_web_link(&bot, &e, title, url).await,
                Err(HandlerError::Validation(_))
            ),
            "{title:?} {url:?}"
        );
    }
    let ghost: Key = "ABC-999".parse().unwrap();
    assert!(matches!(
        h.add_web_link(&bot, &ghost, "ok", "https://example.com/")
            .await,
        Err(HandlerError::NotFound)
    ));
    assert!(matches!(
        h.remove_web_link(&bot, &ghost, id).await,
        Err(HandlerError::NotFound)
    ));
    assert_eq!(
        sink.0.lock().unwrap().len(),
        before,
        "no entry for a refusal"
    );
    assert_eq!(stored().await, rows, "and no row");
    assert!(
        sink.0
            .lock()
            .unwrap()
            .iter()
            .all(|r| !r.payload.to_string().contains("secret")),
        "nothing of the refused URL is in the log"
    );
}

/// `add_web_link_reporting` answers the id a web link is removed by: a new one's, and on a repeat the id of the
/// one that is there.
#[tokio::test]
async fn adding_a_web_link_reports_its_id_and_whether_it_was_added() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);
    let e = h
        .create_epic(&dan, "wr-e", "Invoices", None, "", None)
        .await
        .unwrap()
        .key()
        .clone();
    let (first, added) = h
        .add_web_link_reporting(&dan, &e, "Spec", "https://example.com/spec")
        .await
        .unwrap();
    assert!(added);
    let first = first.id;
    let (second, added) = h
        .add_web_link_reporting(&dan, &e, "Board", "https://example.com/board")
        .await
        .unwrap();
    assert!(added);
    let second = second.id;
    // The ids are the listing's own, in its order.
    let listed: Vec<(i64, String)> = h
        .link_listing(&e)
        .await
        .unwrap()
        .web_links
        .into_iter()
        .map(|w| (w.id, w.title))
        .collect();
    assert_eq!(
        listed,
        vec![(first, "Spec".to_string()), (second, "Board".to_string())]
    );
    // A repeat, in another spelling of the URL: the id of the one that is there, and nothing added.
    let (again, added) = h
        .add_web_link_reporting(&dan, &e, "Spec", "HTTPS://Example.com/spec")
        .await
        .unwrap();
    assert_eq!((again.id, added), (first, false));
    // And that id removes it.
    assert!(h.remove_web_link(&dan, &e, first).await.unwrap());
}

/// `add_web_link_reporting` answers the web link as stored, not as sent: the trimmed title and the URL as the URL
/// parser writes it, for the add (the row it wrote) and for a repeat sent in another spelling (the row it matched).
/// The MCP receipt is built from this answer, never from a read after the commit. Catches an answer that echoes
/// the title and URL as sent, and one whose title or URL is not the stored row's (the listing is the stored rows).
#[tokio::test]
async fn adding_a_web_link_reports_it_as_stored_for_the_add_and_the_repeat() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);
    let e = h
        .create_epic(&dan, "ws-e", "Invoices", None, "", None)
        .await
        .unwrap()
        .key()
        .clone();
    let (added, changed) = h
        .add_web_link_reporting(&dan, &e, "  Spec  ", "HTTPS://Example.com")
        .await
        .unwrap();
    assert!(changed);
    let listed = h.web_links_of(&e).await.unwrap();
    assert_eq!(
        listed,
        vec![added.clone()],
        "the add answers the row it wrote"
    );
    assert_eq!(
        (added.title.as_str(), added.url.as_str()),
        ("Spec", "https://example.com/")
    );
    let (again, changed) = h
        .add_web_link_reporting(&dan, &e, "Spec ", "https://EXAMPLE.com/")
        .await
        .unwrap();
    assert!(!changed);
    assert_eq!(again, added, "the repeat answers the row it matched");
}

/// What plan apply (GRA-879) relies on when it calls the `_in` forms in a transaction it opens per link: they write
/// on the caller's transaction only, so nothing of them is left when it is not committed.
/// As for item links (Task 3), the caller is the public form and what stops it short of the commit is the sink
/// refusing the entry, by which time the `_in` form has written the row.
#[tokio::test]
async fn a_web_link_written_in_a_transaction_that_is_rolled_back_leaves_no_row_and_no_entry() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk_table_sink(pool.clone()).await;
    let c = ctx("dan@example.com", false);
    let s = h
        .create_story(
            &c,
            "s",
            "s",
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
        .clone();
    // (rows of web_links, entries with this action), as another connection sees them: what was committed.
    let left = |action: &'static str| {
        let pool = pool.clone();
        async move {
            let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM web_links")
                .fetch_one(&pool)
                .await
                .unwrap();
            (rows, entries_committed(&pool, action).await)
        }
    };

    sink.refuse(Some(&s));
    assert!(
        h.add_web_link(&c, &s, "Page", "https://example.com/")
            .await
            .is_err()
    );
    assert_eq!(
        left("web_link_added").await,
        (0, 0),
        "no row and no entry of the add is left"
    );
    // The control: the same call, not refused, leaves the row and its entry.
    sink.refuse(None);
    assert!(
        h.add_web_link(&c, &s, "Page", "https://example.com/")
            .await
            .unwrap()
    );
    assert_eq!(left("web_link_added").await, (1, 1), "the control");

    let id = h.link_listing(&s).await.unwrap().web_links[0].id;
    sink.refuse(Some(&s));
    assert!(h.remove_web_link(&c, &s, id).await.is_err());
    assert_eq!(
        left("web_link_removed").await,
        (1, 0),
        "the web link is still there and no entry says otherwise"
    );
    sink.refuse(None);
    assert!(h.remove_web_link(&c, &s, id).await.unwrap());
    assert_eq!(left("web_link_removed").await, (0, 1), "the control");
}

/// kb/ADR-0010 R5: a web link and its entry commit together.
#[tokio::test]
async fn a_web_link_and_its_entry_commit_together() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk_probe(
        pool.clone(),
        "SELECT (SELECT title FROM web_links WHERE item_key = $1 LIMIT 1)",
    );
    let c = ctx("dan@example.com", false);
    let s = h
        .create_story(
            &c,
            "s",
            "s",
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
        .clone();
    assert!(
        h.add_web_link(&c, &s, "Page", "https://example.com/")
            .await
            .unwrap()
    );
    let added = sink.entries("web_link_added");
    assert_eq!(added.len(), 1);
    assert_eq!(
        added[0].1.as_deref(),
        Some("Page"),
        "written before its entry"
    );
    assert_eq!(added[0].2, None, "not committed before its entry");
    let id = h.link_listing(&s).await.unwrap().web_links[0].id;
    assert!(h.remove_web_link(&c, &s, id).await.unwrap());
    let removed = sink.entries("web_link_removed");
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].1, None, "deleted before its entry");
    assert_eq!(
        removed[0].2.as_deref(),
        Some("Page"),
        "the delete is not committed yet"
    );
}

/// kb/ADR-0010 R5 and kb/ADR-0031 R7 for a plan: each item link and each web link is a transaction of its own,
/// holding its row and its entries. The plan's items exist already (an earlier apply made them). The sink refuses
/// the entry on `c`, which the plan's second item link names: by then the first item link (`a` follows `b`) and
/// its two entries are committed, and the second link's row is written. The first link stays, the second link's
/// row does not, and the web link after it is never reached. Applying the plan again, once the sink refuses
/// nothing, finishes it.
#[tokio::test]
async fn a_plan_link_whose_entry_is_refused_is_left_out_and_the_earlier_links_stay() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk_table_sink(pool.clone()).await;
    let dan = ctx("dan@example.com", false);
    let plan = |with_links: bool| -> reeve_protocol::ApplyPlan {
        let mut v = serde_json::json!({
            "id": "one-by-one",
            "stories": [
                {"name": "a", "title": "A"},
                {"name": "b", "title": "B"},
                {"name": "c", "title": "C"}
            ]
        });
        if with_links {
            v["links"] = serde_json::json!([
                {"from": "a", "type": "follows", "to": "b"},
                {"from": "c", "type": "follows", "to": "a"}
            ]);
            v["web_links"] =
                serde_json::json!([{"on": "b", "title": "Page", "url": "https://example.com/"}]);
        }
        serde_json::from_value(v).unwrap()
    };
    let made = h.apply_plan(&dan, plan(false)).await.unwrap();
    let c_key = made
        .items
        .iter()
        .find(|i| i.name == "c")
        .unwrap()
        .key
        .clone();
    // (item link rows, web link rows, item link entries, web link entries) as another connection sees them.
    let left = || {
        let pool = pool.clone();
        async move {
            let count = |table: &'static str| {
                let pool = pool.clone();
                async move {
                    sqlx::query_scalar::<_, i64>(&format!("SELECT count(*) FROM {table}"))
                        .fetch_one(&pool)
                        .await
                        .unwrap()
                }
            };
            (
                count("item_links").await,
                count("web_links").await,
                entries_committed(&pool, "item_link_added").await,
                entries_committed(&pool, "web_link_added").await,
            )
        }
    };

    assert_eq!(
        left().await,
        (0, 0, 0, 0),
        "the control: nothing before the plan's links"
    );

    sink.refuse(Some(&c_key));
    // The control for the refusal below: replaying the items records nothing, so a plan without links is not
    // refused, and the failure that follows is the second link's entry on `c` and nothing earlier.
    assert!(
        h.apply_plan(&dan, plan(false)).await.is_ok(),
        "the control: a replay of the items writes no entry"
    );
    assert!(h.apply_plan(&dan, plan(true)).await.is_err());
    assert_eq!(
        left().await,
        (1, 0, 2, 0),
        "the first link and its two entries stay; the refused link left no row and no entry, and the web link \
         after it was not written"
    );

    // Applying the same plan again, not refused, finishes it: the first link is there, the rest is created.
    sink.refuse(None);
    let applied = h.apply_plan(&dan, plan(true)).await.unwrap();
    assert_eq!(
        (
            applied.links[0].created,
            applied.links[1].created,
            applied.web_links[0].created
        ),
        (false, true, true)
    );
    assert_eq!(left().await, (2, 1, 4, 1));
}

/// Two applies of one plan at once both answer, with the same complete lists; each item link and web link is
/// reported created by exactly one of them, and is stored and logged exactly once. Each round uses a plan id of
/// its own, so nothing of the round exists when the two start.
#[tokio::test]
async fn concurrent_applies_of_one_plan_report_each_link_created_once() {
    use reeve_protocol::{ItemLinkType, ItemType, PlannedLink, PlannedWebLink};
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk_table_sink(pool.clone()).await;
    let dan = ctx("dan@example.com", false);
    let count = |table: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(&format!("SELECT count(*) FROM {table}"))
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    // Everything an answer says about a link but `created`.
    let link_ids = |v: &[PlannedLink]| {
        v.iter()
            .map(|l| {
                (
                    l.from.clone(),
                    l.from_level,
                    l.link_type,
                    l.to.clone(),
                    l.to_level,
                )
            })
            .collect::<Vec<_>>()
    };
    let web_ids = |v: &[PlannedWebLink]| {
        v.iter()
            .map(|w| (w.on.clone(), w.level, w.title.clone(), w.url.clone()))
            .collect::<Vec<_>>()
    };
    for round in 0..5_i64 {
        let plan: reeve_protocol::ApplyPlan = serde_json::from_value(serde_json::json!({
            "id": format!("race-{round}"),
            "epics": [{"name": "e", "title": "E"}],
            "stories": [{"name": "a", "title": "A", "epic": "e"}, {"name": "b", "title": "B", "epic": "e"}],
            "links": [
                {"from": "b", "type": "follows", "to": "a"},
                {"from": "b", "type": "relates_to", "to": "e"}
            ],
            "web_links": [{"on": "e", "title": "Page", "url": "https://example.com/page"}]
        }))
        .unwrap();
        let (x, y) = tokio::join!(
            h.apply_plan(&dan, plan.clone()),
            h.apply_plan(&dan, plan.clone())
        );
        let (x, y) = (x.unwrap(), y.unwrap());
        let key = |name: &str| x.items.iter().find(|i| i.name == name).unwrap().key.clone();
        let (e, a, b) = (key("e"), key("a"), key("b"));

        // Both answers are complete: the lengths first, so that nothing below compares against a short list.
        assert_eq!(
            (x.links.len(), x.web_links.len()),
            (2, 1),
            "round {round}: x"
        );
        assert_eq!(
            (y.links.len(), y.web_links.len()),
            (2, 1),
            "round {round}: y"
        );
        // Both name the same links, whole (keys, levels and name), in the plan's order.
        assert_eq!(link_ids(&x.links), link_ids(&y.links), "round {round}");
        assert_eq!(
            web_ids(&x.web_links),
            web_ids(&y.web_links),
            "round {round}"
        );
        // And they are the plan's links, on this round's items.
        assert_eq!(
            link_ids(&x.links)[0],
            (
                b.clone(),
                ItemType::Story,
                ItemLinkType::Follows,
                a.clone(),
                ItemType::Story
            ),
            "round {round}"
        );
        let related = link_ids(&x.links)[1].clone();
        assert!(
            related
                == (
                    e.clone(),
                    ItemType::Epic,
                    ItemLinkType::RelatesTo,
                    b.clone(),
                    ItemType::Story
                )
                || related
                    == (
                        b.clone(),
                        ItemType::Story,
                        ItemLinkType::RelatesTo,
                        e.clone(),
                        ItemType::Epic
                    ),
            "round {round}: the relates_to joins the epic and b, each with its own level: {related:?}"
        );
        assert_eq!(
            web_ids(&x.web_links),
            vec![(
                e.clone(),
                ItemType::Epic,
                "Page".to_string(),
                "https://example.com/page".to_string()
            )],
            "round {round}"
        );
        // Each was created by exactly one of the two applies.
        for i in 0..2 {
            assert!(
                x.links[i].created != y.links[i].created,
                "round {round}: {:?} was created by exactly one apply",
                link_ids(&x.links)[i]
            );
        }
        assert!(
            x.web_links[0].created != y.web_links[0].created,
            "round {round}: the web link was created by exactly one apply"
        );
        // What is committed agrees with what was reported: one row per link, one entry per item a link names.
        let rounds = round + 1;
        assert_eq!(
            count("item_links").await,
            2 * rounds,
            "round {round}: item link rows"
        );
        assert_eq!(
            count("web_links").await,
            rounds,
            "round {round}: web link rows"
        );
        assert_eq!(
            entries_committed(&pool, "item_link_added").await,
            4 * rounds,
            "round {round}: two entries per item link"
        );
        assert_eq!(
            entries_committed(&pool, "web_link_added").await,
            rounds,
            "round {round}: one entry per web link"
        );
    }
}

/// kb/ADR-0010 R5 and kb/ADR-0031 R7 for a plan's web links: each is a transaction of its own, holding its row
/// and its entry. The plan's items exist already (an earlier apply made them). The sink refuses the entry on `b`,
/// which the plan's second web link is on: by then the first web link (on `a`) and its entry are committed, and
/// the second's row is written. The first stays with its entry, the second leaves neither row nor entry, and
/// applying the same plan again, once the sink refuses nothing, answers the first as existing and creates the
/// second.
#[tokio::test]
async fn a_plan_web_link_whose_entry_is_refused_is_left_out_and_the_earlier_web_link_stays() {
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk_table_sink(pool.clone()).await;
    let dan = ctx("dan@example.com", false);
    let plan = |with_web_links: bool| -> reeve_protocol::ApplyPlan {
        let mut v = serde_json::json!({
            "id": "web-one-by-one",
            "stories": [
                {"name": "a", "title": "A"},
                {"name": "b", "title": "B"}
            ]
        });
        if with_web_links {
            v["web_links"] = serde_json::json!([
                {"on": "a", "title": "Page A", "url": "https://example.com/a"},
                {"on": "b", "title": "Page B", "url": "https://example.com/b"}
            ]);
        }
        serde_json::from_value(v).unwrap()
    };
    let made = h.apply_plan(&dan, plan(false)).await.unwrap();
    let key = |name: &str| {
        made.items
            .iter()
            .find(|i| i.name == name)
            .unwrap()
            .key
            .clone()
    };
    let (a_key, b_key) = (key("a"), key("b"));
    // (web link rows, web link entries) as another connection sees them.
    let left = || {
        let pool = pool.clone();
        async move {
            (
                sqlx::query_scalar::<_, i64>("SELECT count(*) FROM web_links")
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                entries_committed(&pool, "web_link_added").await,
            )
        }
    };

    assert_eq!(
        left().await,
        (0, 0),
        "the control: nothing before the plan's web links"
    );

    sink.refuse(Some(&b_key));
    // The control for the refusal below: replaying the items records nothing, so a plan without web links is not
    // refused, and the failure that follows is the second web link's entry on `b` and nothing earlier.
    assert!(
        h.apply_plan(&dan, plan(false)).await.is_ok(),
        "the control: a replay of the items writes no entry"
    );
    assert!(h.apply_plan(&dan, plan(true)).await.is_err());
    assert_eq!(
        left().await,
        (1, 1),
        "the first web link and its entry stay; the refused one left no row and no entry"
    );

    // Applying the same plan again, not refused: the first web link was there, the second is created.
    sink.refuse(None);
    let applied = h.apply_plan(&dan, plan(true)).await.unwrap();
    assert_eq!(
        applied
            .web_links
            .iter()
            .map(|w| (w.on.clone(), w.created))
            .collect::<Vec<_>>(),
        vec![(a_key, false), (b_key, true)]
    );
    assert_eq!(left().await, (2, 2));
}

/// A reference in a plan is resolved for the place it stands in. `OLD-1` is the native key of an epic (issued
/// under an earlier prefix) and the external reference of a newer story. A blocker's end is a story, so it names
/// the story; an item link's end, a web link's `on` and a decision's `on` are any item, resolved as `resolve_any`
/// does (native keys of any level first), so they name the epic, which is what `reeve link add OLD-1 …` does.
/// Neither answer is taken for the other, whichever the plan asks for first.
#[tokio::test]
async fn a_plan_reference_that_is_a_native_key_and_an_external_reference_names_each_in_its_place() {
    use reeve_protocol::ItemType;
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk_table_sink(pool.clone()).await;
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
    let story = |idem: &'static str, title: &'static str| {
        let (h, c) = (&h, &c);
        async move {
            h.create_story(
                c,
                idem,
                title,
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
    let imported = story("is", "Imported").await;
    let held = story("held", "Held").await;
    sqlx::query("UPDATE stories SET external_ref = $1 WHERE key = $2")
        .bind(&raw)
        .bind(imported.to_string())
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        (
            h.resolve_ref(&raw, ItemType::Story).await.unwrap(),
            h.resolve_any(&raw).await.unwrap()
        ),
        (imported.clone(), (ItemType::Epic, old_epic.key().clone())),
        "the control: as a story the reference is the imported story, untyped it is the epic"
    );

    // The blocker is resolved first and names the story; the item link and the web link name the epic.
    let plan: reeve_protocol::ApplyPlan = serde_json::from_value(serde_json::json!({
        "id": "collision",
        "blocks": [{"story": held, "blocked_by": raw}],
        "links": [{"from": held, "type": "follows", "to": raw}],
        "web_links": [{"on": raw, "title": "Page", "url": "https://example.com/"}]
    }))
    .unwrap();
    let applied = h.apply_plan(&c, plan).await.unwrap();
    assert_eq!(applied.blocks[0].blocked_by, imported, "the blocker");
    assert_eq!(
        (&applied.links[0].to, applied.links[0].to_level),
        (old_epic.key(), ItemType::Epic),
        "the item link"
    );
    assert_eq!(
        (&applied.web_links[0].on, applied.web_links[0].level),
        (old_epic.key(), ItemType::Epic),
        "the web link"
    );
    let on_epic = h.link_listing(old_epic.key()).await.unwrap();
    assert_eq!(
        (on_epic.links.len(), on_epic.web_links.len()),
        (1, 1),
        "both are on the epic"
    );
    let on_story = h.link_listing(&imported).await.unwrap();
    assert_eq!(
        (on_story.links.len(), on_story.web_links.len()),
        (0, 0),
        "and none on the story that has the reference"
    );

    // The other way round: a decision's `on` is resolved before the blocker and names the epic; the blocker still
    // names the story.
    let plan: reeve_protocol::ApplyPlan = serde_json::from_value(serde_json::json!({
        "id": "collision-2",
        "decisions": [{"name": "d", "on": raw, "title": "Pick", "decider": "dan@example.com"}],
        "blocks": [{"story": held, "blocked_by": raw}]
    }))
    .unwrap();
    let applied = h.apply_plan(&c, plan).await.unwrap();
    assert_eq!(applied.blocks[0].blocked_by, imported);
    let decisions = h
        .list_decisions(Some(old_epic.key()), false, true)
        .await
        .unwrap();
    assert_eq!(decisions.len(), 1, "the decision is on the epic");
}

/// The scopes the other way round: `OLD-1` is the native key of a story (issued under an earlier prefix) and the
/// external reference of an epic. A story's `epic` takes an epic, so it names the epic; a decision's `on` takes any
/// item, native keys first, so it names the story. A re-apply answers the same decision on the same story and
/// creates nothing. What it catches: one cache of resolved references shared by the typed and the untyped scope,
/// which would hand the decision the epic the story's `epic` resolved first (or the epic the story), and on the
/// re-apply move or duplicate the decision.
#[tokio::test]
async fn a_plan_decision_on_a_native_story_key_that_is_an_epic_external_reference_is_on_the_story()
{
    use reeve_protocol::ItemType;
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk_table_sink(pool.clone()).await;
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
        .unwrap()
        .key()
        .clone();
    let raw = old_story.to_string();
    let epic = h
        .create_epic(&c, "ie", "Imported epic", None, "", None)
        .await
        .unwrap()
        .key()
        .clone();
    sqlx::query("UPDATE epics SET external_ref = $1 WHERE key = $2")
        .bind(&raw)
        .bind(epic.to_string())
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        (
            h.resolve_ref(&raw, ItemType::Epic).await.unwrap(),
            h.resolve_any(&raw).await.unwrap()
        ),
        (epic.clone(), (ItemType::Story, old_story.clone())),
        "the control: as an epic the reference is the imported epic, untyped it is the story"
    );

    let plan = || -> reeve_protocol::ApplyPlan {
        serde_json::from_value(serde_json::json!({
            "id": "collision-3",
            "stories": [{"name": "s", "title": "S", "epic": raw}],
            "decisions": [{"name": "d", "on": raw, "title": "Pick", "decider": "dan@example.com"}]
        }))
        .unwrap()
    };
    let counts = || {
        let pool = pool.clone();
        async move {
            sqlx::query_as::<_, (i64, i64, i64)>(
                "SELECT (SELECT count(*) FROM stories), (SELECT count(*) FROM epics), \
                 (SELECT count(*) FROM required_decisions)",
            )
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let first = h.apply_plan(&c, plan()).await.unwrap();
    let s = h.get_story(&first.items[0].key).await.unwrap();
    assert_eq!(s.epic(), Some(&epic), "the story lands under the epic");
    let on_story = h
        .list_decisions(Some(&old_story), false, true)
        .await
        .unwrap();
    assert_eq!(
        on_story
            .iter()
            .map(|d| (d.id(), d.on_type()))
            .collect::<Vec<_>>(),
        vec![(first.decisions[0].id, ItemType::Story)],
        "the decision is on the native story"
    );
    assert!(
        h.list_decisions(Some(&epic), false, true)
            .await
            .unwrap()
            .is_empty(),
        "and none on the epic"
    );

    let before = counts().await;
    let again = h.apply_plan(&c, plan()).await.unwrap();
    assert_eq!(counts().await, before, "the re-apply creates nothing");
    assert_eq!(
        (again.items[0].created, again.decisions[0].created),
        (false, false)
    );
    assert_eq!(
        again.decisions[0].id, first.decisions[0].id,
        "the same decision"
    );
    let on_story = h
        .list_decisions(Some(&old_story), false, true)
        .await
        .unwrap();
    assert_eq!(
        on_story
            .iter()
            .map(|d| (d.id(), d.on()))
            .collect::<Vec<_>>(),
        vec![(first.decisions[0].id, &old_story)],
        "still on the native story"
    );
}

/// A replay of a plan opens no write transaction for a link that is already there: the item links and web links
/// of the plan's items are read once before the link phase, and an entry found there answers `created` false
/// without an insert. Seen at the statement level: the first apply sends the inserts, the replay sends none and
/// reads the item links once, however many the plan lists.
#[tokio::test]
async fn a_replayed_plan_reads_its_links_once_and_sends_no_insert_for_one_that_exists() {
    let (_c, pool) = common::pg_counted().await;
    let (h, _sink) = mk_table_sink(pool.clone()).await;
    let dan = ctx("dan@example.com", false);
    let plan = || -> reeve_protocol::ApplyPlan {
        serde_json::from_value(serde_json::json!({
            "id": "replay",
            "epics": [{"name": "e", "title": "E"}],
            "stories": [
                {"name": "a", "title": "A", "epic": "e"},
                {"name": "b", "title": "B", "epic": "e"},
                {"name": "c", "title": "C", "epic": "e"}
            ],
            "links": [
                {"from": "b", "type": "follows", "to": "a"},
                {"from": "c", "type": "follows", "to": "b"},
                {"from": "c", "type": "relates_to", "to": "a"},
                {"from": "e", "type": "duplicates", "to": "c"}
            ],
            "web_links": [
                {"on": "a", "title": "Page", "url": "https://example.com/a"},
                {"on": "a", "title": "Notes", "url": "https://example.com/notes"},
                {"on": "e", "title": "Page", "url": "https://example.com/a"}
            ]
        }))
        .unwrap()
    };
    // How many times the statements whose text holds `text` were sent, in the block last measured.
    let calls = |text: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT coalesce(sum(calls), 0)::bigint FROM pg_stat_statements \
                 WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
                 AND query ILIKE '%' || $1 || '%'",
            )
            .bind(text)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };

    common::statements(&pool, || async {
        let first = h.apply_plan(&dan, plan()).await.unwrap();
        assert!(first.links.iter().all(|l| l.created));
        assert!(first.web_links.iter().all(|w| w.created));
    })
    .await;
    assert_eq!(
        (
            calls("INSERT INTO item_links").await,
            calls("INSERT INTO web_links").await
        ),
        (4, 3),
        "the control: the first apply sends one insert per link, and the view shows them"
    );

    common::statements(&pool, || async {
        let again = h.apply_plan(&dan, plan()).await.unwrap();
        assert_eq!(
            (again.links.len(), again.web_links.len()),
            (4, 3),
            "every entry is answered"
        );
        assert!(again.links.iter().all(|l| !l.created));
        assert!(again.web_links.iter().all(|w| !w.created));
    })
    .await;
    assert_eq!(
        (
            calls("INSERT INTO item_links").await,
            calls("INSERT INTO web_links").await
        ),
        (0, 0),
        "the replay sends no insert"
    );
    assert_eq!(
        calls("FROM item_links").await,
        1,
        "the item links of the plan's items are read once"
    );
    assert_eq!(
        calls("FROM web_links").await,
        2,
        "the web links are read once by the validation and once before the link phase"
    );
}

/// Waits until `edit` has answered or waits on a lock (the test's open transaction holds it).
async fn answered_or_waiting<T>(pool: &sqlx::PgPool, edit: &tokio::task::JoinHandle<T>) {
    for _ in 0..250 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND datname = current_database()",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        if edit.is_finished() || waiting >= 1 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("the edit neither answered nor waited on the lock");
}

/// An edit that names a version ahead of the stored one is a conflict carrying the stored version, like a stale one:
/// the row it read is not the row its version names.
#[tokio::test]
async fn an_edit_at_a_version_ahead_of_the_stored_one_is_a_conflict() {
    let (_db, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan", false);
    let i = h
        .create_initiative(&dan, "i", "I", "", "Now", None, None, None)
        .await
        .unwrap();
    let e = h.create_epic(&dan, "e", "E", None, "", None).await.unwrap();
    let s = h
        .create_story(
            &dan,
            "s",
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
        .unwrap();
    let entries = sink.0.lock().unwrap().len();
    let title = || Some("Ahead".to_string());
    assert!(matches!(
        h.edit_initiative(&dan, i.key(), 2, title(), None, None, None, None, None)
            .await
            .unwrap_err(),
        HandlerError::Conflict { current: 1 }
    ));
    assert!(matches!(
        h.edit_epic(&dan, e.key(), 2, title(), None, None, None)
            .await
            .unwrap_err(),
        HandlerError::Conflict { current: 1 }
    ));
    assert!(matches!(
        h.edit_story(
            &dan,
            s.key(),
            2,
            title(),
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
        HandlerError::Conflict { current: 1 }
    ));
    assert_eq!(
        h.ports.initiatives.get_initiative(i.key()).await.unwrap(),
        i
    );
    assert_eq!(h.ports.epics.get_epic(e.key()).await.unwrap(), e);
    assert_eq!(h.get_story(s.key()).await.unwrap(), s);
    assert_eq!(sink.0.lock().unwrap().len(), entries, "no entry");
}

/// An unmarked item is at version 1. An edit names version 2 and reads version 1; before it writes, another
/// transaction marks discovery complete, which makes version 2. The edit must not then write the unmarked row it
/// read over the mark: it is refused at its read. The edit is held between its read and its update on the lock of
/// the external reference it sets, which it takes after the read (`set_ref`); the test's transaction holds that
/// lock and writes the mark. Goes red if an edit handler stops comparing the version it was given with the row it
/// read: the conditional update alone lets the edit through once the mark has committed.
#[tokio::test]
async fn an_edit_ahead_of_its_read_never_undoes_a_concurrent_discovery_mark() {
    let (_db, pool) = common::pg().await;
    let (h, _) = mk(pool.clone());
    let dan = ctx("dan", false);
    let i = h
        .create_initiative(&dan, "i", "I", "", "Now", None, None, None)
        .await
        .unwrap();
    let e = h.create_epic(&dan, "e", "E", None, "", None).await.unwrap();
    for (table, key, reference) in [
        ("initiatives", i.key().clone(), "JPD-1"),
        ("epics", e.key().clone(), "JPD-2"),
    ] {
        let mut mark = pool.begin().await.unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('reeve_import:' || $1))")
            .bind(reference)
            .execute(&mut *mark)
            .await
            .unwrap();
        let edit = tokio::spawn({
            let (h, dan, key) = (h.clone(), dan.clone(), key.clone());
            async move {
                let (title, r) = (Some("Ahead".to_string()), Some(reference.to_string()));
                if table == "epics" {
                    h.edit_epic(&dan, &key, 2, title, None, None, r)
                        .await
                        .map(|_| ())
                } else {
                    h.edit_initiative(&dan, &key, 2, title, None, None, None, None, r)
                        .await
                        .map(|_| ())
                }
            }
        });
        answered_or_waiting(&pool, &edit).await;
        // The mark, as `mark_discovery` writes it; committing it lets the edit go on.
        sqlx::query(&format!(
            "UPDATE {table} SET discovery_at = now(), discovery_by = 'ann', version = version + 1 WHERE key = $1 AND version = 1"
        ))
        .bind(key.to_string())
        .execute(&mut *mark)
        .await
        .unwrap();
        mark.commit().await.unwrap();
        let answer = edit.await.unwrap();
        let (marked, title): (bool, String) = sqlx::query_as(&format!(
            "SELECT discovery_at IS NOT NULL, title FROM {table} WHERE key = $1"
        ))
        .bind(key.to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(marked, "{table}: the edit undid the discovery mark");
        assert_ne!(title, "Ahead", "{table}: the edit was written");
        assert!(
            matches!(answer, Err(HandlerError::Conflict { current: 1 })),
            "{table}: {answer:?}"
        );
    }
}

/// The story form of the test above. A story is at version 1, with no owner, under its epic. An edit names version
/// 2 and reads version 1; before it writes, another caller makes version 2: a move to in progress, which makes the
/// mover its owner, or an edit that moves it to another epic. The story edit rebuilds the whole row from its read
/// (owner and epic included), so it must be refused at its read rather than write the stale owner or epic back. It
/// is held between its read and its update on the external reference lock `set_ref` takes, as above. Goes red if the
/// story edit handler stops comparing the version it was given with the row it read: the conditional update alone
/// lets the edit through at version 2, and it clears the mover's ownership or puts the story back under its epic.
#[tokio::test]
async fn a_story_edit_ahead_of_its_read_never_undoes_a_concurrent_owner_or_epic() {
    let (_db, pool) = common::pg().await;
    let (h, _) = mk(pool.clone());
    let (dan, ann) = (ctx("dan", false), ctx("ann@example.com", false));
    let epic = ready_epic(&h, &dan).await;
    let other = h
        .create_epic(&dan, "other", "Other", None, "", None)
        .await
        .unwrap();
    for (case, reference) in [("owner", "JPD-3"), ("epic", "JPD-4")] {
        let s = h
            .create_story(
                &dan,
                case,
                "S",
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
        assert_eq!((s.version(), s.owner()), (1, None));
        let key = s.key().clone();
        let mut lock = pool.begin().await.unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('reeve_import:' || $1))")
            .bind(reference)
            .execute(&mut *lock)
            .await
            .unwrap();
        let edit = tokio::spawn({
            let (h, dan, key) = (h.clone(), dan.clone(), key.clone());
            async move {
                h.edit_story(
                    &dan,
                    &key,
                    2,
                    Some("Ahead".into()),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(reference.into()),
                )
                .await
            }
        });
        answered_or_waiting(&pool, &edit).await;
        // The concurrent write, through its own handler: it takes no reference lock, so it is not held.
        let theirs = if case == "owner" {
            h.move_story(&ann, &key, 1, DeliveryState::InProgress, None, None)
                .await
                .unwrap()
        } else {
            h.edit_story(
                &ann,
                &key,
                1,
                None,
                None,
                None,
                Some(Some(other.key().clone())),
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap()
        };
        assert_eq!(theirs.version(), 2, "{case}");
        lock.commit().await.unwrap();
        let answer = edit.await.unwrap();
        let now = h.get_story(&key).await.unwrap();
        if case == "owner" {
            assert_eq!(
                now.owner(),
                Some("ann@example.com"),
                "owner: the edit undid the mover's ownership"
            );
        } else {
            assert_eq!(
                now.epic(),
                Some(other.key()),
                "epic: the edit put the story back under its epic"
            );
        }
        assert_ne!(now.title(), "Ahead", "{case}: the edit was written");
        assert!(
            matches!(answer, Err(HandlerError::Conflict { current: 1 })),
            "{case}: {answer:?}"
        );
    }
}

/// The real initiative rows, with one write of another caller's let in: the first read after an update (the
/// handler's read after its commit, when it makes one) first retitles the row on another connection, as a second
/// writer landing between the edit's commit and that read would.
struct RetitledAfterUpdate {
    real: Arc<dyn InitiativeRepo>,
    pool: sqlx::PgPool,
    armed: Mutex<bool>,
}
#[async_trait]
impl InitiativeRepo for RetitledAfterUpdate {
    async fn insert_initiative(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        i: &Initiative,
        idem: &str,
        now: chrono::DateTime<Utc>,
    ) -> Result<bool, RepoError> {
        self.real.insert_initiative(tx, i, idem, now).await
    }
    async fn get_initiative(&self, key: &Key) -> Result<Initiative, RepoError> {
        let armed = std::mem::take(&mut *self.armed.lock().unwrap());
        if armed {
            sqlx::query(
                "UPDATE initiatives SET title = 'Theirs', version = version + 1 WHERE key = $1",
            )
            .bind(key.to_string())
            .execute(&self.pool)
            .await?;
        }
        self.real.get_initiative(key).await
    }
    async fn get_initiative_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        key: &Key,
    ) -> Result<Initiative, RepoError> {
        self.real.get_initiative_in(tx, key).await
    }
    async fn list_initiatives_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> Result<Vec<Initiative>, RepoError> {
        self.real.list_initiatives_in(tx).await
    }
    async fn update_initiative(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        i: &Initiative,
        expected_version: i64,
        now: chrono::DateTime<Utc>,
    ) -> Result<(), RepoError> {
        self.real
            .update_initiative(tx, i, expected_version, now)
            .await?;
        *self.armed.lock().unwrap() = true;
        Ok(())
    }
}

/// The handler that writes an initiative edit answers the row that write left, never a read made after its commit:
/// another caller's write can land between the two, and the MCP receipt built from the answer would then report that
/// caller's title and version as this edit's. A chat sending that version next would overwrite a change it never
/// saw. Goes red if the handler answers a read after its commit: it then answers "Theirs" at version 3.
#[tokio::test]
async fn an_initiative_edit_answers_the_row_it_wrote_not_a_later_one() {
    let (_db, pool) = common::pg().await;
    let (mut h, _) = mk(pool.clone());
    h.ports.initiatives = Arc::new(RetitledAfterUpdate {
        real: h.ports.initiatives.clone(),
        pool: pool.clone(),
        armed: Mutex::new(false),
    });
    let dan = ctx("dan", false);
    let i = h
        .create_initiative(&dan, "i", "I", "", "Now", None, None, None)
        .await
        .unwrap();
    let edited = h
        .edit_initiative(
            &dan,
            i.key(),
            1,
            Some("Mine".into()),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        (edited.title(), edited.version()),
        ("Mine", 2),
        "the edit answered another writer's row"
    );
    // The other writer did land after the edit: the row now is theirs.
    let now = h.ports.initiatives.get_initiative(i.key()).await.unwrap();
    assert_eq!((now.title(), now.version()), ("Theirs", 3));
}

/// The handler that writes an epic edit answers the move it wrote: the initiative left and the one joined, `None`
/// when the edit did not change the initiative. It is the move its `edited` entry names, and the epic it answers
/// is the row that write left.
#[tokio::test]
async fn an_epic_edit_answers_the_move_it_wrote() {
    let (_db, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan", false);
    let a = h
        .create_initiative(&dan, "a", "A", "", "Now", None, None, None)
        .await
        .unwrap();
    let b = h
        .create_initiative(&dan, "b", "B", "", "Now", None, None, None)
        .await
        .unwrap();
    let (a, b) = (a.key().clone(), b.key().clone());
    let e = h.create_epic(&dan, "e", "E", None, "", None).await.unwrap();
    let key = e.key().clone();
    let name =
        |k: &Option<Key>| serde_json::json!({"initiative": k.as_ref().map(ToString::to_string)});
    // (the title sent, the initiative sent, the move expected)
    type Step = (Option<&'static str>, Option<Option<Key>>, Option<EpicMove>);
    let steps: Vec<Step> = vec![
        (
            None,
            Some(Some(a.clone())),
            Some(EpicMove {
                from: None,
                to: Some(a.clone()),
            }),
        ),
        (Some("Retitled"), None, None),
        (Some("Same place"), Some(Some(a.clone())), None),
        (
            Some("Moved and retitled"),
            Some(Some(b.clone())),
            Some(EpicMove {
                from: Some(a.clone()),
                to: Some(b.clone()),
            }),
        ),
        (
            None,
            Some(None),
            Some(EpicMove {
                from: Some(b.clone()),
                to: None,
            }),
        ),
        (None, Some(None), None),
    ];
    let mut version = e.version();
    for (n, (title, initiative, expected)) in steps.into_iter().enumerate() {
        let (edited, moved) = h
            .edit_epic_reporting(
                &dan,
                &key,
                version,
                title.map(String::from),
                None,
                initiative,
                None,
                &[],
            )
            .await
            .unwrap();
        assert_eq!(moved, expected, "step {n}");
        let entry = sink.0.lock().unwrap().last().unwrap().clone();
        assert_eq!(entry.action, "edited", "step {n}");
        match &moved {
            Some(m) => {
                assert_eq!(entry.payload["from"], name(&m.from), "step {n}");
                assert_eq!(entry.payload["to"], name(&m.to), "step {n}");
                assert_eq!(edited.initiative(), m.to.as_ref(), "step {n}");
            }
            None => {
                assert!(entry.payload.get("from").is_none(), "step {n}");
                assert!(entry.payload.get("to").is_none(), "step {n}");
            }
        }
        version += 1;
        assert_eq!(edited.version(), version, "step {n}");
        assert_eq!(
            edited,
            h.ports.epics.get_epic(&key).await.unwrap(),
            "step {n}: the epic answered is the stored one"
        );
    }
}

#[tokio::test]
async fn brief_template_writes_log_who_changed_what_and_never_the_body() {
    use BriefTemplateLevel::Initiative;
    use serde_json::json;
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let body = "# Scenario\n\n| Step | Who |\n|---|---|\n";

    let t = h
        .create_brief_template(&dan, Initiative, "  Scenario ", body)
        .await
        .unwrap();
    assert_eq!(
        (t.name.as_str(), t.body.as_str(), t.is_default, t.version),
        ("Scenario", body, false, 1)
    );
    // The same content again, the name in another case: the same template, and nothing is written.
    assert_eq!(
        h.create_brief_template(&dan, Initiative, "scenario", body)
            .await
            .unwrap(),
        t
    );
    // The same name with another body is not an edit.
    let err = h
        .create_brief_template(&dan, Initiative, "SCENARIO", "other")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, HandlerError::Validation(m) if *m == format!(
            "the initiative brief template #{} is already named \"Scenario\": edit that one, or choose another name",
            t.id
        )),
        "{err:?}"
    );

    // An edit names the version it read.
    let stale = h
        .edit_brief_template(&dan, t.id, 7, Some("X".into()), None)
        .await
        .unwrap_err();
    assert!(
        matches!(stale, HandlerError::Conflict { current: 1 }),
        "{stale:?}"
    );
    // An edit that changes nothing writes nothing and keeps the version.
    let same = h
        .edit_brief_template(&dan, t.id, 1, Some(" Scenario ".into()), Some(body.into()))
        .await
        .unwrap();
    assert_eq!(same, t);
    let edited = h
        .edit_brief_template(
            &dan,
            t.id,
            1,
            Some("User scenario".into()),
            Some("SECRET BODY".into()),
        )
        .await
        .unwrap();
    assert_eq!(
        (edited.name.as_str(), edited.body.as_str(), edited.version),
        ("User scenario", "SECRET BODY", 2)
    );
    let body_only = h
        .edit_brief_template(&dan, t.id, 2, None, Some("SECRET BODY 2".into()))
        .await
        .unwrap();
    assert_eq!(
        (body_only.name.as_str(), body_only.version),
        ("User scenario", 3)
    );

    // The mark: set, set again (nothing), clear, clear again (nothing), set. It changes no version.
    let on = h
        .mark_brief_template_default(&dan, t.id, true)
        .await
        .unwrap();
    assert_eq!((on.is_default, on.version), (true, 3));
    assert_eq!(
        h.mark_brief_template_default(&dan, t.id, true)
            .await
            .unwrap(),
        on
    );
    let off = h
        .mark_brief_template_default(&dan, t.id, false)
        .await
        .unwrap();
    assert!(!off.is_default);
    assert_eq!(
        h.mark_brief_template_default(&dan, t.id, false)
            .await
            .unwrap(),
        off
    );
    h.mark_brief_template_default(&dan, t.id, true)
        .await
        .unwrap();

    h.remove_brief_template(&dan, t.id).await.unwrap();
    // A template that is gone is not found, by every path.
    assert!(matches!(
        h.remove_brief_template(&dan, t.id).await,
        Err(HandlerError::NotFound)
    ));
    assert!(matches!(
        h.brief_template(t.id).await,
        Err(HandlerError::NotFound)
    ));
    assert!(matches!(
        h.edit_brief_template(&dan, t.id, 3, None, Some("x".into()))
            .await,
        Err(HandlerError::NotFound)
    ));
    assert!(matches!(
        h.mark_brief_template_default(&dan, t.id, true).await,
        Err(HandlerError::NotFound)
    ));

    let log = sink.0.lock().unwrap();
    let id = t.id.to_string();
    let seen: Vec<(&str, &str, &str, serde_json::Value)> = log
        .iter()
        .map(|r| {
            (
                r.entity_type,
                r.entity_key.as_str(),
                r.action,
                r.payload.clone(),
            )
        })
        .collect();
    let entry = |action: &'static str, payload: serde_json::Value| {
        ("brief_template", id.as_str(), action, payload)
    };
    let named = json!({"level": "initiative", "name": "User scenario"});
    let with = |extra: serde_json::Value| {
        let mut p = named.clone();
        p.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        p
    };
    assert_eq!(
        seen,
        vec![
            entry(
                "brief_template_created",
                json!({"level": "initiative", "name": "Scenario"})
            ),
            entry(
                "brief_template_edited",
                with(json!({"fields": ["name", "body"], "from": "Scenario"}))
            ),
            entry("brief_template_edited", with(json!({"fields": ["body"]}))),
            entry(
                "brief_template_default_set",
                with(json!({"replaced": null}))
            ),
            entry("brief_template_default_cleared", named.clone()),
            entry(
                "brief_template_default_set",
                with(json!({"replaced": null}))
            ),
            entry("brief_template_removed", with(json!({"was_default": true}))),
        ]
    );
    assert!(log.iter().all(|r| r.actor == "dan@example.com"));
    let text = format!("{:?}", *log);
    assert!(
        !text.contains("SECRET BODY") && !text.contains("| Step |"),
        "no entry copies a body: {text}"
    );
}

#[tokio::test]
async fn brief_template_names_and_the_default_are_one_per_level() {
    use BriefTemplateLevel::{Epic, Initiative};
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let a = h
        .create_brief_template(&dan, Initiative, "Scenario", "A")
        .await
        .unwrap();
    let b = h
        .create_brief_template(&dan, Initiative, "Tech debt", "B")
        .await
        .unwrap();
    // The same name at the other level is another template.
    let e = h
        .create_brief_template(&dan, Epic, "Scenario", "E")
        .await
        .unwrap();
    assert_ne!(a.id, e.id);

    // A rename onto a name the level has is refused, in any case; a template may change the case of its own name.
    let err = h
        .edit_brief_template(&dan, b.id, 1, Some(" scenario".into()), None)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, HandlerError::Validation(m) if *m == format!(
            "the initiative brief template #{} is already named \"Scenario\": edit that one, or choose another name",
            a.id
        )),
        "{err:?}"
    );
    let recased = h
        .edit_brief_template(&dan, b.id, 1, Some("TECH DEBT".into()), None)
        .await
        .unwrap();
    assert_eq!((recased.name.as_str(), recased.version), ("TECH DEBT", 2));

    // Listed initiative first, then by name as names compare (lowercased), whatever the ids and whatever the
    // bytes: "Zulu" is created before "alpha" and sorts before it byte for byte ('Z' is 0x5A, 'a' is 0x61), so
    // the order by id and the order by the raw name both put "Zulu" ahead of "alpha", and only the order by the
    // name key gives the list below.
    let listed = |ts: &[BriefTemplateDto]| -> Vec<(BriefTemplateLevel, String, bool)> {
        ts.iter()
            .map(|t| (t.level, t.name.clone(), t.is_default))
            .collect()
    };
    let zulu = h
        .create_brief_template(&dan, Initiative, "Zulu", "Z")
        .await
        .unwrap();
    let alpha = h
        .create_brief_template(&dan, Initiative, "alpha", "A2")
        .await
        .unwrap();
    assert!(
        e.id < zulu.id && zulu.id < alpha.id,
        "the control: by id (as by raw name), Zulu comes before alpha"
    );
    assert_eq!(
        listed(&h.brief_templates(None).await.unwrap()),
        vec![
            (Initiative, "alpha".to_string(), false),
            (Initiative, "Scenario".to_string(), false),
            (Initiative, "TECH DEBT".to_string(), false),
            (Initiative, "Zulu".to_string(), false),
            (Epic, "Scenario".to_string(), false),
        ]
    );
    assert_eq!(
        h.brief_templates(Some(Epic)).await.unwrap(),
        vec![e.clone()]
    );
    assert_eq!(h.brief_template(a.id).await.unwrap(), a);

    // What a create reads: a named template at its level only, and the level's default.
    assert_eq!(h.brief_template_body(Initiative, None).await.unwrap(), None);
    assert_eq!(
        h.brief_template_body(Initiative, Some(" tech debt "))
            .await
            .unwrap()
            .as_deref(),
        Some("B")
    );
    assert_eq!(
        h.brief_template_body(Epic, Some("tech debt"))
            .await
            .unwrap(),
        None,
        "a name is looked up at its level only"
    );

    // The mark moves within a level, says what it replaced, and leaves the other level alone.
    h.mark_brief_template_default(&dan, a.id, true)
        .await
        .unwrap();
    h.mark_brief_template_default(&dan, e.id, true)
        .await
        .unwrap();
    h.mark_brief_template_default(&dan, b.id, true)
        .await
        .unwrap();
    assert_eq!(
        listed(&h.brief_templates(None).await.unwrap())
            .into_iter()
            .filter(|(_, _, d)| *d)
            .map(|(l, n, _)| (l, n))
            .collect::<Vec<_>>(),
        vec![
            (Initiative, "TECH DEBT".to_string()),
            (Epic, "Scenario".to_string())
        ]
    );
    assert_eq!(
        h.brief_template_body(Initiative, None)
            .await
            .unwrap()
            .as_deref(),
        Some("B")
    );
    assert_eq!(
        h.brief_template_body(Epic, None).await.unwrap().as_deref(),
        Some("E")
    );
    let replaced: Vec<serde_json::Value> = sink
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.action == "brief_template_default_set")
        .map(|r| r.payload["replaced"].clone())
        .collect();
    assert_eq!(
        replaced,
        vec![
            serde_json::Value::Null,
            serde_json::Value::Null,
            serde_json::json!(a.id)
        ]
    );

    // Removing the default leaves its level without one, and the other level as it was.
    h.remove_brief_template(&dan, b.id).await.unwrap();
    assert_eq!(h.brief_template_body(Initiative, None).await.unwrap(), None);
    assert_eq!(
        h.brief_template_body(Epic, None).await.unwrap().as_deref(),
        Some("E")
    );

    // The domain's rules reach the caller as validation errors.
    for (name, body) in [(" ", "x"), ("N", " \n")] {
        assert!(matches!(
            h.create_brief_template(&dan, Epic, name, body).await,
            Err(HandlerError::Validation(_))
        ));
    }
}

#[tokio::test]
async fn a_brief_template_and_its_entry_commit_together() {
    let (_c, pool) = common::pg().await;
    // The probe answers the template's name by the entry's key: null while no committed or visible row has it.
    let (h, probe) = mk_probe(
        pool,
        "SELECT (SELECT name FROM brief_templates WHERE id::text = $1)",
    );
    let dan = ctx("dan@example.com", false);
    let t = h
        .create_brief_template(&dan, BriefTemplateLevel::Epic, "Tech debt", "## Debt\n")
        .await
        .unwrap();
    let seen = probe.entries("brief_template_created");
    assert_eq!(seen.len(), 1);
    let (_, inside, outside) = &seen[0];
    assert_eq!(
        inside.as_deref(),
        Some("Tech debt"),
        "the row was written before its entry, in the entry's transaction"
    );
    assert_eq!(
        outside, &None,
        "and nothing was committed before the entry was recorded"
    );
    assert_eq!(h.brief_template(t.id).await.unwrap().name, "Tech debt");
}

/// How many ungranted advisory locks in this database match `waiting`, a condition on `pg_locks`'s columns.
async fn advisory_waiters(conn: &mut sqlx::PgConnection, waiting: &str) -> i64 {
    sqlx::query_scalar(&format!(
        "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND NOT granted \
         AND database = (SELECT oid FROM pg_database WHERE datname = current_database()) AND ({waiting})"
    ))
    .fetch_one(conn)
    .await
    .unwrap()
}

/// Holds an advisory lock in a transaction of its own, runs `a` and `b` while it is held, and releases it once both
/// are seen waiting on an advisory lock in `pg_locks` that matches `waiting` (a condition on `pg_locks`'s columns,
/// naming the lock under test), or after five seconds when they never are. `lock` is the
/// exact statement the adapter under test sends, written out by the caller with its one argument in `arg` (never
/// the port: a no-op adapter must not also make the holder a no-op). Answers both results and whether both waited.
/// The holder is released whether or not they waited, so a run without the lock ends in a failed assertion, not a
/// hang; and the whole, the release included, must end within 30 seconds, so a call that stalls after the release
/// fails here by name rather than at nextest's termination. pg_locks is server-wide and every test has a database
/// of its own on one server, so the count is scoped to this database; `mk`'s memory sink takes no lock of its own,
/// so the only advisory locks in this database are the ones under test.
async fn while_both_wait_on<A, B>(
    pool: &sqlx::PgPool,
    lock: &str,
    arg: Option<&str>,
    waiting: &str,
    a: impl std::future::Future<Output = A>,
    b: impl std::future::Future<Output = B>,
) -> (A, B, bool) {
    use std::time::Duration;
    let mut holder = pool.begin().await.unwrap();
    let held = sqlx::query(lock);
    match arg {
        Some(arg) => held.bind(arg),
        None => held,
    }
    .execute(&mut *holder)
    .await
    .unwrap();
    let release = async {
        let mut waited = false;
        {
            let mut conn = pool.acquire().await.unwrap();
            for _ in 0..250 {
                if advisory_waiters(&mut conn, waiting).await >= 2 {
                    waited = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        holder.commit().await.unwrap();
        waited
    };
    tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(a, b, release)
    })
    .await
    .expect("the two calls did not end within 30 seconds of the lock being taken")
}

/// `while_both_wait_on` the brief template lock, a single bigint key (`objsubid` 1 in `pg_locks`).
async fn while_both_wait<A, B>(
    pool: &sqlx::PgPool,
    a: impl std::future::Future<Output = A>,
    b: impl std::future::Future<Output = B>,
) -> (A, B, bool) {
    while_both_wait_on(
        pool,
        "SELECT pg_advisory_xact_lock(hashtext('reeve_brief_templates'))",
        None,
        "objsubid = 1",
        a,
        b,
    )
    .await
}

/// Template writes that overlap are serialised by the advisory lock: each reads what the other committed. Each pair
/// below is made to overlap, not hoped to: a holder takes the lock first, both writers are seen queued on it, and only
/// then is it released, so they run one after the other in whichever order Postgres grants the lock. Every assertion
/// after `waited` is one that only the lock makes true: without it the same calls end in a unique-constraint error
/// (the two creates, the two marks) or in a lost update with no conflict (the two edits).
#[tokio::test]
async fn overlapping_brief_template_writes_are_serialised() {
    use BriefTemplateLevel::Initiative;
    let (_c, pool) = common::pg().await;
    let (h, sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);
    let erin = ctx("erin@example.com", false);
    // The entries of one action so far, in the order they were recorded: (the template's id, the payload).
    let entries = |action: &str| -> Vec<(String, serde_json::Value)> {
        sink.0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.action == action)
            .map(|r| (r.entity_key.clone(), r.payload.clone()))
            .collect()
    };
    let of = |all: &[(String, serde_json::Value)], id: i64| -> usize {
        all.iter().filter(|(key, _)| *key == id.to_string()).count()
    };

    // 1. The same create twice at once: one row, one entry, and both callers are answered the same template.
    let (a, b, waited) = while_both_wait(
        &pool,
        h.create_brief_template(&dan, Initiative, "Same", "body"),
        h.create_brief_template(&erin, Initiative, "Same", "body"),
    )
    .await;
    assert!(
        waited,
        "the two creates never both waited on the template lock"
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a, b, "both creates answer the one template");
    assert_eq!(of(&entries("brief_template_created"), a.id), 1);

    // 2. One name with two bodies at once: one is stored and the other is told the name is taken. Never a database
    //    error.
    let (one, two, waited) = while_both_wait(
        &pool,
        h.create_brief_template(&dan, Initiative, "Clash", "one"),
        h.create_brief_template(&erin, Initiative, "Clash", "two"),
    )
    .await;
    assert!(
        waited,
        "the two clashing creates never both waited on the template lock"
    );
    let (won, lost) = match (one, two) {
        (Ok(won), Err(lost)) | (Err(lost), Ok(won)) => (won, lost),
        other => panic!("wanted one template and one refusal, got {other:?}"),
    };
    assert!(
        matches!(&lost, HandlerError::Validation(m) if m.contains("is already named")),
        "{lost:?}"
    );
    let named: Vec<BriefTemplateDto> = h
        .brief_templates(Some(Initiative))
        .await
        .unwrap()
        .into_iter()
        .filter(|t| t.name == "Same" || t.name == "Clash")
        .collect();
    assert_eq!(named.len(), 2, "one row per name: {named:?}");

    // 3. Two edits that both read version 1: one is saved and the other conflicts with the version that is current
    //    now. Without the lock both would be saved, the second over the first.
    let (first, second, waited) = while_both_wait(
        &pool,
        h.edit_brief_template(&dan, won.id, 1, None, Some("dan's".into())),
        h.edit_brief_template(&erin, won.id, 1, None, Some("erin's".into())),
    )
    .await;
    assert!(
        waited,
        "the two edits never both waited on the template lock"
    );
    let (saved, stale) = match (first, second) {
        (Ok(saved), Err(stale)) | (Err(stale), Ok(saved)) => (saved, stale),
        other => panic!("wanted one save and one conflict, got {other:?}"),
    };
    assert_eq!(saved.version, 2);
    assert!(
        matches!(stale, HandlerError::Conflict { current: 2 }),
        "{stale:?}"
    );
    assert_eq!(
        h.brief_template(won.id).await.unwrap(),
        saved,
        "the stored row is the saved edit, not the refused one"
    );
    assert_eq!(of(&entries("brief_template_edited"), won.id), 1);

    // 4. Two templates of one level marked default at once: both calls succeed, one mark stands, and the entry
    //    recorded last is the one that stands and names the other as what it replaced.
    let (x, y, waited) = while_both_wait(
        &pool,
        h.mark_brief_template_default(&dan, a.id, true),
        h.mark_brief_template_default(&erin, won.id, true),
    )
    .await;
    assert!(
        waited,
        "the two marks never both waited on the template lock"
    );
    x.unwrap();
    y.unwrap();
    let marked: Vec<i64> = h
        .brief_templates(Some(Initiative))
        .await
        .unwrap()
        .into_iter()
        .filter(|t| t.is_default)
        .map(|t| t.id)
        .collect();
    assert_eq!(marked.len(), 1, "one default per level");
    let sets = entries("brief_template_default_set");
    assert_eq!(sets.len(), 2, "each mark is logged once");
    let (earlier, later) = (&sets[0], &sets[1]);
    assert_eq!(
        later.0,
        marked[0].to_string(),
        "the last entry is the mark that stands"
    );
    assert_eq!(
        later.1["replaced"],
        serde_json::json!(earlier.0.parse::<i64>().unwrap()),
        "and it names the mark it replaced"
    );
}

/// `made_under` refuses exactly what a create handler refuses before its own idempotency lookup, with the handler's
/// message, and otherwise answers the item the key made. Catches a lookup that skips the handlers' pre-replay checks
/// (more than `MAX_CLAIM` distinct attachment ids), which would let the server's create actions answer a retry the
/// handler would refuse.
#[tokio::test]
async fn made_under_refuses_what_a_create_replay_refuses() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let too_many: Vec<uuid::Uuid> = (0..=MAX_CLAIM).map(|_| uuid::Uuid::new_v4()).collect();
    let i = h
        .create_initiative(&dan, "mu-i", "I", "", "Now", None, None, None)
        .await
        .unwrap();
    let e = h
        .create_epic(&dan, "mu-e", "E", None, "", None)
        .await
        .unwrap();
    let by_handler = [
        h.create_initiative_reporting(&dan, "mu-i", "I", "", "Now", None, None, None, &too_many)
            .await
            .map(|_| ()),
        h.create_epic_reporting(&dan, "mu-e", "E", None, "", None, &too_many)
            .await
            .map(|_| ()),
    ];
    for ((item, idem, key), handler) in [
        (ItemType::Initiative, "mu-i", i.key()),
        (ItemType::Epic, "mu-e", e.key()),
    ]
    .into_iter()
    .zip(by_handler)
    {
        let Err(HandlerError::Validation(want)) = handler else {
            panic!("{item:?}: the control: the handler's replay is refused: {handler:?}")
        };
        match h.made_under(item, idem, &too_many).await {
            Err(HandlerError::Validation(m)) => assert_eq!(m, want, "{item:?}"),
            other => panic!("{item:?}: {other:?}"),
        }
        assert_eq!(
            h.made_under(item, idem, &[]).await.unwrap().as_ref(),
            Some(key),
            "{item:?}"
        );
        assert_eq!(h.made_under(item, "mu-none", &[]).await.unwrap(), None);
    }
}

/// An edit's answer is built from the story as the edit wrote it. Here another writer has since retitled the
/// story, moved it under another epic, given it an external reference, started it and marked it urgent; `written`
/// is the story as the first edit left it. Every field the `Story` carries is `written`'s; what it does not carry
/// (the urgent mark, on the story's row but not in the aggregate, and the blocker and the web link, in other
/// tables) is read from the store. Catches: an answer that re-reads the row (it would show "Theirs", the later
/// version, in_progress and the other epic), and an answer that drops what only a read adds.
#[tokio::test]
async fn a_story_dto_as_written_keeps_the_written_story_and_reads_the_rest() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let now = Utc::now();
    let mine = h
        .create_epic(&dan, "w-e1", "Mine", None, "", None)
        .await
        .unwrap();
    let theirs = h
        .create_epic(&dan, "w-e2", "Theirs", None, "", None)
        .await
        .unwrap();
    let blocker = h
        .create_story(
            &dan,
            "w-b",
            "Blocker",
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
    let s = h
        .create_story(
            &dan,
            "w-s",
            "First",
            Kind::Story,
            None,
            "",
            vec!["c".into()],
            vec![],
            None,
            None,
        )
        .await
        .unwrap();
    // This call's edit: title and epic, version 1 -> 2.
    let written = h
        .edit_story(
            &dan,
            s.key(),
            1,
            Some("Mine".into()),
            None,
            None,
            Some(Some(mine.key().clone())),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!((written.title(), written.version()), ("Mine", 2));
    // Another writer reranks a neighbour before it, which renumbers the column and gives the story another stored
    // rank (a rerank bumps only the moved story's version, so the story stays at version 2).
    // (Both start at rank 0 and tie by key: the blocker goes last, then back before the story.)
    for before in [None, Some(s.key())] {
        let v = h.get_story(blocker.key()).await.unwrap().version();
        h.rank_story(&dan, blocker.key(), v, before).await.unwrap();
    }
    assert_ne!(
        h.get_story(s.key()).await.unwrap().rank(),
        written.rank(),
        "the control: the stored rank is not the written one"
    );
    // Another writer lands before the answer is built.
    h.edit_story(
        &dan,
        s.key(),
        2,
        Some("Theirs".into()),
        None,
        None,
        Some(Some(theirs.key().clone())),
        None,
        None,
        None,
        None,
        Some("OLD-7".into()),
    )
    .await
    .unwrap();
    h.move_story(
        &dan,
        s.key(),
        3,
        DeliveryState::InProgress,
        None,
        Some("demo".into()),
    )
    .await
    .unwrap();
    h.add_block(&dan, s.key(), blocker.key()).await.unwrap();
    assert!(
        h.add_web_link(&dan, s.key(), "Page", "https://example.com/s")
            .await
            .unwrap()
    );
    // A mark raises no version (`set_urgent_mark` names only the mark's columns).
    h.mark_urgent(&dan, ItemType::Story, s.key(), None)
        .await
        .unwrap();

    let stored = h.story_dto(s.key(), now).await.unwrap();
    assert_eq!(
        (stored.title.as_str(), stored.version, stored.delivery),
        ("Theirs", 4, DeliveryState::InProgress),
        "the control: the store holds the other writer's row"
    );
    assert_ne!(
        stored.rank,
        written.rank(),
        "the control: the stored row's rank is not the written one"
    );

    let dto = h.story_dto_as_written(&written, now).await.unwrap();
    assert_eq!(
        (
            dto.title.as_str(),
            dto.version,
            dto.delivery,
            dto.resolution,
            dto.epic.as_ref(),
            dto.external_ref.as_deref(),
            dto.rank
        ),
        (
            "Mine",
            2,
            DeliveryState::Todo,
            None,
            Some(mine.key()),
            None,
            written.rank()
        ),
        "what the story carries is the written one"
    );
    assert!(
        dto.urgent_mark.is_some(),
        "not in the aggregate: read from the store"
    );
    assert_eq!(
        dto.blocked_by,
        vec![blocker.key().clone()],
        "another table: read from the store"
    );
    assert_eq!(
        dto.web_links.as_ref().map(|w| w.len()),
        Some(1),
        "another table: read from the store"
    );
    assert_eq!(
        dto.links,
        Some(vec![]),
        "a read of one item carries its item links"
    );
}

/// The same for an epic: what the `Epic` carries (title, initiative, version, external reference) is the written
/// one; its web links are read from the store. Catches an answer that re-reads the epic's row.
#[tokio::test]
async fn an_epic_dto_as_written_keeps_the_written_epic_and_reads_the_rest() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let now = Utc::now();
    let ini = |idem: &'static str| {
        let (h, dan) = (h.clone(), dan.clone());
        async move {
            h.create_initiative(&dan, idem, "I", "", "Now", None, None, None)
                .await
                .unwrap()
        }
    };
    let (mine, theirs) = (ini("w-i1").await, ini("w-i2").await);
    let e = h
        .create_epic(&dan, "w-e", "First", None, "", None)
        .await
        .unwrap();
    let written = h
        .edit_epic(
            &dan,
            e.key(),
            1,
            Some("Mine".into()),
            None,
            Some(Some(mine.key().clone())),
            None,
        )
        .await
        .unwrap();
    h.edit_epic(
        &dan,
        e.key(),
        2,
        Some("Theirs".into()),
        Some("their brief".into()),
        Some(Some(theirs.key().clone())),
        Some("OLD-8".into()),
    )
    .await
    .unwrap();
    assert!(
        h.add_web_link(&dan, e.key(), "Page", "https://example.com/e")
            .await
            .unwrap()
    );
    assert_eq!(
        h.epic_dto(e.key(), now).await.unwrap().title,
        "Theirs",
        "the control"
    );

    let dto = h.epic_dto_as_written(&written, now).await.unwrap();
    assert_eq!(
        (
            dto.title.as_str(),
            dto.version,
            dto.initiative.as_ref(),
            dto.brief.as_str(),
            dto.external_ref.as_deref()
        ),
        ("Mine", 2, Some(mine.key()), "", None),
        "what the epic carries is the written one"
    );
    assert_eq!(
        dto.web_links.as_ref().map(|w| w.len()),
        Some(1),
        "another table: read from the store"
    );
    assert_eq!(dto.links, Some(vec![]));
}

/// The same for an initiative: what the `Initiative` carries is the written one; its progress and its web links are
/// read from the store. Catches an answer that re-reads the initiative's row, and one that loses the progress.
#[tokio::test]
async fn an_initiative_dto_as_written_keeps_the_written_initiative_and_reads_the_rest() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool);
    let dan = ctx("dan@example.com", false);
    let now = Utc::now();
    let i = h
        .create_initiative(&dan, "w-i", "First", "", "Now", None, None, None)
        .await
        .unwrap();
    let written = h
        .edit_initiative(
            &dan,
            i.key(),
            1,
            Some("Mine".into()),
            None,
            None,
            Some(Some("2027-03".into())),
            None,
            None,
        )
        .await
        .unwrap();
    h.edit_initiative(
        &dan,
        i.key(),
        2,
        Some("Theirs".into()),
        None,
        Some("Later".into()),
        Some(None),
        None,
        Some("OLD-9".into()),
    )
    .await
    .unwrap();
    // One story under it, so its progress is 0 of 1 and not "nothing counts".
    let e = h
        .create_epic(&dan, "w-ie", "E", Some(i.key().clone()), "", None)
        .await
        .unwrap();
    h.create_story(
        &dan,
        "w-is",
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
    assert!(
        h.add_web_link(&dan, i.key(), "Page", "https://example.com/i")
            .await
            .unwrap()
    );
    assert_eq!(
        h.initiative_dto(i.key(), now).await.unwrap().title,
        "Theirs",
        "the control"
    );

    let dto = h.initiative_dto_as_written(&written, now).await.unwrap();
    assert_eq!(
        (
            dto.title.as_str(),
            dto.version,
            dto.horizon.as_str(),
            dto.target_month.as_deref(),
            dto.external_ref.as_deref()
        ),
        ("Mine", 2, "Now", Some("2027-03"), None),
        "what the initiative carries is the written one"
    );
    assert_eq!(dto.progress, Some(0.0), "other tables: read from the store");
    assert_eq!(
        dto.web_links.as_ref().map(|w| w.len()),
        Some(1),
        "another table: read from the store"
    );
    assert_eq!(dto.links, Some(vec![]));
}

/// The lock plan apply's creates take on a plan item's idempotency key: the adapter's statement with its key space
/// (`IDEMPOTENCY_LOCK_SPACE` in `repo/keys.rs`, 1, private to the crate) written out.
const IDEMPOTENCY_LOCK: &str = "SELECT pg_advisory_xact_lock(1, hashtext($1))";

/// That lock as `pg_locks` shows it: a lock taken with two integer keys has the first key in `classid`, the second
/// in `objid` and `objsubid` 2 (a single bigint key has `objsubid` 1). The 1 in `classid` is
/// `IDEMPOTENCY_LOCK_SPACE`, as in `IDEMPOTENCY_LOCK`.
const IDEMPOTENCY_LOCK_WAIT: &str = "classid = 1 AND objsubid = 2";

/// Two applies of one plan id that give one name different levels, made to overlap, not hoped to, at every pair of
/// levels (initiative and epic, initiative and story, epic and story) and in both orders, each case under a plan id
/// of its own: a holder takes the key's lock first; the first apply passes the plan's check (nothing exists yet, so
/// the level check finds nothing) and is seen queued on that very lock inside its create; only then does the second
/// start, pass the check and queue behind it; and once both are seen waiting the lock is released. PostgreSQL grants
/// the lock in queue order, so the first apply creates and the second, the one whose guard is under test, is
/// refused and told which item has the name; sent again with the name at the winner's level it answers that item.
/// Exactly one item exists for the name afterwards across the three tables. Both orders matter: each level's look
/// must find each of the other two, and a look at a wrong level is only reached by the apply that comes second.
/// Catches: the fault GRA-888 names (both create, since an idempotency key is unique per table only), any of the
/// three plan creates left unguarded (an ordinary create in `apply_plan`), a look that skips a level, a lock in
/// another key space, under another key or none (`waited` is false), and a refusal in the words of the check, which
/// would say nothing was written.
#[tokio::test]
async fn two_applies_of_one_plan_cannot_give_a_name_two_levels() {
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);
    let erin = ctx("erin@example.com", false);
    // A plan of one item named `name` in `list`; an initiative needs its horizon.
    let plan = |id: &str, list: &str, name: &str| -> reeve_protocol::ApplyPlan {
        let mut item = serde_json::json!({"name": name, "title": "T"});
        if list == "initiatives" {
            item["horizon"] = serde_json::json!("Now");
        }
        serde_json::from_value(serde_json::json!({"id": id, list: [item]})).unwrap()
    };
    let level = |list: &str| match list {
        "initiatives" => "an initiative",
        "epics" => "an epic",
        "stories" => "a story",
        other => panic!("{other}"),
    };
    // `plan_key`: "plan:<length of the id>:<id>:<name>".
    for (id, name, key, first, second) in [
        ("race-ie", "ie", "plan:7:race-ie:ie", "initiatives", "epics"),
        ("race-ei", "ei", "plan:7:race-ei:ei", "epics", "initiatives"),
        (
            "race-is",
            "is",
            "plan:7:race-is:is",
            "initiatives",
            "stories",
        ),
        (
            "race-si",
            "si",
            "plan:7:race-si:si",
            "stories",
            "initiatives",
        ),
        ("race-es", "es", "plan:7:race-es:es", "epics", "stories"),
        ("race-se", "se", "plan:7:race-se:se", "stories", "epics"),
    ] {
        let case = format!("{first}, then {second}");
        // The second apply starts once the first is seen queued on the lock, so it queues behind it.
        let after_the_first = async {
            {
                let mut conn = pool.acquire().await.unwrap();
                let mut seen = false;
                for _ in 0..250 {
                    if advisory_waiters(&mut conn, IDEMPOTENCY_LOCK_WAIT).await >= 1 {
                        seen = true;
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                assert!(
                    seen,
                    "{case}: the first apply never queued on the idempotency key's lock"
                );
            }
            h.apply_plan(&erin, plan(id, second, name)).await
        };
        let (won, lost, waited) = while_both_wait_on(
            &pool,
            IDEMPOTENCY_LOCK,
            Some(key),
            IDEMPOTENCY_LOCK_WAIT,
            h.apply_plan(&dan, plan(id, first, name)),
            after_the_first,
        )
        .await;
        assert!(
            waited,
            "{case}: the two applies never both waited on the idempotency key's lock"
        );
        let (initiatives, epics, stories): (i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM initiatives WHERE idempotency_key = $1), \
             (SELECT count(*) FROM epics WHERE idempotency_key = $1), \
             (SELECT count(*) FROM stories WHERE idempotency_key = $1)",
        )
        .bind(key)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            initiatives + epics + stories,
            1,
            "{case}: {initiatives} initiatives, {epics} epics and {stories} stories for one name"
        );
        let won = won.unwrap_or_else(|e| panic!("{case}: the first apply was refused: {e:?}"));
        assert_eq!((won.items.len(), won.items[0].created), (1, true), "{case}");
        let made = &won.items[0].key;
        let Err(HandlerError::Validation(m)) = &lost else {
            panic!("{case}: wanted the second apply refused, got {lost:?}")
        };
        let a_level = level(first);
        assert_eq!(
            m,
            &format!(
                "\"{name}\" is {a_level} ({made}), created by another apply of this plan while this one ran; a \
                 name keeps its level. This apply stopped at \"{name}\": items written before it (initiatives, \
                 then epics, then stories) may exist; this apply wrote nothing after it, and no decisions, \
                 blockers or links. List \"{name}\" as {a_level}, or give it a new name, and apply the same id \
                 again; what exists is reused."
            ),
            "{case}"
        );
        // The loser applied again as the message says: the same name at the winner's level answers the one item.
        let again = h.apply_plan(&erin, plan(id, first, name)).await.unwrap();
        assert_eq!(
            (&again.items[0].key, again.items[0].created),
            (made, false),
            "{case}"
        );
    }
}

/// What does not take the lock. While a holder has the lock of a plan item's key, a second apply of the plan that
/// made the item answers it (the same-level replay is answered before the lock, and writes nothing), and a single
/// create under that very key at another level is not held up (a single create never takes the lock). Both are
/// given five seconds and must not need the holder to let go. Catches: a replay that queues behind another apply's
/// create, and the lock leaking into creates that are not a plan's. The single create's outcome, a second item
/// for the plan's name at another level, is not wanted: it pins a known limit (the property covers applies of one
/// plan id, not a single create that sends a plan's key by hand). GRA-984 makes single creates refuse a key with
/// the `plan:` prefix; that story replaces this half of the test with the refusal.
#[tokio::test]
async fn a_replayed_plan_and_a_single_create_never_wait_on_the_idempotency_lock() {
    use std::time::Duration;
    let (_c, pool) = common::pg().await;
    let (h, _sink) = mk(pool.clone());
    let dan = ctx("dan@example.com", false);
    let plan = || -> reeve_protocol::ApplyPlan {
        serde_json::from_value(
            serde_json::json!({"id": "once", "stories": [{"name": "s", "title": "S"}]}),
        )
        .unwrap()
    };
    let first = h.apply_plan(&dan, plan()).await.unwrap();
    let key = "plan:4:once:s";
    let mut holder = pool.begin().await.unwrap();
    sqlx::query(IDEMPOTENCY_LOCK)
        .bind(key)
        .execute(&mut *holder)
        .await
        .unwrap();
    let again = tokio::time::timeout(Duration::from_secs(5), h.apply_plan(&dan, plan()))
        .await
        .expect("a replay waited on the idempotency key's lock")
        .unwrap();
    assert_eq!(
        (&again.items[0].key, again.items[0].created),
        (&first.items[0].key, false)
    );
    tokio::time::timeout(
        Duration::from_secs(5),
        h.create_epic(&dan, key, "An epic under a plan's key", None, "", None),
    )
    .await
    .expect("a single create waited on the idempotency key's lock")
    .unwrap();
    // The known limit (GRA-984), read from the tables: the plan's name now has a story and an epic.
    let (stories, epics): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM stories WHERE idempotency_key = $1), \
         (SELECT count(*) FROM epics WHERE idempotency_key = $1)",
    )
    .bind(key)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (stories, epics),
        (1, 1),
        "known limit, not wanted (GRA-984): a single create under a plan's key at another level is not stopped"
    );
    holder.rollback().await.unwrap();
}
