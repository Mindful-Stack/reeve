mod common;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use reeve_planning::{
    ActivityRecord, ActivitySink, Closed, Ctx, Epic, Handlers, Initiative, ItemType, RepoError,
    Settings, Story, StoryFilter, compute_readiness, make_planning_repo,
};
use reeve_protocol::{
    CheckResult, ClosureKind, DeliveryState, Key, Kind, Reason, Resolution, Reversibility,
    TokenKind, Via,
};
use std::sync::Arc;

#[tokio::test]
async fn keys_items_versions_and_readiness_inputs() {
    let (_c, pool) = common::pg().await;
    let repo = make_planning_repo(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 10, 26, 9, 0, 0).unwrap();
    let buckets = vec!["Now".to_string(), "Later".to_string()];

    let mut tx = pool.begin().await.unwrap();
    let k1 = repo.next_key(&mut tx, "ABC").await.unwrap();
    let k2 = repo.next_key(&mut tx, "ABC").await.unwrap();
    assert_eq!(
        (k1.to_string(), k2.to_string()),
        ("ABC-1".to_string(), "ABC-2".to_string())
    );
    let ini = Initiative::new(k1.clone(), "Orion", "", "Now", &buckets, None, None).unwrap();
    assert!(
        repo.insert_initiative(&mut tx, &ini, "idem-ini", now)
            .await
            .unwrap()
    );
    let epic = Epic::new(k2.clone(), "Orion v2", Some(k1.clone()), "").unwrap();
    assert!(
        repo.insert_epic(&mut tx, &epic, "idem-epic", now)
            .await
            .unwrap()
    );
    let k3 = repo.next_key(&mut tx, "ABC").await.unwrap();
    let story = Story::new(
        k3.clone(),
        "Booking flow",
        Kind::Story,
        Some(k2.clone()),
        "",
        vec!["works".into()],
        None,
        vec![],
    )
    .unwrap();
    assert!(
        repo.insert_story(&mut tx, &story, "idem-story", now)
            .await
            .unwrap()
    );
    let dup = Story::new(
        repo.next_key(&mut tx, "ABC").await.unwrap(),
        "Dup",
        Kind::Story,
        None,
        "",
        vec![],
        None,
        vec![],
    )
    .unwrap();
    assert!(
        !repo
            .insert_story(&mut tx, &dup, "idem-story", now)
            .await
            .unwrap(),
        "a repeated idempotency key inserts nothing"
    );
    let (d, inserted) = repo
        .insert_decision(
            &mut tx,
            ItemType::Epic,
            &k2,
            "Pick queue",
            "dan@example.com",
            None,
            None,
            "idem-d",
            now,
        )
        .await
        .unwrap();
    assert!(inserted);
    let (again, inserted_again) = repo
        .insert_decision(
            &mut tx,
            ItemType::Epic,
            &k2,
            "Pick queue",
            "dan@example.com",
            None,
            None,
            "idem-d",
            now,
        )
        .await
        .unwrap();
    assert!(!inserted_again);
    assert_eq!(
        again.id, d.id,
        "a repeated idempotency key returns the existing decision"
    );
    tx.commit().await.unwrap();

    // Idempotency lookups find the same keys.
    assert_eq!(
        repo.find_by_idempotency(ItemType::Story, "idem-story")
            .await
            .unwrap(),
        Some(k3.clone())
    );
    assert_eq!(
        repo.find_by_idempotency(ItemType::Story, "nope")
            .await
            .unwrap(),
        None
    );

    // Readiness input carries ancestors epic-first, their decisions, and criteria count.
    let mut tx = pool.begin().await.unwrap();
    let input = repo
        .readiness_inputs(&mut tx, std::slice::from_ref(&k3))
        .await
        .unwrap()
        .remove(&k3)
        .unwrap();
    assert_eq!(
        input
            .ancestors
            .iter()
            .map(|a| a.key.to_string())
            .collect::<Vec<_>>(),
        ["ABC-2", "ABC-1"]
    );
    assert!(!input.ancestors[0].discovery_complete);
    assert_eq!(input.ancestors[0].decisions[0].id, d.id);
    assert_eq!(input.criteria_count, 1);
    tx.rollback().await.unwrap();

    // Version check: a stale update is refused with the current version.
    let mut tx = pool.begin().await.unwrap();
    let fresh = repo.get_story(&k3).await.unwrap();
    repo.set_delivery(
        &mut tx,
        &k3,
        fresh.version(),
        DeliveryState::InProgress,
        None,
        Some("dan@example.com"),
        now,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    let err = repo
        .set_delivery(
            &mut tx,
            &k3,
            fresh.version(),
            DeliveryState::InReview,
            None,
            None,
            now,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        reeve_planning::RepoError::Conflict { current: 2 }
    ));
    tx.rollback().await.unwrap();
    let after = repo.get_story(&k3).await.unwrap();
    assert_eq!(
        after.owner(),
        Some("dan@example.com"),
        "owner_if_none fills an empty owner"
    );
    assert_eq!(after.version(), 2);
    assert_eq!(after.delivery(), DeliveryState::InProgress);

    // Done with a resolution: owner_if_none does not overwrite an existing owner; counts see the completion.
    let mut tx = pool.begin().await.unwrap();
    repo.set_delivery(
        &mut tx,
        &k3,
        2,
        DeliveryState::Done,
        Some(Resolution::Completed),
        Some("other@example.com"),
        now,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let done = repo.get_story(&k3).await.unwrap();
    assert_eq!(done.owner(), Some("dan@example.com"));
    assert_eq!(done.delivery(), DeliveryState::Done);
    assert_eq!(done.resolution(), Some(Resolution::Completed));
    let counts = repo.story_counts_under_initiative(&k1).await.unwrap();
    assert_eq!((counts.completed, counts.dropped, counts.total), (1, 0, 1));

    // Reopening to todo with no resolution clears the resolution.
    let mut tx = pool.begin().await.unwrap();
    repo.set_delivery(&mut tx, &k3, 3, DeliveryState::Todo, None, None, now)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let reopened = repo.get_story(&k3).await.unwrap();
    assert_eq!(reopened.delivery(), DeliveryState::Todo);
    assert_eq!(reopened.resolution(), None);
    assert_eq!(reopened.version(), 4);
    let counts = repo.story_counts_under_initiative(&k1).await.unwrap();
    assert_eq!((counts.completed, counts.dropped, counts.total), (0, 0, 1));

    // update_story, update_epic and update_initiative: success, stale write, missing row.
    let mut tx = pool.begin().await.unwrap();
    repo.update_story(&mut tx, &reopened, None, 4, now)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(repo.get_story(&k3).await.unwrap().version(), 5);
    let mut tx = pool.begin().await.unwrap();
    assert!(matches!(
        repo.update_story(&mut tx, &reopened, None, 4, now)
            .await
            .unwrap_err(),
        reeve_planning::RepoError::Conflict { current: 5 }
    ));
    tx.rollback().await.unwrap();

    // A content update writes rank only when one is supplied: `reopened` carries the rank read earlier, and a rerank
    // committed since must survive an update that supplies none.
    let stale_rank = reopened.rank();
    let mut tx = pool.begin().await.unwrap();
    repo.set_ranks(&mut tx, &[(k3.clone(), stale_rank + 7)])
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    repo.update_story(&mut tx, &reopened, None, 5, now)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let kept = repo.get_story(&k3).await.unwrap();
    assert_eq!(
        kept.rank(),
        stale_rank + 7,
        "a rank-less update restored a stale rank"
    );
    let mut tx = pool.begin().await.unwrap();
    repo.update_story(&mut tx, &reopened, Some(3), 6, now)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(repo.get_story(&k3).await.unwrap().rank(), 3);

    let mut tx = pool.begin().await.unwrap();
    repo.update_epic(&mut tx, &epic, 1, now).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(repo.get_epic(&k2).await.unwrap().version(), 2);
    let mut tx = pool.begin().await.unwrap();
    assert!(matches!(
        repo.update_epic(&mut tx, &epic, 1, now).await.unwrap_err(),
        reeve_planning::RepoError::Conflict { current: 2 }
    ));
    tx.rollback().await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    repo.update_initiative(&mut tx, &ini, 1, now).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(repo.get_initiative(&k1).await.unwrap().version(), 2);
    let mut tx = pool.begin().await.unwrap();
    assert!(matches!(
        repo.update_initiative(&mut tx, &ini, 1, now)
            .await
            .unwrap_err(),
        reeve_planning::RepoError::Conflict { current: 2 }
    ));
    tx.rollback().await.unwrap();

    let ghost: reeve_protocol::Key = "ABC-99".parse().unwrap();
    let mut tx = pool.begin().await.unwrap();
    let ghost_epic = Epic::new(ghost.clone(), "Ghost", None, "").unwrap();
    assert!(matches!(
        repo.update_epic(&mut tx, &ghost_epic, 1, now)
            .await
            .unwrap_err(),
        reeve_planning::RepoError::NotFound
    ));
    assert!(matches!(
        repo.set_delivery(&mut tx, &ghost, 1, DeliveryState::Done, None, None, now)
            .await
            .unwrap_err(),
        reeve_planning::RepoError::NotFound
    ));
    tx.rollback().await.unwrap();
    assert!(matches!(
        repo.get_story(&ghost).await.unwrap_err(),
        reeve_planning::RepoError::NotFound
    ));

    // Saving a decision's check result round-trips through JSONB.
    let mut tx = pool.begin().await.unwrap();
    let mut d2 = repo.get_decision(d.id).await.unwrap();
    d2.check = CheckResult::Accepted {
        commit: "abc123".into(),
        reversibility: None,
    };
    d2.checked_at = Some(now);
    repo.save_decision(&mut tx, &d2).await.unwrap();
    tx.commit().await.unwrap();
    let after_d = repo.get_decision(d.id).await.unwrap();
    assert_eq!(
        after_d.check,
        CheckResult::Accepted {
            commit: "abc123".into(),
            reversibility: None,
        }
    );
    assert_eq!(after_d.version, 2);
    let mut tx = pool.begin().await.unwrap();
    assert!(
        matches!(
            repo.save_decision(&mut tx, &d2).await.unwrap_err(),
            reeve_planning::RepoError::Conflict { current: 2 }
        ),
        "saving with the old version is refused"
    );
    tx.rollback().await.unwrap();

    // A second story without an epic: untriaged, labelled, blocked by the first.
    let mut tx = pool.begin().await.unwrap();
    let ku = repo.next_key(&mut tx, "ABC").await.unwrap();
    let loose = Story::new(
        ku.clone(),
        "Loose",
        Kind::Story,
        None,
        "",
        vec![],
        None,
        vec!["watches".to_string()],
    )
    .unwrap();
    assert!(
        repo.insert_story(&mut tx, &loose, "idem-loose", now)
            .await
            .unwrap()
    );
    repo.add_block(&mut tx, &ku, &k3).await.unwrap();
    repo.add_block(&mut tx, &ku, &k3).await.unwrap();
    let input = repo
        .readiness_inputs(&mut tx, std::slice::from_ref(&ku))
        .await
        .unwrap()
        .remove(&ku)
        .unwrap();
    assert_eq!(input.open_blockers, vec![k3.clone()]);
    tx.commit().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert_eq!(
        repo.blockers_of_many(&mut tx, std::slice::from_ref(&ku))
            .await
            .unwrap()
            .remove(&ku)
            .unwrap(),
        vec![k3.clone()]
    );
    tx.rollback().await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    assert!(
        repo.add_block(&mut tx, &ku, &ku).await.is_err(),
        "a story cannot block itself"
    );
    drop(tx);

    // Comments: the same external_ref returns the existing row.
    let mut tx = pool.begin().await.unwrap();
    let (c1, fresh_c) = repo
        .insert_comment(
            &mut tx,
            Some(&k3),
            None,
            "dan@example.com",
            "hi",
            Some("jira-c1"),
            None,
            now,
        )
        .await
        .unwrap();
    let (c2, fresh_c2) = repo
        .insert_comment(
            &mut tx,
            Some(&k3),
            None,
            "dan@example.com",
            "hi again",
            Some("jira-c1"),
            None,
            now,
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(fresh_c && !fresh_c2);
    assert_eq!(c1, c2);
    assert_eq!(repo.list_comments_for_story(&k3).await.unwrap().len(), 1);

    // Filters.
    let keys = |v: Vec<Story>| v.iter().map(|s| s.key().to_string()).collect::<Vec<_>>();
    let f = |f: StoryFilter| {
        let repo = repo.clone();
        let pool = pool.clone();
        async move {
            let mut tx = pool.begin().await.unwrap();
            let found = repo.list_stories(&mut tx, &f, None).await.unwrap();
            tx.rollback().await.unwrap();
            keys(found)
        }
    };
    assert_eq!(
        f(StoryFilter {
            untriaged: true,
            ..Default::default()
        })
        .await,
        [ku.to_string()]
    );
    assert_eq!(
        f(StoryFilter {
            label: Some("watches".into()),
            ..Default::default()
        })
        .await,
        [ku.to_string()]
    );
    assert_eq!(
        f(StoryFilter {
            epic: Some(k2.clone()),
            ..Default::default()
        })
        .await,
        [k3.to_string()]
    );
    assert_eq!(
        f(StoryFilter {
            owner: Some("dan@example.com".into()),
            ..Default::default()
        })
        .await,
        [k3.to_string()]
    );
    assert_eq!(
        f(StoryFilter {
            delivery: Some(DeliveryState::Todo),
            ..Default::default()
        })
        .await
        .len(),
        2
    );
    assert_eq!(
        f(StoryFilter {
            delivery: Some(DeliveryState::Done),
            ..Default::default()
        })
        .await
        .len(),
        0
    );
}

#[tokio::test]
async fn statement_counter_counts_each_statement() {
    let (_c, pool) = common::pg_counted().await;
    let one = common::statements(&pool, || async {
        sqlx::query("SELECT 1").execute(&pool).await.unwrap();
    })
    .await;
    let three = common::statements(&pool, || async {
        for _ in 0..3 {
            sqlx::query("SELECT 1").execute(&pool).await.unwrap();
        }
    })
    .await;
    assert_eq!(three - one, 2, "one={one} three={three}");
    let tx_cost = common::statements(&pool, || async {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SELECT 1").execute(&mut *tx).await.unwrap();
        tx.rollback().await.unwrap();
    })
    .await;
    assert_eq!(tx_cost - one, 2, "BEGIN and ROLLBACK are counted");
    let concurrent = common::statements(&pool, || async {
        let (a, b) = tokio::join!(
            sqlx::query("SELECT pg_sleep(0.2)").execute(&pool),
            sqlx::query("SELECT 1").execute(&pool)
        );
        a.unwrap();
        b.unwrap();
    })
    .await;
    assert_eq!(
        concurrent - one,
        1,
        "statements on a second connection are counted"
    );
}

struct NoSink;
#[async_trait]
impl ActivitySink for NoSink {
    async fn record(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _rec: ActivityRecord,
    ) -> Result<(), RepoError> {
        Ok(())
    }
}

fn handlers(pool: &sqlx::PgPool) -> Handlers {
    Handlers {
        repo: make_planning_repo(pool.clone()),
        activity: Arc::new(NoSink),
        pool: pool.clone(),
        settings: Settings {
            key_prefix: "ABC".into(),
            horizon_buckets: vec!["Now".into(), "Later".into()],
            repos: vec!["knowledge-base".into(), "kb".into()],
            done_window_days: 14,
            sprint_length_days: 14,
            upkeep_line: 20,
        },
    }
}

fn dan() -> Ctx {
    Ctx {
        actor: "dan@example.com".into(),
        via: Via::Interactive,
        now: Utc.with_ymd_and_hms(2026, 10, 26, 9, 0, 0).unwrap(),
    }
}

async fn story(
    h: &Handlers,
    idem: &str,
    kind: Kind,
    epic: Option<&Key>,
    criteria: &[&str],
) -> Story {
    h.create_story(
        &dan(),
        idem,
        idem,
        kind,
        epic.cloned(),
        "",
        criteria.iter().map(|c| c.to_string()).collect(),
        vec![],
        None,
        None,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn stories_by_keys_keeps_the_order_given() {
    let (_c, pool) = common::pg().await;
    let h = handlers(&pool);
    let k1 = story(&h, "s1", Kind::Story, None, &[]).await.key().clone();
    let k2 = story(&h, "s2", Kind::Story, None, &[]).await.key().clone();
    let k3 = story(&h, "s3", Kind::Story, None, &[]).await.key().clone();
    let missing: Key = "ABC-99".parse().unwrap();
    let mut tx = pool.begin().await.unwrap();
    let got = h
        .repo
        .stories_by_keys(&mut tx, &[k3.clone(), k1.clone(), missing, k2.clone()])
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        got.iter().map(|s| s.key().clone()).collect::<Vec<_>>(),
        vec![k3, k1, k2]
    );
}

#[tokio::test]
async fn batched_readiness_equals_one_key_at_a_time() {
    let (_c, pool) = common::pg().await;
    let h = handlers(&pool);
    let c = dan();
    let i1 = h
        .create_initiative(&c, "i1", "I1", "", "Now", None, None, None)
        .await
        .unwrap();
    let i2 = h
        .create_initiative(&c, "i2", "I2", "", "Now", None, None, None)
        .await
        .unwrap();
    h.mark_discovery(&c, ItemType::Initiative, i1.key(), i1.version(), true)
        .await
        .unwrap();
    let e1 = h
        .create_epic(&c, "e1", "E1", Some(i1.key().clone()), "", None)
        .await
        .unwrap();
    let e2 = h
        .create_epic(&c, "e2", "E2", Some(i2.key().clone()), "", None)
        .await
        .unwrap();
    let e3 = h.create_epic(&c, "e3", "E3", None, "", None).await.unwrap();
    h.mark_discovery(&c, ItemType::Epic, e1.key(), e1.version(), true)
        .await
        .unwrap();

    let s1 = story(&h, "S1", Kind::Story, Some(e1.key()), &["c"]).await;
    let s2 = story(&h, "S2", Kind::Story, Some(e2.key()), &[]).await;
    let s3 = story(&h, "S3", Kind::Story, None, &["c"]).await;
    let b1 = story(&h, "B1", Kind::Bug, None, &["c"]).await;
    let c1 = story(&h, "C1", Kind::Chore, None, &[]).await;
    let s4 = story(&h, "S4", Kind::Story, Some(e1.key()), &["c"]).await;
    let s5 = story(&h, "S5", Kind::Story, Some(e1.key()), &["c"]).await;
    let s6 = story(&h, "S6", Kind::Story, Some(e1.key()), &["c"]).await;
    let d1 = story(&h, "D1", Kind::Story, Some(e1.key()), &["c"]).await;
    let s7 = story(&h, "S7", Kind::Story, Some(e3.key()), &["c"]).await;

    // Decisions: unchecked, no record on I2; closed and accepted on E1; not found on S1.
    let d_i2 = h
        .require_decision(&c, "d-i2", i2.key(), "On I2", "dan", None, None)
        .await
        .unwrap();
    let d_closed = h
        .require_decision(&c, "d-closed", e1.key(), "Closed on E1", "dan", None, None)
        .await
        .unwrap();
    h.close_decision(&c, d_closed.id, ClosureKind::Withdrawn, "no longer needed")
        .await
        .unwrap();
    let d_acc = h
        .require_decision(&c, "d-acc", e1.key(), "Accepted on E1", "dan", None, None)
        .await
        .unwrap();
    let d_s1 = h
        .require_decision(&c, "d-s1", s1.key(), "On S1", "dan", None, None)
        .await
        .unwrap();
    for (d, rid, check) in [
        (
            &d_acc,
            "kb/ADR-0001",
            CheckResult::Accepted {
                commit: "abc".into(),
                reversibility: None,
            },
        ),
        (&d_s1, "kb/ADR-0002", CheckResult::NotFound),
    ] {
        let linked = h
            .link_record(&c, d.id, "kb", "knowledge/adrs/x.md", rid, None)
            .await
            .unwrap();
        h.record_check(&c, d.id, rid, linked.version, check, true)
            .await
            .unwrap();
    }

    h.add_block(&c, s4.key(), s5.key()).await.unwrap();
    h.add_block(&c, s5.key(), s4.key()).await.unwrap();
    h.add_block(&c, s6.key(), d1.key()).await.unwrap();
    let d1m = h
        .move_story(
            &c,
            d1.key(),
            d1.version(),
            DeliveryState::InProgress,
            None,
            None,
        )
        .await
        .unwrap();
    h.move_story(
        &c,
        d1.key(),
        d1m.version(),
        DeliveryState::Done,
        Some(Resolution::Completed),
        None,
    )
    .await
    .unwrap();

    let all: Vec<Key> = [&s1, &s2, &s3, &b1, &c1, &s4, &s5, &s6, &d1, &s7]
        .iter()
        .map(|s| s.key().clone())
        .collect();
    let mut tx = pool
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .unwrap();
    let batched = h.repo.readiness_inputs(&mut tx, &all).await.unwrap();
    assert_eq!(batched.len(), all.len());
    let reasons_of = |k: &Key| compute_readiness(&batched[k]).reasons;
    for k in &all {
        let one = h
            .repo
            .readiness_inputs(&mut tx, std::slice::from_ref(k))
            .await
            .unwrap();
        assert_eq!(
            compute_readiness(&one[k]),
            compute_readiness(&batched[k]),
            "{k}"
        );
    }
    let unknown: Key = "ABC-999".parse().unwrap();
    assert!(
        h.repo
            .readiness_inputs(&mut tx, &[unknown])
            .await
            .unwrap()
            .is_empty()
    );
    tx.rollback().await.unwrap();

    assert_eq!(
        reasons_of(s1.key()),
        vec![Reason::DecisionRecordNotFound {
            decision_id: d_s1.id,
            title: "On S1".into()
        }]
    );
    assert_eq!(
        reasons_of(s2.key()),
        vec![
            Reason::AncestorDiscoveryIncomplete {
                item: e2.key().clone()
            },
            Reason::AncestorDiscoveryIncomplete {
                item: i2.key().clone()
            },
            Reason::DecisionNoRecord {
                decision_id: d_i2.id,
                title: "On I2".into()
            },
            Reason::NoCriteria,
        ]
    );
    assert_eq!(reasons_of(s3.key()), vec![Reason::Untriaged]);
    assert_eq!(reasons_of(b1.key()), vec![]);
    assert_eq!(reasons_of(c1.key()), vec![Reason::NoCriteria]);
    assert_eq!(
        reasons_of(s4.key()),
        vec![Reason::BlockedBy {
            story: s5.key().clone()
        }]
    );
    assert_eq!(
        reasons_of(s5.key()),
        vec![Reason::BlockedBy {
            story: s4.key().clone()
        }]
    );
    assert_eq!(reasons_of(s6.key()), vec![]);
    assert_eq!(reasons_of(d1.key()), vec![]);
    assert_eq!(
        reasons_of(s7.key()),
        vec![Reason::AncestorDiscoveryIncomplete {
            item: e3.key().clone()
        }]
    );
}

/// Seeds `n` stories (idempotency keys prefixed `tag`), spread over `epics` new epics (each under its own new initiative), one decision per
/// story and a blocker edge to the previous story; returns their keys.
async fn seed(h: &Handlers, tag: &str, n: usize, epics: usize) -> Vec<Key> {
    let c = dan();
    let mut epic_keys = vec![];
    for e in 0..epics {
        let i = h
            .create_initiative(&c, &format!("{tag}-i{e}"), "I", "", "Now", None, None, None)
            .await
            .unwrap();
        let ep = h
            .create_epic(
                &c,
                &format!("{tag}-e{e}"),
                "E",
                Some(i.key().clone()),
                "",
                None,
            )
            .await
            .unwrap();
        epic_keys.push(ep.key().clone());
    }
    let mut keys: Vec<Key> = vec![];
    for j in 0..n {
        let s = story(
            h,
            &format!("{tag}-s{j}"),
            Kind::Story,
            Some(&epic_keys[j % epics]),
            &["c"],
        )
        .await;
        h.require_decision(&c, &format!("{tag}-d{j}"), s.key(), "D", "dan", None, None)
            .await
            .unwrap();
        if let Some(prev) = keys.last() {
            h.add_block(&c, s.key(), prev).await.unwrap();
        }
        keys.push(s.key().clone());
    }
    keys
}

#[tokio::test]
async fn readiness_inputs_costs_the_same_for_three_and_thirty_stories() {
    let (_c, pool) = common::pg_counted().await;
    let h = handlers(&pool);
    let mut keys = seed(&h, "a", 3, 2).await;
    let measure = |keys: Vec<Key>| {
        let pool = pool.clone();
        let repo = h.repo.clone();
        async move {
            common::statements(&pool.clone(), || async move {
                let mut tx = pool
                    .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
                    .await
                    .unwrap();
                repo.readiness_inputs(&mut tx, &keys).await.unwrap();
                tx.rollback().await.unwrap();
            })
            .await
        }
    };
    let three = measure(keys.clone()).await;
    keys.extend(seed(&h, "b", 27, 9).await);
    assert_eq!(keys.len(), 30);
    let thirty = measure(keys).await;
    assert_eq!(three, thirty);
}

async fn done_at_of(pool: &sqlx::PgPool, key: &Key) -> Option<chrono::DateTime<Utc>> {
    sqlx::query_scalar("SELECT done_at FROM stories WHERE key = $1")
        .bind(key.to_string())
        .fetch_one(pool)
        .await
        .unwrap()
}

fn day(d: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, d, 9, 0, 0).unwrap()
}

/// Moves `key` to `to` through the repository at `now`, reading the current version first.
async fn move_to(
    pool: &sqlx::PgPool,
    key: &Key,
    to: DeliveryState,
    resolution: Option<Resolution>,
    now: chrono::DateTime<Utc>,
) {
    let repo = make_planning_repo(pool.clone());
    let mut tx = pool.begin().await.unwrap();
    let v = repo
        .get_story_for_update(&mut tx, key)
        .await
        .unwrap()
        .version();
    repo.set_delivery(&mut tx, key, v, to, resolution, None, now)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn done_at_follows_delivery() {
    let (_c, pool) = common::pg().await;
    let h = handlers(&pool);
    let k = story(&h, "s", Kind::Story, None, &[]).await.key().clone();
    let done = Some(Resolution::Completed);

    move_to(&pool, &k, DeliveryState::InProgress, None, day(1)).await;
    assert_eq!(done_at_of(&pool, &k).await, None);
    move_to(&pool, &k, DeliveryState::Done, done, day(2)).await;
    assert_eq!(done_at_of(&pool, &k).await, Some(day(2)));
    move_to(
        &pool,
        &k,
        DeliveryState::Done,
        Some(Resolution::Dropped),
        day(3),
    )
    .await;
    assert_eq!(done_at_of(&pool, &k).await, Some(day(2)));
    move_to(&pool, &k, DeliveryState::Todo, None, day(4)).await;
    assert_eq!(done_at_of(&pool, &k).await, None);
}

async fn insert_imported_done(pool: &sqlx::PgPool) -> Key {
    let repo = make_planning_repo(pool.clone());
    let mut tx = pool.begin().await.unwrap();
    let k = repo.next_key(&mut tx, "ABC").await.unwrap();
    let s = Story::from_persisted(
        k.clone(),
        "Old".into(),
        Kind::Story,
        None,
        String::new(),
        vec![],
        None,
        vec![],
        0,
        DeliveryState::Done,
        Some(Resolution::Completed),
        Some("SCRUM-1".into()),
        1,
    );
    assert!(repo.insert_story(&mut tx, &s, "imp", day(1)).await.unwrap());
    tx.commit().await.unwrap();
    k
}

#[tokio::test]
async fn done_at_stays_null_on_an_imported_done_story_until_it_leaves_done() {
    let (_c, pool) = common::pg().await;
    let k = insert_imported_done(&pool).await;
    assert_eq!(done_at_of(&pool, &k).await, None);
    move_to(
        &pool,
        &k,
        DeliveryState::Done,
        Some(Resolution::Completed),
        day(2),
    )
    .await;
    assert_eq!(done_at_of(&pool, &k).await, None);
    move_to(&pool, &k, DeliveryState::InProgress, None, day(3)).await;
    assert_eq!(done_at_of(&pool, &k).await, None);
    move_to(
        &pool,
        &k,
        DeliveryState::Done,
        Some(Resolution::Completed),
        day(5),
    )
    .await;
    assert_eq!(done_at_of(&pool, &k).await, Some(day(5)));
}

#[tokio::test]
async fn the_backfill_dates_native_done_stories_only() {
    let (_c, pool) = common::pg_unmigrated().await;
    let mut partial = reeve_planning::migrator();
    partial.migrations = std::borrow::Cow::Owned(
        partial
            .migrations
            .iter()
            .filter(|m| m.version < 1005)
            .cloned()
            .collect(),
    );
    partial.run(&pool).await.unwrap();
    let t0 = day(7);
    for (key, delivery, ext) in [
        ("ABC-1", "done", None),
        ("ABC-2", "done", Some("SCRUM-1")),
        ("ABC-3", "todo", None),
    ] {
        sqlx::query("INSERT INTO stories (key, title, kind, delivery, external_ref, version, idempotency_key, created_at, updated_at) VALUES ($1, 't', 'story', $2, $3, 1, $1, $5, $4)")
            .bind(key).bind(delivery).bind(ext).bind(t0).bind(day(6)).execute(&pool).await.unwrap();
    }
    reeve_planning::migrator().run(&pool).await.unwrap();
    for (k, want) in [("ABC-1", Some(t0)), ("ABC-2", None), ("ABC-3", None)] {
        assert_eq!(done_at_of(&pool, &k.parse().unwrap()).await, want, "{k}");
    }
}

#[tokio::test]
async fn list_stories_with_a_cutoff() {
    let (_c, pool) = common::pg().await;
    let h = handlers(&pool);
    let cutoff = day(10);
    let mk = |i: &'static str| {
        let h = &h;
        async move { story(h, i, Kind::Story, None, &[]).await.key().clone() }
    };
    let t = mk("t").await;
    let p = mk("p").await;
    let r = mk("r").await;
    let e = mk("e").await;
    let o = mk("o").await;
    let n = insert_imported_done(&pool).await;
    move_to(&pool, &p, DeliveryState::InProgress, None, day(1)).await;
    let done = Some(Resolution::Completed);
    for (k, at) in [
        (&r, cutoff + chrono::Duration::seconds(1)),
        (&e, cutoff),
        (&o, cutoff - chrono::Duration::seconds(1)),
    ] {
        move_to(&pool, k, DeliveryState::Done, done, at).await;
    }
    let repo = make_planning_repo(pool.clone());
    let list = |f: StoryFilter, since: Option<chrono::DateTime<Utc>>| {
        let (repo, pool) = (repo.clone(), pool.clone());
        async move {
            let mut tx = pool.begin().await.unwrap();
            let got = repo.list_stories(&mut tx, &f, since).await.unwrap();
            tx.rollback().await.unwrap();
            got.iter().map(|s| s.key().clone()).collect::<Vec<Key>>()
        }
    };
    let sorted = |mut v: Vec<Key>| {
        v.sort_by_key(|k| k.to_string());
        v
    };
    assert_eq!(
        sorted(list(StoryFilter::default(), Some(cutoff)).await),
        sorted(vec![t.clone(), p.clone(), r.clone(), e.clone()])
    );
    assert_eq!(
        sorted(list(StoryFilter::default(), None).await),
        sorted(vec![t, p, r.clone(), e.clone(), o, n])
    );
    let only_done = StoryFilter {
        delivery: Some(DeliveryState::Done),
        ..Default::default()
    };
    assert_eq!(
        sorted(list(only_done, Some(cutoff)).await),
        sorted(vec![r, e])
    );
}

/// The three new facts survive a save and a read: reversibility set at insert, closure kind and closer's token kind
/// set at close. Catches a column left out of DECISION_COLS, decision_from or save_decision.
#[tokio::test]
async fn decision_reversibility_and_closure_round_trip() {
    let (_c, pool) = common::pg().await;
    let h = handlers(&pool);
    let s = story(&h, "rt-story", Kind::Story, None, &["works"]).await;
    let repo = make_planning_repo(pool.clone());
    let mut tx = pool.begin().await.unwrap();
    let (d, _) = repo
        .insert_decision(
            &mut tx,
            ItemType::Story,
            s.key(),
            "Pick a db",
            "dan@example.com",
            None,
            Some(Reversibility::OneWay),
            "rt-1",
            dan().now,
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        repo.get_decision(d.id).await.unwrap().reversibility,
        Some(Reversibility::OneWay)
    );

    let mut d = repo.get_decision(d.id).await.unwrap();
    d.closed = Some(Closed {
        at: dan().now,
        by: "dan@example.com".into(),
        reason: "not needed".into(),
        kind: Some(ClosureKind::Withdrawn),
        via: Some(TokenKind::Automation),
    });
    let mut tx = pool.begin().await.unwrap();
    repo.save_decision(&mut tx, &d).await.unwrap();
    tx.commit().await.unwrap();
    let c = repo.get_decision(d.id).await.unwrap().closed.unwrap();
    assert_eq!(
        (c.kind, c.via),
        (Some(ClosureKind::Withdrawn), Some(TokenKind::Automation))
    );
}

/// The CHECKs refuse values outside the enums (`system` never closes), and a row closed before this migration reads
/// back with no kind and no via. Catches a missing CHECK and a decision_from that defaults legacy closes to a kind.
#[tokio::test]
async fn decision_columns_refuse_unknown_values_and_read_legacy_closes() {
    let (_c, pool) = common::pg().await;
    let h = handlers(&pool);
    let s = story(&h, "ck-story", Kind::Story, None, &["works"]).await;
    let d = h
        .require_decision(
            &dan(),
            "ck-1",
            s.key(),
            "Pick",
            "dan@example.com",
            None,
            None,
        )
        .await
        .unwrap();
    for (col, v) in [
        ("reversibility", "sideways"),
        ("closed_kind", "maybe"),
        ("closed_via", "system"),
    ] {
        let r = sqlx::query(&format!(
            "UPDATE required_decisions SET {col} = $1 WHERE id = $2"
        ))
        .bind(v)
        .bind(d.id)
        .execute(&pool)
        .await;
        assert!(
            matches!(r, Err(sqlx::Error::Database(ref e)) if e.code().as_deref() == Some("23514")),
            "{col} = {v} must be refused by its CHECK"
        );
    }
    sqlx::query(
        "UPDATE required_decisions SET closed_at = now(), closed_by = 'dan@example.com', \
         closed_reason = 'old' WHERE id = $1",
    )
    .bind(d.id)
    .execute(&pool)
    .await
    .unwrap();
    let c = make_planning_repo(pool.clone())
        .get_decision(d.id)
        .await
        .unwrap()
        .closed
        .unwrap();
    assert_eq!((c.reason.as_str(), c.kind, c.via), ("old", None, None));
}
