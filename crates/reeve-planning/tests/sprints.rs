mod common;
use chrono::{TimeZone, Utc};

/// The indexes carry kb/ADR-0014 R1: a second draft or a second active sprint is a unique violation.
#[tokio::test]
async fn at_most_one_draft_and_one_active_sprint() {
    let (_c, pool) = common::pg().await;
    let now = Utc.with_ymd_and_hms(2026, 11, 3, 18, 0, 0).unwrap();
    let ins = |state: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query("INSERT INTO sprints (state, starts_on, ends_on, opened_at, created_at) VALUES ($1, '2026-11-03', '2026-11-17', CASE WHEN $1 = 'draft' THEN NULL ELSE $2 END, $2)")
                .bind(state).bind(now).execute(&pool).await
        }
    };
    ins("draft").await.unwrap();
    let e = ins("draft").await.unwrap_err();
    assert_eq!(
        e.as_database_error().unwrap().code().as_deref(),
        Some("23505")
    );
    ins("active").await.unwrap();
    let e = ins("active").await.unwrap_err();
    assert_eq!(
        e.as_database_error().unwrap().code().as_deref(),
        Some("23505")
    );
}

fn t() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 3, 18, 0, 0).unwrap()
}

async fn closed(pool: &sqlx::PgPool, d: u32) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query("INSERT INTO sprints (state, starts_on, ends_on, opened_at, closed_at, created_at) VALUES ('closed', '2026-10-01', '2026-10-15', $1, $2, $1)")
        .bind(t()).bind(t() + chrono::Duration::days(d as i64)).execute(pool).await
}

#[tokio::test]
async fn many_closed_sprints_are_allowed() {
    let (_c, pool) = common::pg().await;
    closed(&pool, 1).await.unwrap();
    closed(&pool, 2).await.unwrap();
}

async fn sprint(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar("INSERT INTO sprints (state, starts_on, ends_on, created_at) VALUES ('draft', '2026-11-03', '2026-11-17', $1) RETURNING id")
        .bind(t()).fetch_one(pool).await.unwrap()
}

async fn goal(
    pool: &sqlx::PgPool,
    sid: i64,
    extra: &str,
    key: &str,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    // `extra` is a fixed test literal: "col = val" pairs appended as extra columns.
    let (cols, vals) = if extra.is_empty() {
        (String::new(), String::new())
    } else {
        let (c, v) = extra.split_once('=').unwrap();
        (format!(", {}", c.trim()), format!(", {}", v.trim()))
    };
    sqlx::query(&format!("INSERT INTO goals (sprint_id, outcome, check_text, owner, idempotency_key, created_at{cols}) VALUES ($1, 'o', 'c', 'dan', $2, $3{vals})"))
        .bind(sid).bind(key).bind(t()).execute(pool).await
}

#[tokio::test]
async fn goal_links_cascade_and_are_unique() {
    let (_c, pool) = common::pg().await;
    let sid = sprint(&pool).await;
    goal(&pool, sid, "", "k1").await.unwrap();
    let gid: i64 = sqlx::query_scalar("SELECT id FROM goals")
        .fetch_one(&pool)
        .await
        .unwrap();
    let link = || {
        sqlx::query("INSERT INTO goal_links (goal_id, item_key, linked_at, linked_by) VALUES ($1, 'ABC-1', $2, 'dan')").bind(gid).bind(t())
    };
    link().execute(&pool).await.unwrap();
    let e = link().execute(&pool).await.unwrap_err();
    assert_eq!(
        e.as_database_error().unwrap().code().as_deref(),
        Some("23505")
    );
    sqlx::query("DELETE FROM goals WHERE id = $1")
        .bind(gid)
        .execute(&pool)
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM goal_links")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn every_check_rejects_a_violating_row() {
    let (_c, pool) = common::pg().await;
    let sid = sprint(&pool).await;
    let is_check = |r: Result<sqlx::postgres::PgQueryResult, sqlx::Error>, what: &str| {
        let e = r.expect_err(what);
        assert_eq!(
            e.as_database_error().unwrap().code().as_deref(),
            Some("23514"),
            "{what}"
        );
    };
    // closed_at must not precede opened_at (equal is allowed, see below)
    is_check(
        sqlx::query("INSERT INTO sprints (state, starts_on, ends_on, opened_at, closed_at, created_at) VALUES ('closed', '2026-10-01', '2026-10-15', $1, $1 - interval '1 second', $1)").bind(t()).execute(&pool).await,
        "closed_at before opened_at",
    );
    is_check(
        sqlx::query("INSERT INTO sprints (state, starts_on, ends_on, created_at) VALUES ('draft', '2026-10-15', '2026-10-01', $1)").bind(t()).execute(&pool).await,
        "ends before starts",
    );
    is_check(
        sqlx::query("INSERT INTO goals (sprint_id, outcome, check_text, owner, idempotency_key, created_at) VALUES ($1, 'o', 'c', ' ', 'a', $2)").bind(sid).bind(t()).execute(&pool).await,
        "blank owner",
    );
    is_check(
        goal(&pool, sid, "confidence = 'on_track'", "b").await,
        "confidence without confidence_at",
    );
    is_check(
        goal(&pool, sid, "suggested_confidence = 'at_risk'", "c").await,
        "suggestion without at/by",
    );
    is_check(
        goal(&pool, sid, "verdict_note = 'x'", "d").await,
        "verdict_note without verdict",
    );
}

#[tokio::test]
async fn open_and_close_at_the_same_instant_is_allowed() {
    let (_c, pool) = common::pg().await;
    closed(&pool, 0).await.unwrap();
}

mod started_at {
    use async_trait::async_trait;
    use reeve_planning::*;
    use reeve_protocol::{DeliveryState, Kind, Via};
    use std::sync::{Arc, Mutex};

    pub struct MemSink(pub Mutex<Vec<ActivityRecord>>);
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
    pub fn settings() -> Settings {
        Settings {
            key_prefix: "ABC".into(),
            horizon_buckets: vec!["Now".into()],
            repos: vec![],
            done_window_days: 14,
            sprint_length_days: 14,
            upkeep_line: 20,
        }
    }
    pub fn mk(pool: sqlx::PgPool) -> Handlers {
        let sink = Arc::new(MemSink(Mutex::new(vec![])));
        Handlers {
            repo: make_planning_repo(pool.clone()),
            activity: sink,
            pool,
            settings: settings(),
        }
    }
    pub fn at(day: u32, h: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 11, day, h, 0, 0).unwrap()
    }
    pub fn ctx(via: Via, day: u32) -> Ctx {
        Ctx {
            actor: "dan@example.com".into(),
            via,
            now: at(day, 18),
        }
    }

    /// started_at moves on every start from todo or done and never on a move between active states or into done.
    #[tokio::test]
    async fn started_at_is_set_on_each_start_only() {
        let (_c, pool) = super::common::pg().await;
        let h = mk(pool.clone());
        let c = ctx(Via::Interactive, 3);
        let s = h
            .create_story(
                &c,
                "s",
                "S",
                Kind::Chore,
                None,
                "",
                vec!["x".into()],
                vec![],
                None,
                None,
            )
            .await
            .unwrap();
        let started = || {
            let pool = pool.clone();
            let k = s.key().to_string();
            async move {
                sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
                    "SELECT started_at FROM stories WHERE key = $1",
                )
                .bind(k)
                .fetch_one(&pool)
                .await
                .unwrap()
            }
        };
        let s = h
            .move_story(
                &ctx(Via::Interactive, 4),
                s.key(),
                s.version(),
                DeliveryState::InProgress,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(started().await, Some(at(4, 18)));
        let s = h
            .move_story(
                &ctx(Via::Interactive, 5),
                s.key(),
                s.version(),
                DeliveryState::InReview,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            started().await,
            Some(at(4, 18)),
            "a move between active states is not a start"
        );
        let s = h
            .move_story(
                &ctx(Via::Interactive, 6),
                s.key(),
                s.version(),
                DeliveryState::Done,
                Some(reeve_protocol::Resolution::Completed),
                None,
            )
            .await
            .unwrap();
        assert_eq!(started().await, Some(at(4, 18)), "done keeps it");
        h.move_story(
            &ctx(Via::Interactive, 7),
            s.key(),
            s.version(),
            DeliveryState::InProgress,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            started().await,
            Some(at(7, 18)),
            "a restart from done is a start"
        );
    }
}

mod repository {
    use super::started_at::{at, ctx, mk};
    use reeve_planning::*;
    use reeve_protocol::{DeliveryState, GoalVia, Kind, Resolution, SprintState, Via};

    /// The repository returns what the handlers need: one draft at most, idempotent goals, links by id, a story's
    /// active goals through itself, its epic and its initiative, and the story rows `summarise` reads.
    #[tokio::test]
    async fn sprints_goals_links_and_story_times() {
        let (_c, pool) = super::common::pg().await;
        let h = mk(pool.clone());
        let c = ctx(Via::Interactive, 3);
        let init = h
            .create_initiative(&c, "i", "I", "", "Now", None, None, None)
            .await
            .unwrap();
        let epic = h
            .create_epic(&c, "e", "E", Some(init.key().clone()), "", None)
            .await
            .unwrap();
        let mkstory = |idem: &'static str, epic: Option<reeve_protocol::Key>| {
            let h = h.clone();
            let c = c.clone();
            async move {
                h.create_story(
                    &c,
                    idem,
                    idem,
                    Kind::Chore,
                    epic,
                    "",
                    vec!["x".into()],
                    vec![],
                    None,
                    None,
                )
                .await
                .unwrap()
            }
        };
        let direct = mkstory("direct", Some(epic.key().clone())).await;
        let third = mkstory("third", Some(epic.key().clone())).await;
        let loose = mkstory("loose", None).await;
        let done = mkstory("done", None).await;

        let repo = make_planning_repo(pool.clone());
        let mut tx = pool.begin().await.unwrap();
        let d = repo
            .insert_draft(
                &mut tx,
                at(3, 0).date_naive(),
                at(17, 0).date_naive(),
                at(3, 18),
            )
            .await
            .unwrap()
            .unwrap();
        assert!(
            repo.insert_draft(
                &mut tx,
                at(3, 0).date_naive(),
                at(17, 0).date_naive(),
                at(3, 18)
            )
            .await
            .unwrap()
            .is_none(),
            "one draft only"
        );
        assert_eq!(
            repo.sprint_in_state(&mut tx, SprintState::Draft)
                .await
                .unwrap()
                .unwrap()
                .id,
            d.id
        );
        assert!(
            repo.sprint_in_state(&mut tx, SprintState::Active)
                .await
                .unwrap()
                .is_none()
        );

        let (g, fresh) = repo
            .insert_goal(&mut tx, d.id, "ship", "demo", "dan", "", "k1", at(3, 18))
            .await
            .unwrap();
        assert!(fresh);
        let (again, fresh) = repo
            .insert_goal(&mut tx, d.id, "other", "other", "x", "", "k1", at(3, 19))
            .await
            .unwrap();
        assert!(
            (!fresh) && again.id == g.id && again.outcome == "ship",
            "a repeated key answers the first goal"
        );
        for k in [init.key(), epic.key(), direct.key(), loose.key()] {
            assert!(
                repo.insert_goal_link(&mut tx, g.id, k, "dan", at(3, 18))
                    .await
                    .unwrap()
            );
        }
        assert!(
            !repo
                .insert_goal_link(&mut tx, g.id, loose.key(), "dan", at(3, 18))
                .await
                .unwrap()
        );
        let links = repo.links_of_in(&mut tx, &[g.id]).await.unwrap();
        assert_eq!(links.len(), 4);

        // Not active yet: no goals served.
        assert!(
            repo.active_goals_of_stories(&mut tx, &[direct.key().clone()])
                .await
                .unwrap()
                .is_empty()
        );
        let mut s = repo.sprint_for_update(&mut tx, d.id).await.unwrap();
        s.state = SprintState::Active;
        s.opened_at = Some(at(3, 18));
        repo.save_sprint(&mut tx, &s).await.unwrap();
        assert!(
            matches!(
                repo.save_sprint(&mut tx, &s).await,
                Err(RepoError::Conflict { current: 2 })
            ),
            "stale version"
        );
        let served = repo
            .active_goals_of_stories(
                &mut tx,
                &[
                    direct.key().clone(),
                    third.key().clone(),
                    loose.key().clone(),
                    done.key().clone(),
                ],
            )
            .await
            .unwrap();
        // Linked directly, through its epic and through its initiative: one entry, the nearest (story) wins.
        assert_eq!(
            served[direct.key()],
            vec![(g.id, "ship".to_string(), GoalVia::Story)]
        );
        // Through its epic and its initiative: the epic wins.
        assert_eq!(
            served[third.key()],
            vec![(g.id, "ship".to_string(), GoalVia::Epic)]
        );
        assert_eq!(served[loose.key()][0].2, GoalVia::Story);
        assert!(!served.contains_key(done.key()));

        // Goal save keeps the CHECK pairs and bumps the version.
        let mut goal = repo.goal_for_update(&mut tx, g.id).await.unwrap();
        goal.confidence = Some(reeve_protocol::Confidence::AtRisk);
        goal.confidence_at = Some(at(4, 9));
        repo.save_goal(&mut tx, &goal).await.unwrap();
        let back = repo.get_goal_in(&mut tx, g.id).await.unwrap();
        assert_eq!(
            (back.version, back.confidence),
            (2, Some(reeve_protocol::Confidence::AtRisk))
        );
        assert!(matches!(
            repo.save_goal(&mut tx, &goal).await,
            Err(RepoError::Conflict { current: 2 })
        ));
        assert_eq!(
            repo.goals_by_ids_in(&mut tx, &[g.id]).await.unwrap().len(),
            1
        );
        assert_eq!(repo.goals_of_in(&mut tx, &[d.id]).await.unwrap().len(), 1);
        assert_eq!(repo.sprints_in(&mut tx).await.unwrap().len(), 1);
        tx.commit().await.unwrap();

        // Story rows: linked ones through any level, plus a completion inside the window.
        let done = h
            .move_story(
                &ctx(Via::Interactive, 4),
                done.key(),
                done.version(),
                DeliveryState::InProgress,
                None,
                None,
            )
            .await
            .unwrap();
        h.move_story(
            &ctx(Via::Interactive, 5),
            done.key(),
            done.version(),
            DeliveryState::Done,
            Some(Resolution::Completed),
            None,
        )
        .await
        .unwrap();
        let mut tx = pool.begin().await.unwrap();
        let rows = repo
            .sprint_story_times(&mut tx, &[init.key().clone()], at(3, 18), at(17, 18))
            .await
            .unwrap();
        let keys: Vec<_> = rows.iter().map(|r| r.key.to_string()).collect();
        assert_eq!(keys.len(), 3, "{keys:?}"); // direct, third (under the initiative) and the completed one
        let by = |k: &reeve_protocol::Key| rows.iter().find(|r| &r.key == k).unwrap();
        assert_eq!(by(direct.key()).initiative.as_ref(), Some(init.key()));
        assert_eq!(by(direct.key()).epic.as_ref(), Some(epic.key()));
        let dn = by(done.key());
        assert_eq!(
            (dn.started_at, dn.done_at, dn.resolution),
            (
                Some(at(4, 18)),
                Some(at(5, 18)),
                Some(Resolution::Completed)
            )
        );
        assert!(
            rows.iter().all(|r| r.key != *loose.key()),
            "linked only via its own key, which was not asked for"
        );

        // A story key and an epic key cover the story itself and the epic's stories.
        let rows = repo
            .sprint_story_times(
                &mut tx,
                &[loose.key().clone(), epic.key().clone()],
                at(3, 18),
                at(17, 18),
            )
            .await
            .unwrap();
        let keys: Vec<_> = rows.iter().map(|r| r.key.clone()).collect();
        for k in [loose.key(), direct.key(), third.key(), done.key()] {
            assert!(keys.contains(k), "{k} missing from {keys:?}");
        }

        // Removing the link and the goal.
        assert!(
            repo.delete_goal_link(&mut tx, g.id, init.key())
                .await
                .unwrap()
        );
        assert!(
            !repo
                .delete_goal_link(&mut tx, g.id, init.key())
                .await
                .unwrap()
        );
        repo.delete_goal(&mut tx, g.id).await.unwrap();
        assert!(matches!(
            repo.delete_goal(&mut tx, g.id).await,
            Err(RepoError::NotFound)
        ));
        assert!(matches!(
            repo.get_goal_in(&mut tx, g.id).await,
            Err(RepoError::NotFound)
        ));
        assert!(matches!(
            repo.get_sprint_in(&mut tx, 9999).await,
            Err(RepoError::NotFound)
        ));
    }
}

mod locks_and_indexes {
    use super::started_at::at;
    use reeve_planning::*;
    use reeve_protocol::SprintState;

    fn sqlstate(e: RepoError) -> Option<String> {
        match e {
            RepoError::Db(sqlx::Error::Database(d)) => d.code().map(|c| c.to_string()),
            _ => None,
        }
    }

    /// The row locks are real: a second transaction cannot take the same lock (55P03 under a short lock_timeout).
    #[tokio::test]
    async fn sprint_and_goal_lookups_for_update_hold_the_row() {
        let (_c, pool) = super::common::pg().await;
        let repo = make_planning_repo(pool.clone());
        let mut setup = pool.begin().await.unwrap();
        let d = repo
            .insert_draft(
                &mut setup,
                at(3, 0).date_naive(),
                at(17, 0).date_naive(),
                at(3, 18),
            )
            .await
            .unwrap()
            .unwrap();
        let (g, _) = repo
            .insert_goal(&mut setup, d.id, "o", "c", "dan", "", "k", at(3, 18))
            .await
            .unwrap();
        setup.commit().await.unwrap();

        let mut tx1 = pool.begin().await.unwrap();
        repo.sprint_for_update(&mut tx1, d.id).await.unwrap();
        let mut tx2 = pool.begin().await.unwrap();
        sqlx::query("SET LOCAL lock_timeout = '100ms'")
            .execute(&mut *tx2)
            .await
            .unwrap();
        let e = repo.sprint_for_update(&mut tx2, d.id).await.unwrap_err();
        assert_eq!(sqlstate(e).as_deref(), Some("55P03"));
        drop(tx2);
        tx1.rollback().await.unwrap();

        let mut tx1 = pool.begin().await.unwrap();
        repo.goal_for_update(&mut tx1, g.id).await.unwrap();
        let mut tx2 = pool.begin().await.unwrap();
        sqlx::query("SET LOCAL lock_timeout = '100ms'")
            .execute(&mut *tx2)
            .await
            .unwrap();
        let e = repo.goal_for_update(&mut tx2, g.id).await.unwrap_err();
        assert_eq!(sqlstate(e).as_deref(), Some("55P03"));
    }

    /// These constraint names are mapped: a second active sprint and a second draft are unique violations.
    #[tokio::test]
    async fn a_second_active_or_draft_sprint_names_its_index() {
        let (_c, pool) = super::common::pg().await;
        let repo = make_planning_repo(pool.clone());
        let mut tx = pool.begin().await.unwrap();
        let a = repo
            .insert_draft(
                &mut tx,
                at(3, 0).date_naive(),
                at(17, 0).date_naive(),
                at(3, 18),
            )
            .await
            .unwrap()
            .unwrap();
        let mut a = repo.sprint_for_update(&mut tx, a.id).await.unwrap();
        a.state = SprintState::Active;
        a.opened_at = Some(at(3, 18));
        repo.save_sprint(&mut tx, &a).await.unwrap();
        let b = repo
            .insert_draft(
                &mut tx,
                at(17, 0).date_naive(),
                at(30, 0).date_naive(),
                at(3, 19),
            )
            .await
            .unwrap()
            .unwrap();
        let mut b2 = repo.sprint_for_update(&mut tx, b.id).await.unwrap();
        b2.state = SprintState::Active;
        b2.opened_at = Some(at(4, 18));
        sqlx::query("SAVEPOINT s").execute(&mut *tx).await.unwrap();
        let e = repo.save_sprint(&mut tx, &b2).await.unwrap_err();
        let RepoError::Db(sqlx::Error::Database(d)) = e else {
            panic!("{e:?}")
        };
        assert_eq!(
            (d.code().as_deref(), d.constraint()),
            (Some("23505"), Some("sprints_one_active_idx"))
        );
        sqlx::query("ROLLBACK TO SAVEPOINT s")
            .execute(&mut *tx)
            .await
            .unwrap();

        // A second draft through the raw index (insert_draft itself answers None).
        let e = sqlx::query("INSERT INTO sprints (state, starts_on, ends_on, created_at) VALUES ('draft', '2026-12-01', '2026-12-15', $1)")
            .bind(at(5, 18)).execute(&mut *tx).await.unwrap_err();
        assert_eq!(
            e.as_database_error().unwrap().constraint(),
            Some("sprints_one_draft_idx")
        );
    }
}

/// The sprint and goal handlers (kb/ADR-0014 R5, R6): the lifecycle, who may write, the closed state, and the races.
mod lifecycle {
    use super::started_at::{MemSink, at, ctx, settings};
    use async_trait::async_trait;
    use reeve_planning::*;
    use reeve_protocol::{Kind, SprintState, Via};
    use std::sync::{Arc, Mutex};

    pub(super) fn mk(pool: sqlx::PgPool) -> (Handlers, Arc<MemSink>) {
        let sink = Arc::new(MemSink(Mutex::new(vec![])));
        let h = Handlers {
            repo: make_planning_repo(pool.clone()),
            activity: sink.clone(),
            pool,
            settings: settings(),
        };
        (h, sink)
    }

    pub(super) async fn goal(h: &Handlers, c: &Ctx, idem: &str) -> Goal {
        h.add_goal(
            c,
            idem,
            "By Nov 17, customers can book",
            "a customer booked one",
            None,
            "",
            false,
        )
        .await
        .unwrap()
        .0
    }

    pub(super) async fn story(h: &Handlers, c: &Ctx, idem: &str) -> Story {
        h.create_story(
            c,
            idem,
            idem,
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
    }

    #[tokio::test]
    async fn the_cycle_draft_open_confidence_close() {
        let (_c, pool) = super::common::pg().await;
        let (h, sink) = mk(pool.clone());
        let c = ctx(Via::Interactive, 2);
        let g = goal(&h, &c, "g1").await; // creates the first draft
        let draft = h.sprints(at(1, 0)).await.unwrap().draft.unwrap();
        assert_eq!(g.sprint_id, draft.id);
        let opened = h
            .open_sprint(&ctx(Via::Interactive, 3), draft.id, draft.version, None)
            .await
            .unwrap();
        assert_eq!(opened.state, SprintState::Active);
        assert_eq!(opened.opened_at, Some(at(3, 18)));
        let list = h.sprints(at(1, 0)).await.unwrap();
        let next = list.draft.unwrap();
        assert_eq!(
            next.starts_on, opened.ends_on,
            "opening creates the next draft from the active end"
        );
        h.record_confidence(
            &ctx(Via::Interactive, 10),
            g.id,
            reeve_protocol::Confidence::AtRisk,
            Some("review queue".into()),
        )
        .await
        .unwrap();
        let refused = h
            .close_sprint(
                &ctx(Via::Interactive, 17),
                opened.id,
                opened.version,
                vec![],
            )
            .await
            .unwrap_err();
        assert!(
            matches!(refused, HandlerError::Refused(ref r) if r == &vec![reeve_protocol::Reason::VerdictMissing { goals: vec![g.id] }])
        );
        let closed = h
            .close_sprint(
                &ctx(Via::Interactive, 17),
                opened.id,
                opened.version,
                vec![(
                    g.id,
                    reeve_protocol::Verdict::PartlyMet,
                    Some("one booking".into()),
                )],
            )
            .await
            .unwrap();
        assert_eq!(closed.closed_at, Some(at(17, 18)));
        assert_eq!(closed.state, SprintState::Closed);
        let actions: Vec<_> = sink
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|r| (r.entity_type, r.action))
            .collect();
        assert_eq!(
            actions,
            vec![
                ("goal", "created"),
                ("sprint", "opened"),
                ("goal", "confidence_recorded"),
                ("goal", "judged"),
                ("sprint", "closed")
            ]
        );
    }

    fn interactive_only<T: std::fmt::Debug>(r: Result<T, HandlerError>) -> bool {
        matches!(r, Err(HandlerError::Refused(ref x)) if x == &vec![reeve_protocol::Reason::InteractiveOnly])
    }

    /// Every interactive-only write (kb/ADR-0014 R5), as automation and as system, on an active sprint with a draft
    /// beside it, so a state check cannot answer first and mask a missing interactive check. Linking and suggesting a
    /// confidence stay open to both.
    #[tokio::test]
    async fn only_interactive_callers_change_sprints_and_goals() {
        let (_c, pool) = super::common::pg().await;
        let (h, _) = mk(pool.clone());
        let me = ctx(Via::Interactive, 2);
        let g1 = goal(&h, &me, "g1").await;
        let d = h.sprints(at(1, 0)).await.unwrap().draft.unwrap();
        let a = h.open_sprint(&me, d.id, d.version, None).await.unwrap();
        let g2 = goal(&h, &me, "g2").await; // in the new draft
        let d2 = h.sprints(at(1, 0)).await.unwrap().draft.unwrap();
        assert_eq!(g2.sprint_id, d2.id);
        let s = story(&h, &me, "s").await;
        for via in [Via::Automation, Via::System] {
            let x = ctx(via, 4);
            assert!(
                interactive_only(h.open_sprint(&x, d2.id, d2.version, None).await),
                "{via:?} open"
            );
            assert!(
                interactive_only(
                    h.close_sprint(
                        &x,
                        a.id,
                        a.version,
                        vec![(g1.id, reeve_protocol::Verdict::Met, None)]
                    )
                    .await
                ),
                "{via:?} close"
            );
            assert!(
                interactive_only(h.add_goal(&x, "g3", "o", "c", None, "", true).await),
                "{via:?} add"
            );
            assert!(
                interactive_only(
                    h.edit_goal(&x, g1.id, g1.version, Some("x".into()), None, None, None)
                        .await
                ),
                "{via:?} edit goal"
            );
            assert!(
                interactive_only(h.remove_goal(&x, g2.id, g2.version).await),
                "{via:?} remove"
            );
            assert!(
                interactive_only(
                    h.record_confidence(&x, g1.id, reeve_protocol::Confidence::AtRisk, None)
                        .await
                ),
                "{via:?} confidence"
            );
            h.suggest_confidence(
                &x,
                g1.id,
                reeve_protocol::Confidence::AtRisk,
                Some("3 of 7 done, 2 in review".into()),
            )
            .await
            .unwrap();
            h.link_goal(&x, g1.id, std::slice::from_ref(s.key()))
                .await
                .unwrap();
            h.unlink_goal(&x, g1.id, s.key()).await.unwrap();
        }
        // Nothing the refused calls named changed.
        let now = h.sprints(at(1, 0)).await.unwrap();
        assert_eq!(now.active.unwrap().version, a.version);
        assert_eq!(now.draft.unwrap().version, d2.version);
        assert_eq!(now.goals.len(), 2);
    }

    /// kb/ADR-0014 R5: an agent drafts a confidence, a person records it; recording clears the draft.
    #[tokio::test]
    async fn a_suggested_confidence_waits_for_a_person() {
        let (_c, pool) = super::common::pg().await;
        let (h, sink) = mk(pool.clone());
        let me = ctx(Via::Interactive, 2);
        let g = goal(&h, &me, "g1").await;
        let d = h.sprints(at(1, 0)).await.unwrap().draft.unwrap();
        h.open_sprint(&me, d.id, d.version, None).await.unwrap();
        let agent = Ctx {
            actor: "Bot@Example.COM".into(),
            ..ctx(Via::Automation, 9)
        };
        let s = h
            .suggest_confidence(
                &agent,
                g.id,
                reeve_protocol::Confidence::AtRisk,
                Some("2 in review".into()),
            )
            .await
            .unwrap();
        assert_eq!(
            (s.suggested_confidence, s.confidence),
            (Some(reeve_protocol::Confidence::AtRisk), None),
            "a suggestion is not a recording"
        );
        assert_eq!(
            s.suggested_by.as_deref(),
            Some("bot@example.com"),
            "stored normalised, as owner and linked_by are"
        );
        assert_eq!(s.suggested_at, Some(at(9, 18)));
        let r = h
            .record_confidence(
                &ctx(Via::Interactive, 10),
                g.id,
                reeve_protocol::Confidence::OnTrack,
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            (r.confidence, r.suggested_confidence, r.suggested_at),
            (Some(reeve_protocol::Confidence::OnTrack), None, None)
        );
        assert_eq!(r.confidence_at, Some(at(10, 18)));
        let entries: Vec<_> = sink
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.action.starts_with("confidence"))
            .cloned()
            .collect();
        let actions: Vec<_> = entries.iter().map(|e| e.action).collect();
        assert_eq!(actions, vec!["confidence_suggested", "confidence_recorded"]);
        assert_eq!(entries[1].payload["suggested"]["confidence"], "at_risk");
    }

    /// The goal as it is now, read through `sprints()` (there is no single-goal read).
    async fn goal_now(h: &Handlers, id: i64) -> Goal {
        h.sprints(at(1, 0))
            .await
            .unwrap()
            .goals
            .into_iter()
            .find(|g| g.id == id)
            .unwrap()
    }

    fn refused_with<T: std::fmt::Debug>(
        r: Result<T, HandlerError>,
        reason: reeve_protocol::Reason,
    ) -> bool {
        matches!(r, Err(HandlerError::Refused(ref x)) if x == &vec![reason])
    }

    /// Links log under the item; once closed, every write to the sprint and its goals is refused with exactly
    /// `SprintClosed`, each call made with valid arguments (current versions, an existing item never linked before), so
    /// nothing but the closed state can refuse it.
    #[tokio::test]
    async fn a_closed_sprint_is_read_only_and_links_log_under_the_item() {
        let (_c, pool) = super::common::pg().await;
        let (h, sink) = mk(pool.clone());
        let c = ctx(Via::Interactive, 2);
        let g = goal(&h, &c, "g1").await;
        let s = story(&h, &c, "s").await;
        let fresh = story(&h, &c, "t").await;
        h.link_goal(&c, g.id, std::slice::from_ref(s.key()))
            .await
            .unwrap();
        let e = sink.0.lock().unwrap().last().cloned().unwrap();
        assert_eq!(
            (e.entity_type, e.entity_key.as_str(), e.action),
            ("story", s.key().to_string().as_str(), "goal_linked")
        );
        assert_eq!(e.payload["goal_id"], g.id);
        let d = h.sprints(at(1, 0)).await.unwrap().draft.unwrap();
        let a = h.open_sprint(&c, d.id, d.version, None).await.unwrap();
        let g = goal_now(&h, g.id).await;
        h.close_sprint(
            &c,
            a.id,
            a.version,
            vec![(g.id, reeve_protocol::Verdict::Met, None)],
        )
        .await
        .unwrap();
        let g = goal_now(&h, g.id).await;
        let closed = h.sprint(a.id).await.unwrap();
        let bot = ctx(Via::Automation, 18);
        use reeve_protocol::Reason::SprintClosed;
        let k = std::slice::from_ref(fresh.key());
        assert!(
            refused_with(h.link_goal(&c, g.id, k).await, SprintClosed),
            "link"
        );
        assert!(
            refused_with(h.link_goal(&bot, g.id, k).await, SprintClosed),
            "link by automation"
        );
        assert!(
            refused_with(h.unlink_goal(&c, g.id, s.key()).await, SprintClosed),
            "unlink"
        );
        assert!(
            refused_with(h.unlink_goal(&bot, g.id, s.key()).await, SprintClosed),
            "unlink by automation"
        );
        assert!(
            refused_with(
                h.edit_goal(&c, g.id, g.version, Some("x".into()), None, None, None)
                    .await,
                SprintClosed
            ),
            "edit goal"
        );
        assert!(
            refused_with(h.remove_goal(&c, g.id, g.version).await, SprintClosed),
            "remove goal"
        );
        assert!(
            refused_with(
                h.record_confidence(&c, g.id, reeve_protocol::Confidence::OnTrack, None)
                    .await,
                SprintClosed
            ),
            "confidence"
        );
        assert!(
            refused_with(
                h.suggest_confidence(&bot, g.id, reeve_protocol::Confidence::OnTrack, None)
                    .await,
                SprintClosed
            ),
            "suggestion"
        );
        assert!(
            refused_with(
                h.close_sprint(
                    &c,
                    closed.id,
                    closed.version,
                    vec![(g.id, reeve_protocol::Verdict::Missed, None)]
                )
                .await,
                SprintClosed
            ),
            "close again"
        );
        let links: i64 = sqlx::query_scalar("SELECT count(*) FROM goal_links")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(links, 1, "only the link made before the close");
        assert_eq!(goal_now(&h, g.id).await, g, "the goal is unchanged");
    }

    /// Edits log each changed field's old and new value; an edit that changes nothing writes nothing; a stale version
    /// is a conflict; a blank field is refused.
    #[tokio::test]
    async fn editing_a_goal_logs_what_changed() {
        let (_c, pool) = super::common::pg().await;
        let (h, sink) = mk(pool.clone());
        let c = ctx(Via::Interactive, 2);
        let g = goal(&h, &c, "g1").await;
        let e = h
            .edit_goal(
                &c,
                g.id,
                g.version,
                Some(g.outcome.clone()),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(e.version, g.version, "no change, no write");
        let e = h
            .edit_goal(
                &c,
                g.id,
                g.version,
                Some("new outcome".into()),
                None,
                Some("Bob@Example.com".into()),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            (e.outcome.as_str(), e.owner.as_str(), e.version),
            ("new outcome", "bob@example.com", g.version + 1)
        );
        let last = sink.0.lock().unwrap().last().cloned().unwrap();
        assert_eq!((last.entity_type, last.action), ("goal", "edited"));
        assert_eq!(
            last.payload["fields"],
            serde_json::json!(["outcome", "owner"])
        );
        assert_eq!(last.payload["from"]["owner"], "dan@example.com");
        assert!(matches!(
            h.edit_goal(&c, g.id, g.version, Some("again".into()), None, None, None)
                .await,
            Err(HandlerError::Conflict { current }) if current == e.version
        ));
        assert!(matches!(
            h.edit_goal(&c, g.id, e.version, None, Some(" ".into()), None, None)
                .await,
            Err(HandlerError::Validation(_))
        ));
        assert_eq!(sink.0.lock().unwrap().len(), 2, "created and one edit");
    }

    /// Removing a goal ends each of its links in the item's history, then logs the removal; only drafts lose goals.
    #[tokio::test]
    async fn removing_a_goal_unlinks_every_item() {
        let (_c, pool) = super::common::pg().await;
        let (h, sink) = mk(pool.clone());
        let c = ctx(Via::Interactive, 2);
        let g = goal(&h, &c, "g1").await;
        let s = story(&h, &c, "s").await;
        let t = story(&h, &c, "t").await;
        sink.0.lock().unwrap().clear();
        h.link_goal(&c, g.id, &[s.key().clone(), t.key().clone()])
            .await
            .unwrap();
        h.link_goal(&c, g.id, std::slice::from_ref(s.key()))
            .await
            .unwrap();
        assert_eq!(
            sink.0.lock().unwrap().len(),
            2,
            "a repeated link writes nothing"
        );
        // Unlinking what is not linked writes nothing either.
        h.unlink_goal(&c, g.id, s.key()).await.unwrap();
        h.unlink_goal(&c, g.id, s.key()).await.unwrap();
        assert_eq!(sink.0.lock().unwrap().len(), 3);
        h.link_goal(&c, g.id, std::slice::from_ref(s.key()))
            .await
            .unwrap();
        sink.0.lock().unwrap().clear();
        let g = goal_now(&h, g.id).await;
        h.remove_goal(&c, g.id, g.version).await.unwrap();
        let entries: Vec<_> = sink
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|e| (e.entity_type, e.entity_key.clone(), e.action))
            .collect();
        assert_eq!(
            entries,
            vec![
                ("story", s.key().to_string(), "goal_unlinked"),
                ("story", t.key().to_string(), "goal_unlinked"),
                ("goal", g.id.to_string(), "removed"),
            ]
        );
        assert!(h.sprints(at(1, 0)).await.unwrap().goals.is_empty());
        let links: i64 = sqlx::query_scalar("SELECT count(*) FROM goal_links")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(links, 0);

        // An active sprint's goal stays: it is judged at close instead.
        let g = goal(&h, &c, "g2").await;
        let d = h.sprints(at(1, 0)).await.unwrap().draft.unwrap();
        h.open_sprint(&c, d.id, d.version, None).await.unwrap();
        let g = goal_now(&h, g.id).await;
        assert!(refused_with(
            h.remove_goal(&c, g.id, g.version).await,
            reeve_protocol::Reason::SprintNotDraft
        ));
    }

    /// Goals go to the draft by default, or to the active sprint on request; a blank outcome or check is refused, and a
    /// repeated idempotency key answers the first goal without a second entry.
    #[tokio::test]
    async fn adding_goals_to_the_draft_or_the_active_sprint() {
        let (_c, pool) = super::common::pg().await;
        let (h, sink) = mk(pool.clone());
        let c = ctx(Via::Interactive, 2);
        assert!(refused_with(
            h.add_goal(&c, "a", "o", "c", None, "", true).await,
            reeve_protocol::Reason::SprintNotActive
        ));
        assert!(matches!(
            h.add_goal(&c, "b", " ", "c", None, "", false).await,
            Err(HandlerError::Validation(_))
        ));
        let (g, w) = h
            .add_goal(&c, "g1", "o", "c", None, "", false)
            .await
            .unwrap();
        assert!(w.is_empty());
        assert_eq!(g.owner, "dan@example.com");
        let (again, _) = h
            .add_goal(&c, "g1", "o", "c", None, "", false)
            .await
            .unwrap();
        assert_eq!(again.id, g.id);
        assert_eq!(sink.0.lock().unwrap().len(), 1);
        let d = h.sprints(at(1, 0)).await.unwrap().draft.unwrap();
        let a = h.open_sprint(&c, d.id, d.version, None).await.unwrap();
        let (late, _) = h
            .add_goal(
                &c,
                "g2",
                "o2",
                "c2",
                Some("Ann@Example.com".into()),
                "",
                true,
            )
            .await
            .unwrap();
        assert_eq!(
            (late.sprint_id, late.owner.as_str()),
            (a.id, "ann@example.com")
        );
        let v = h.sprints(at(1, 0)).await.unwrap();
        assert_eq!(v.active.unwrap().id, a.id);
        assert!(v.draft.is_some());
        assert!(v.closed.is_empty());
    }

    /// Opening refuses a stale version, an end not after today, and a second active sprint.
    #[tokio::test]
    async fn opening_checks_version_dates_and_the_active_sprint() {
        let (_c, pool) = super::common::pg().await;
        let (h, _) = mk(pool.clone());
        let c = ctx(Via::Interactive, 3);
        goal(&h, &c, "g1").await;
        let d = h.sprints(at(1, 0)).await.unwrap().draft.unwrap();
        assert!(matches!(
            h.open_sprint(&c, d.id, d.version + 1, None).await,
            Err(HandlerError::Conflict { .. })
        ));
        assert!(matches!(
            h.open_sprint(&c, d.id, d.version, Some(at(3, 0).date_naive()))
                .await,
            Err(HandlerError::Validation(_))
        ));
        let a = h
            .open_sprint(&c, d.id, d.version, Some(at(20, 0).date_naive()))
            .await
            .unwrap();
        assert_eq!(
            (a.starts_on, a.ends_on),
            (at(3, 0).date_naive(), at(20, 0).date_naive())
        );
        let d2 = h.sprints(at(1, 0)).await.unwrap().draft.unwrap();
        assert!(refused_with(
            h.open_sprint(&c, d2.id, d2.version, None).await,
            reeve_protocol::Reason::SprintActive
        ));
    }

    #[tokio::test]
    async fn two_concurrent_opens_leave_one_active_and_one_draft() {
        let (_c, pool) = super::common::pg().await;
        let (h, _) = mk(pool.clone());
        let c = ctx(Via::Interactive, 3);
        goal(&h, &c, "g1").await;
        let d = h.sprints(at(1, 0)).await.unwrap().draft.unwrap();
        let (a, b) = tokio::join!(
            h.open_sprint(&c, d.id, d.version, None),
            h.open_sprint(&c, d.id, d.version, None)
        );
        assert_eq!([a.is_ok(), b.is_ok()].iter().filter(|x| **x).count(), 1);
        let n: (i64, i64) = sqlx::query_as("SELECT count(*) FILTER (WHERE state = 'active'), count(*) FILTER (WHERE state = 'draft') FROM sprints").fetch_one(&pool).await.unwrap();
        assert_eq!(n, (1, 1));
    }

    /// Waits until some session in this database waits on a lock; panics after five seconds.
    async fn until_waiting_on_a_lock(pool: &sqlx::PgPool, what: &str) {
        for _ in 0..500 {
            let n: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock'")
                .fetch_one(pool).await.unwrap();
            if n > 0 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("{what} never waited on the sprint row lock");
    }

    /// Deterministic: a closing transaction holds the sprint row and has closed it, uncommitted; the link must wait on
    /// that row and then see the sprint closed. Goes red if `link_goal` reads the sprint without FOR UPDATE (it would
    /// link into the sprint before the close commits and never wait).
    #[tokio::test]
    async fn a_link_waits_for_a_closing_sprint_and_is_refused() {
        let (_c, pool) = super::common::pg().await;
        let (h, _) = mk(pool.clone());
        let c = ctx(Via::Interactive, 3);
        let g = goal(&h, &c, "g1").await;
        let d = h.sprints(at(1, 0)).await.unwrap().draft.unwrap();
        let a = h.open_sprint(&c, d.id, d.version, None).await.unwrap();
        let s = story(&h, &c, "s").await;
        let mut closer = pool.begin().await.unwrap();
        sqlx::query("SELECT id FROM sprints WHERE id = $1 FOR UPDATE")
            .bind(a.id)
            .execute(&mut *closer)
            .await
            .unwrap();
        sqlx::query("UPDATE sprints SET state = 'closed', closed_at = $2 WHERE id = $1")
            .bind(a.id)
            .bind(at(17, 18))
            .execute(&mut *closer)
            .await
            .unwrap();
        let link = tokio::spawn({
            let (h, c, k, gid) = (h.clone(), c.clone(), s.key().clone(), g.id);
            async move { h.link_goal(&c, gid, &[k]).await }
        });
        until_waiting_on_a_lock(&pool, "link_goal").await;
        closer.commit().await.unwrap();
        let r = link.await.unwrap();
        assert!(
            matches!(r, Err(HandlerError::Refused(ref x)) if x == &vec![reeve_protocol::Reason::SprintClosed]),
            "{r:?}"
        );
        let links: i64 = sqlx::query_scalar("SELECT count(*) FROM goal_links")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(links, 0);
    }

    /// Deterministic: a removing transaction holds the sprint row (the lock order is sprint, then goal) and has deleted
    /// the goal, uncommitted; the link waits on the sprint row and then must answer NotFound. Goes red if `link_goal`
    /// trusts its unlocked read of the goal (the insert would hit the goal_links foreign key and fail as a 500).
    #[tokio::test]
    async fn a_link_to_a_goal_removed_meanwhile_is_not_found() {
        let (_c, pool) = super::common::pg().await;
        let (h, _) = mk(pool.clone());
        let c = ctx(Via::Interactive, 3);
        let g = goal(&h, &c, "g1").await;
        let s = story(&h, &c, "s").await;
        let mut remover = pool.begin().await.unwrap();
        sqlx::query("SELECT id FROM sprints WHERE id = $1 FOR UPDATE")
            .bind(g.sprint_id)
            .execute(&mut *remover)
            .await
            .unwrap();
        sqlx::query("DELETE FROM goals WHERE id = $1")
            .bind(g.id)
            .execute(&mut *remover)
            .await
            .unwrap();
        let link = tokio::spawn({
            let (h, c, k, gid) = (h.clone(), c.clone(), s.key().clone(), g.id);
            async move { h.link_goal(&c, gid, &[k]).await }
        });
        until_waiting_on_a_lock(&pool, "link_goal").await;
        remover.commit().await.unwrap();
        let r = link.await.unwrap();
        assert!(matches!(r, Err(HandlerError::NotFound)), "{r:?}");
    }

    /// The same shape for `add_goal`: the draft it read is opened by another transaction before it can lock it. Goes
    /// red if `add_goal` does not recheck the locked row's state (it would add a goal to the now-active sprint as if to
    /// the draft).
    #[tokio::test]
    async fn add_goal_refuses_when_its_draft_was_opened_meanwhile() {
        let (_c, pool) = super::common::pg().await;
        let (h, _) = mk(pool.clone());
        let c = ctx(Via::Interactive, 3);
        goal(&h, &c, "g1").await;
        let d = h.sprints(at(1, 0)).await.unwrap().draft.unwrap();
        let mut opener = pool.begin().await.unwrap();
        sqlx::query("SELECT id FROM sprints WHERE id = $1 FOR UPDATE")
            .bind(d.id)
            .execute(&mut *opener)
            .await
            .unwrap();
        sqlx::query("UPDATE sprints SET state = 'active', opened_at = $2 WHERE id = $1")
            .bind(d.id)
            .bind(at(3, 18))
            .execute(&mut *opener)
            .await
            .unwrap();
        let add = tokio::spawn({
            let (h, c) = (h.clone(), c.clone());
            async move { h.add_goal(&c, "g2", "o", "c", None, "", false).await }
        });
        until_waiting_on_a_lock(&pool, "add_goal").await;
        opener.commit().await.unwrap();
        let r = add.await.unwrap();
        assert!(
            matches!(r, Err(HandlerError::Refused(ref x)) if x == &vec![reeve_protocol::Reason::SprintNotDraft]),
            "{r:?}"
        );
        let goals: i64 = sqlx::query_scalar("SELECT count(*) FROM goals")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(goals, 1);
    }

    /// A write that holds the sprint and goal row locks takes no second connection (ports.rs): run on a pool of one,
    /// any pool read made inside the transaction would wait for the connection the transaction itself holds and time
    /// out. Goes red if `remove_goal` resolves its links' item types through the pool.
    #[tokio::test]
    async fn goal_writes_run_on_a_pool_of_one_connection() {
        let (_c, pool) = super::common::pg().await;
        let (h, _) = mk(pool.clone());
        let c = ctx(Via::Interactive, 2);
        let g = goal(&h, &c, "g1").await;
        let s = story(&h, &c, "s").await;
        let t = story(&h, &c, "t").await;
        let one = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(2))
            .connect_with((*pool.connect_options()).clone())
            .await
            .unwrap();
        let (h1, sink) = mk(one);
        h1.link_goal(&c, g.id, &[s.key().clone(), t.key().clone()])
            .await
            .unwrap();
        h1.unlink_goal(&c, g.id, t.key()).await.unwrap();
        h1.link_goal(&c, g.id, std::slice::from_ref(t.key()))
            .await
            .unwrap();
        let g = goal_now(&h1, g.id).await;
        h1.remove_goal(&c, g.id, g.version).await.unwrap();
        let unlinked: Vec<_> = sink
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.action == "goal_unlinked")
            .map(|e| (e.entity_type, e.entity_key.clone()))
            .collect();
        assert_eq!(
            unlinked,
            vec![
                ("story", t.key().to_string()),
                ("story", s.key().to_string()),
                ("story", t.key().to_string()),
            ]
        );
    }

    /// Records each entry with what the mutation's own transaction sees at that moment: the sprint states and the
    /// number of goals and links. Every value must already be the final one, so each entry was the transaction's last work.
    struct ProbeSink(Mutex<Vec<(&'static str, String)>>);
    #[async_trait]
    impl ActivitySink for ProbeSink {
        async fn record(
            &self,
            tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
            rec: ActivityRecord,
        ) -> Result<(), RepoError> {
            let seen: String = sqlx::query_scalar(
                "SELECT (SELECT string_agg(state, ',' ORDER BY id) FROM sprints) || ' goals=' || \
                 (SELECT count(*) FROM goals) || ' links=' || (SELECT count(*) FROM goal_links) || ' judged=' || \
                 (SELECT count(*) FROM goals WHERE verdict IS NOT NULL)",
            )
            .fetch_one(&mut **tx)
            .await?;
            self.0.lock().unwrap().push((rec.action, seen));
            Ok(())
        }
    }

    /// Activity is each transaction's final work (architecture.md): every entry sees the transaction's writes done.
    #[tokio::test]
    async fn entries_are_recorded_after_every_write() {
        let (_c, pool) = super::common::pg().await;
        let sink = Arc::new(ProbeSink(Mutex::new(vec![])));
        let h = Handlers {
            repo: make_planning_repo(pool.clone()),
            activity: sink.clone(),
            pool: pool.clone(),
            settings: settings(),
        };
        let c = ctx(Via::Interactive, 2);
        let g = goal(&h, &c, "g1").await;
        let gone = goal(&h, &c, "g2").await;
        let s = story(&h, &c, "s").await;
        let t = story(&h, &c, "t").await;
        sink.0.lock().unwrap().clear();
        h.link_goal(&c, g.id, &[s.key().clone(), t.key().clone()])
            .await
            .unwrap();
        h.link_goal(&c, gone.id, &[s.key().clone(), t.key().clone()])
            .await
            .unwrap();
        h.remove_goal(&c, gone.id, gone.version).await.unwrap();
        let d = h.sprints(at(1, 0)).await.unwrap().draft.unwrap();
        let a = h.open_sprint(&c, d.id, d.version, None).await.unwrap();
        h.close_sprint(
            &c,
            a.id,
            a.version,
            vec![(g.id, reeve_protocol::Verdict::Met, None)],
        )
        .await
        .unwrap();
        let seen = sink.0.lock().unwrap().clone();
        let want = |a: &'static str, s: &str| (a, s.to_string());
        assert_eq!(
            seen,
            vec![
                want("goal_linked", "draft goals=2 links=2 judged=0"),
                want("goal_linked", "draft goals=2 links=2 judged=0"),
                want("goal_linked", "draft goals=2 links=4 judged=0"),
                want("goal_linked", "draft goals=2 links=4 judged=0"),
                want("goal_unlinked", "draft goals=1 links=2 judged=0"),
                want("goal_unlinked", "draft goals=1 links=2 judged=0"),
                want("removed", "draft goals=1 links=2 judged=0"),
                want("opened", "active,draft goals=1 links=2 judged=0"),
                want("judged", "closed,draft goals=1 links=2 judged=1"),
                want("closed", "closed,draft goals=1 links=2 judged=1"),
            ]
        );
    }
}

/// The sprint view, story goals, the goal filter and story context.
mod views {
    use super::lifecycle::{goal, mk};
    use super::started_at::{MemSink, at, ctx};
    use reeve_planning::*;
    use reeve_protocol::{DeliveryState, GoalProgressDto, Kind, Resolution, SprintCountsDto, Via};

    fn at_hm(day: u32, h: u32, m: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 11, day, h, m, 0).unwrap()
    }

    /// The counts the sink captured in the sprint's `closed` entry: exactly what a client reads back from
    /// `GET /activity?entity_type=sprint&entity_key=<id>&action=closed`.
    fn at_close(sink: &MemSink, sprint: i64) -> SprintCountsDto {
        let e = sink
            .0
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|e| {
                e.entity_type == "sprint"
                    && e.action == "closed"
                    && e.entity_key == sprint.to_string()
            })
            .cloned()
            .unwrap();
        serde_json::from_value(e.payload["counts"].clone()).unwrap()
    }

    async fn draft(h: &Handlers) -> Sprint {
        h.sprints(at(1, 0)).await.unwrap().draft.unwrap()
    }

    /// kb/ADR-0014 R4: the move's result and activity are the same with and without an active sprint.
    #[tokio::test]
    async fn moves_never_depend_on_goals() {
        let (_c, pool) = super::common::pg().await;
        let (h, sink) = mk(pool.clone());
        let c = ctx(Via::Automation, 3);
        let me = ctx(Via::Interactive, 3);
        let s = h
            .create_story(
                &me,
                "s",
                "S",
                Kind::Chore,
                None,
                "",
                vec!["x".into()],
                vec![],
                None,
                None,
            )
            .await
            .unwrap();
        goal(&h, &me, "g1").await;
        let d = draft(&h).await;
        h.open_sprint(&me, d.id, d.version, None).await.unwrap();
        let before = sink.0.lock().unwrap().len();
        h.move_story(
            &c,
            s.key(),
            s.version(),
            DeliveryState::InProgress,
            None,
            None,
        )
        .await
        .unwrap();
        let new: Vec<_> = sink.0.lock().unwrap()[before..]
            .iter()
            .map(|r| r.action)
            .collect();
        assert_eq!(new, vec!["moved"]);
    }

    #[tokio::test]
    async fn stories_carry_their_active_goals_and_filter_by_them() {
        let (_c, pool) = super::common::pg().await;
        let (h, _) = mk(pool.clone());
        let me = ctx(Via::Interactive, 3);
        let i = h
            .create_initiative(&me, "i", "I", "", "Now", None, None, None)
            .await
            .unwrap();
        let e = h
            .create_epic(&me, "e", "E", Some(i.key().clone()), "", None)
            .await
            .unwrap();
        let a = h
            .create_story(
                &me,
                "a",
                "A",
                Kind::Story,
                Some(e.key().clone()),
                "",
                vec!["x".into()],
                vec![],
                None,
                None,
            )
            .await
            .unwrap();
        let b = h
            .create_story(
                &me,
                "b",
                "B",
                Kind::Chore,
                None,
                "",
                vec!["x".into()],
                vec![],
                None,
                None,
            )
            .await
            .unwrap();
        let g = goal(&h, &me, "g1").await;
        h.link_goal(&me, g.id, &[i.key().clone()]).await.unwrap();
        let list = |f: StoryFilter| {
            let h = h.clone();
            async move {
                h.list_stories(&f, false, reeve_protocol::DoneFilter::All, at(3, 18))
                    .await
                    .unwrap()
            }
        };
        // Draft goals do not resolve: only the active sprint's do.
        let all = list(StoryFilter::default()).await;
        assert!(all.iter().all(|s| s.story.goals.is_empty()));
        let d = draft(&h).await;
        h.open_sprint(&me, d.id, d.version, None).await.unwrap();
        let f = |goal| StoryFilter {
            goal: Some(goal),
            ..Default::default()
        };
        let keys = |v: &[reeve_protocol::StoryWithReadiness]| {
            v.iter().map(|s| s.story.key.clone()).collect::<Vec<_>>()
        };
        let any = list(f(GoalFilter::Any)).await;
        assert_eq!(keys(&any), vec![a.key().clone()]);
        assert_eq!(
            any[0].story.goals[0].via,
            reeve_protocol::GoalVia::Initiative
        );
        assert_eq!(any[0].story.goals[0].id, g.id);
        assert_eq!(
            keys(&list(f(GoalFilter::Id(g.id))).await),
            vec![a.key().clone()]
        );
        assert!(list(f(GoalFilter::Id(g.id + 100))).await.is_empty());
        let none = list(f(GoalFilter::None)).await;
        assert_eq!(keys(&none), vec![b.key().clone()]);
        let ctx_dto = h.story_context(a.key(), chrono::Utc::now()).await.unwrap();
        assert_eq!(ctx_dto.goals.len(), 1);
        assert_eq!(ctx_dto.goals[0].goal.id, g.id);
        assert_eq!(ctx_dto.goals[0].goal.check, "a customer booked one");
        assert_eq!(ctx_dto.goals[0].goal.links, vec![i.key().clone()]);
        assert_eq!(ctx_dto.goals[0].via, reeve_protocol::GoalVia::Initiative);
        assert_eq!(ctx_dto.goals[0].starts_on, at(3, 18).date_naive());
        assert!(
            h.story_context(b.key(), chrono::Utc::now())
                .await
                .unwrap()
                .goals
                .is_empty()
        );
    }

    /// The goal filter before any sprint is open (no story serves a goal), and combined with `ready`, `delivery` and
    /// `epic`: each narrows the rows the other keeps, and never changes what a kept row shows.
    #[tokio::test]
    async fn the_goal_filter_combines_with_every_other_filter() {
        let (_c, pool) = super::common::pg().await;
        let (h, _) = mk(pool.clone());
        let me = ctx(Via::Interactive, 3);
        let i = h
            .create_initiative(&me, "i", "I", "", "Now", None, None, None)
            .await
            .unwrap();
        // The epic's and initiative's discovery are incomplete, so their stories are not ready.
        let e = h
            .create_epic(&me, "e", "E", Some(i.key().clone()), "", None)
            .await
            .unwrap();
        let mk_story = |idem: &'static str, kind, epic: Option<reeve_protocol::Key>| {
            let h = h.clone();
            let me = me.clone();
            async move {
                h.create_story(
                    &me,
                    idem,
                    idem,
                    kind,
                    epic,
                    "",
                    vec!["x".into()],
                    vec![],
                    None,
                    None,
                )
                .await
                .unwrap()
            }
        };
        let a = mk_story("a", Kind::Story, Some(e.key().clone())).await; // serves via the initiative, not ready
        let c = mk_story("c", Kind::Chore, Some(e.key().clone())).await; // serves via the initiative, started below
        let b = mk_story("b", Kind::Chore, None).await; // serves directly, ready
        let u = mk_story("u", Kind::Chore, None).await; // upkeep, ready
        let g = goal(&h, &me, "g1").await;
        h.link_goal(&me, g.id, &[i.key().clone(), b.key().clone()])
            .await
            .unwrap();
        let list = |f: StoryFilter, ready: bool| {
            let h = h.clone();
            async move {
                let mut k: Vec<_> = h
                    .list_stories(&f, ready, reeve_protocol::DoneFilter::All, at(3, 18))
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|s| s.story.key)
                    .collect();
                k.sort_by_key(|k| k.to_string());
                k
            }
        };
        let with = |goal, f: StoryFilter| StoryFilter {
            goal: Some(goal),
            ..f
        };
        let sorted = |mut v: Vec<reeve_protocol::Key>| {
            v.sort_by_key(|k| k.to_string());
            v
        };
        let all = sorted(vec![
            a.key().clone(),
            c.key().clone(),
            b.key().clone(),
            u.key().clone(),
        ]);
        // No active sprint: nothing serves a goal, so `any` is empty and `none` is everything.
        assert!(
            list(with(GoalFilter::Any, StoryFilter::default()), false)
                .await
                .is_empty()
        );
        assert_eq!(
            list(with(GoalFilter::None, StoryFilter::default()), false).await,
            all
        );

        let d = draft(&h).await;
        h.open_sprint(&me, d.id, d.version, None).await.unwrap();
        h.move_story(
            &me,
            c.key(),
            c.version(),
            DeliveryState::InProgress,
            None,
            Some("test".into()),
        )
        .await
        .unwrap();
        // ready: only pickable (ready, todo) stories, then the goal filter.
        assert_eq!(
            list(with(GoalFilter::Any, StoryFilter::default()), true).await,
            vec![b.key().clone()]
        );
        assert_eq!(
            list(with(GoalFilter::None, StoryFilter::default()), true).await,
            vec![u.key().clone()]
        );
        // delivery: the started story still lists under its goal.
        let started = StoryFilter {
            delivery: Some(DeliveryState::InProgress),
            ..Default::default()
        };
        assert_eq!(
            list(with(GoalFilter::Id(g.id), started.clone()), false).await,
            vec![c.key().clone()]
        );
        assert!(
            list(with(GoalFilter::None, started), false)
                .await
                .is_empty()
        );
        // epic: both of the epic's stories serve the goal through its initiative.
        let in_epic = StoryFilter {
            epic: Some(e.key().clone()),
            ..Default::default()
        };
        assert_eq!(
            list(with(GoalFilter::Any, in_epic.clone()), false).await,
            sorted(vec![a.key().clone(), c.key().clone()])
        );
        assert!(
            list(with(GoalFilter::None, in_epic), false)
                .await
                .is_empty()
        );
    }

    /// A story started in N and completed in N+1 counts only in N+1; N shows it in flight at close. Windows are
    /// half-open: a completion exactly at N's close belongs to N+1, one exactly at N's opening to N. Reparenting after
    /// the close changes N's derived counts and leaves its at-close counts as they were.
    #[tokio::test]
    async fn work_counts_in_the_sprint_that_holds_its_completion() {
        let (_c, pool) = super::common::pg().await;
        let (h, sink) = mk(pool.clone());
        let me = |t| Ctx {
            actor: "dan@example.com".into(),
            via: Via::Interactive,
            now: t,
        };
        let mv = |k: reeve_protocol::Key, v: i64, to, res, t| {
            let h = h.clone();
            async move {
                // Readiness is not under test: an interactive override reason gets every start past the gate.
                h.move_story(&me(t), &k, v, to, res, Some("test".into()))
                    .await
                    .unwrap()
            }
        };
        let (n_open, n_close, n1_open) = (at_hm(3, 18, 0), at_hm(17, 18, 0), at_hm(17, 18, 0));
        let e1 = h
            .create_epic(&me(n_open), "e1", "E1", None, "", None)
            .await
            .unwrap();
        let e2 = h
            .create_epic(&me(n_open), "e2", "E2", None, "", None)
            .await
            .unwrap();
        let story = |idem: &'static str| {
            let h = h.clone();
            let e = e1.key().clone();
            async move {
                h.create_story(
                    &me(at_hm(3, 9, 0)),
                    idem,
                    idem,
                    Kind::Chore,
                    Some(e),
                    "",
                    vec!["x".into()],
                    vec![],
                    None,
                    None,
                )
                .await
                .unwrap()
            }
        };
        let (s, t, u) = (story("s").await, story("t").await, story("u").await);
        let gn = h
            .add_goal(
                &me(at_hm(2, 9, 0)),
                "gn",
                "N outcome",
                "N check",
                None,
                "",
                false,
            )
            .await
            .unwrap()
            .0;
        h.link_goal(&me(at_hm(2, 9, 0)), gn.id, &[e1.key().clone()])
            .await
            .unwrap();
        let d = draft(&h).await;
        let n = h
            .open_sprint(&me(n_open), d.id, d.version, None)
            .await
            .unwrap();
        // u: started before N and completed exactly at N's opening: N (from is inclusive).
        let u = mv(
            u.key().clone(),
            u.version(),
            DeliveryState::InProgress,
            None,
            at_hm(3, 10, 0),
        )
        .await;
        mv(
            u.key().clone(),
            u.version(),
            DeliveryState::Done,
            Some(Resolution::Completed),
            n_open,
        )
        .await;
        // s: started in N, unfinished at N's close.
        let s = mv(
            s.key().clone(),
            s.version(),
            DeliveryState::InProgress,
            None,
            at_hm(5, 12, 0),
        )
        .await;
        // t: started in N, completed exactly at N's close: N+1 (to is exclusive).
        let t = mv(
            t.key().clone(),
            t.version(),
            DeliveryState::InProgress,
            None,
            at_hm(6, 12, 0),
        )
        .await;
        let n = h
            .close_sprint(
                &me(n_close),
                n.id,
                n.version,
                vec![(gn.id, reeve_protocol::Verdict::PartlyMet, None)],
            )
            .await
            .unwrap();
        h.add_goal(&me(n1_open), "gm", "M outcome", "M check", None, "", false)
            .await
            .unwrap();
        let d = draft(&h).await;
        let n1 = h
            .open_sprint(&me(n1_open), d.id, d.version, None)
            .await
            .unwrap();
        mv(
            t.key().clone(),
            t.version(),
            DeliveryState::Done,
            Some(Resolution::Completed),
            n_close,
        )
        .await;
        mv(
            s.key().clone(),
            s.version(),
            DeliveryState::Done,
            Some(Resolution::Completed),
            at_hm(20, 12, 0),
        )
        .await;

        let close_n = at_close(&sink, n.id);
        let g = &close_n.goals[0];
        assert_eq!(
            (g.completed, g.in_flight),
            (1, 2),
            "N at close: u completed; s and t in flight"
        );
        let now_n = SprintCountsDto::from(
            &h.sprint_view(n.id, at_hm(20, 13, 0))
                .await
                .unwrap()
                .3
                .unwrap()
                .counts,
        );
        assert_eq!(now_n, close_n, "nothing moved between parents yet");
        let now_n1 = SprintCountsDto::from(
            &h.sprint_view(n1.id, at_hm(20, 13, 0))
                .await
                .unwrap()
                .3
                .unwrap()
                .counts,
        );
        assert_eq!(now_n1.completed, 2, "N+1: t (exactly at its opening) and s");

        // Reparent s to an epic no goal of N covers: N's derived counts change, its at-close counts do not.
        let s_now = h.get_story(s.key()).await.unwrap();
        h.edit_story(
            &me(at_hm(21, 9, 0)),
            s.key(),
            s_now.version(),
            None,
            None,
            None,
            Some(Some(e2.key().clone())),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let after = SprintCountsDto::from(
            &h.sprint_view(n.id, at_hm(21, 10, 0))
                .await
                .unwrap()
                .3
                .unwrap()
                .counts,
        );
        assert_eq!(after.goals[0].in_flight, 1, "now: only t");
        assert_eq!(
            at_close(&sink, n.id),
            close_n,
            "at close is the closed entry, unchanged"
        );
    }

    /// Literal counts over two real sprints, independent of the summary code agreeing with itself: a story counts in
    /// the sprint it completed in, an unlinked completion is upkeep, and an epic link covers the epic's stories.
    #[tokio::test]
    async fn two_sprints_count_each_story_where_it_completed() {
        let (_c, pool) = super::common::pg().await;
        let (h, _) = mk(pool.clone());
        let me = |day, hour| Ctx {
            actor: "dan@example.com".into(),
            via: Via::Interactive,
            now: at(day, hour),
        };
        let e = h
            .create_epic(&me(2, 9), "e", "E", None, "", None)
            .await
            .unwrap();
        let mk_story = |idem: &'static str, epic: Option<reeve_protocol::Key>| {
            let h = h.clone();
            async move {
                h.create_story(
                    &me(2, 9),
                    idem,
                    idem,
                    Kind::Chore,
                    epic,
                    "",
                    vec!["x".into()],
                    vec![],
                    None,
                    None,
                )
                .await
                .unwrap()
            }
        };
        let a = mk_story("a", Some(e.key().clone())).await;
        let b = mk_story("b", None).await;
        let g1 = h
            .add_goal(&me(2, 9), "g1", "o1", "c1", None, "", false)
            .await
            .unwrap()
            .0;
        h.link_goal(&me(2, 9), g1.id, &[e.key().clone()])
            .await
            .unwrap();
        let d = draft(&h).await;
        let s1 = h
            .open_sprint(&me(3, 18), d.id, d.version, None)
            .await
            .unwrap();
        // Readiness is not under test: an interactive override reason gets each start past the gate.
        let a = h
            .move_story(
                &me(5, 12),
                a.key(),
                a.version(),
                DeliveryState::InProgress,
                None,
                Some("test".into()),
            )
            .await
            .unwrap();
        let b = h
            .move_story(
                &me(5, 12),
                b.key(),
                b.version(),
                DeliveryState::InProgress,
                None,
                Some("test".into()),
            )
            .await
            .unwrap();
        h.move_story(
            &me(6, 12),
            b.key(),
            b.version(),
            DeliveryState::Done,
            Some(Resolution::Completed),
            None,
        )
        .await
        .unwrap();
        let s1 = h
            .close_sprint(
                &me(17, 18),
                s1.id,
                s1.version,
                vec![(g1.id, reeve_protocol::Verdict::PartlyMet, None)],
            )
            .await
            .unwrap();
        let g2 = h
            .add_goal(&me(17, 18), "g2", "o2", "c2", None, "", false)
            .await
            .unwrap()
            .0;
        h.link_goal(&me(17, 18), g2.id, &[e.key().clone()])
            .await
            .unwrap();
        let d = draft(&h).await;
        let s2 = h
            .open_sprint(&me(17, 18), d.id, d.version, None)
            .await
            .unwrap();
        h.move_story(
            &me(20, 12),
            a.key(),
            a.version(),
            DeliveryState::Done,
            Some(Resolution::Completed),
            None,
        )
        .await
        .unwrap();

        let one = SprintCountsDto::from(
            &h.sprint_view(s1.id, at(21, 9))
                .await
                .unwrap()
                .3
                .unwrap()
                .counts,
        );
        assert_eq!(
            (one.goals[0].completed, one.goals[0].in_flight),
            (0, 1),
            "A started in sprint 1, completed after it"
        );
        assert_eq!(
            (one.completed, one.upkeep_completed),
            (1, 1),
            "B: completed in sprint 1, unlinked"
        );
        let two = SprintCountsDto::from(
            &h.sprint_view(s2.id, at(21, 9))
                .await
                .unwrap()
                .3
                .unwrap()
                .counts,
        );
        assert_eq!(
            two.goals[0].completed, 1,
            "A counts in sprint 2, where it completed"
        );
        assert_eq!((two.completed, two.upkeep_completed), (1, 0));
    }

    /// The `closed` entry carries the counts at close, computed in the close's transaction over `[opened_at,
    /// closed_at)` (kb/ADR-0014 R6). A literal, not `sprint_view`'s answer: both come from the same computation.
    #[tokio::test]
    async fn the_closed_entry_carries_the_counts_at_close() {
        let (_c, pool) = super::common::pg().await;
        let (h, sink) = mk(pool.clone());
        let c = ctx(Via::Interactive, 2);
        let g = goal(&h, &c, "g1").await;
        let s = super::lifecycle::story(&h, &c, "s").await;
        h.link_goal(&c, g.id, std::slice::from_ref(s.key()))
            .await
            .unwrap();
        let d = draft(&h).await;
        let a = h
            .open_sprint(&ctx(Via::Interactive, 3), d.id, d.version, None)
            .await
            .unwrap();
        let s = h
            .move_story(
                &ctx(Via::Interactive, 5),
                s.key(),
                s.version(),
                DeliveryState::InProgress,
                None,
                Some("test".into()),
            )
            .await
            .unwrap();
        h.move_story(
            &ctx(Via::Interactive, 6),
            s.key(),
            s.version(),
            DeliveryState::Done,
            Some(Resolution::Completed),
            None,
        )
        .await
        .unwrap();
        h.close_sprint(
            &ctx(Via::Interactive, 17),
            a.id,
            a.version,
            vec![(g.id, reeve_protocol::Verdict::Met, None)],
        )
        .await
        .unwrap();
        assert_eq!(
            at_close(&sink, a.id),
            SprintCountsDto {
                goals: vec![GoalProgressDto {
                    goal_id: g.id,
                    completed: 1,
                    ..Default::default()
                }],
                completed: 1,
                upkeep_completed: 0,
            }
        );
    }

    /// The close computes its counts while holding the sprint and goal row locks, so on its own transaction: on a pool
    /// of one connection, a pool read inside it would wait for the connection the transaction holds and time out.
    #[tokio::test]
    async fn a_close_computes_its_counts_on_its_own_transaction() {
        let (_c, pool) = super::common::pg().await;
        let (h, _) = mk(pool.clone());
        let c = ctx(Via::Interactive, 2);
        let g = goal(&h, &c, "g1").await;
        let s = super::lifecycle::story(&h, &c, "s").await;
        h.link_goal(&c, g.id, std::slice::from_ref(s.key()))
            .await
            .unwrap();
        let d = draft(&h).await;
        let a = h
            .open_sprint(&ctx(Via::Interactive, 3), d.id, d.version, None)
            .await
            .unwrap();
        h.move_story(
            &ctx(Via::Interactive, 5),
            s.key(),
            s.version(),
            DeliveryState::InProgress,
            None,
            Some("test".into()),
        )
        .await
        .unwrap();
        let one = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(2))
            .connect_with((*pool.connect_options()).clone())
            .await
            .unwrap();
        let (h1, sink) = mk(one);
        h1.close_sprint(
            &ctx(Via::Interactive, 17),
            a.id,
            a.version,
            vec![(g.id, reeve_protocol::Verdict::Missed, None)],
        )
        .await
        .unwrap();
        assert_eq!(at_close(&sink, a.id).goals[0].in_flight, 1);
    }

    /// The view of a draft has no window: no numbers. The active sprint's numbers come with `sprints()`.
    #[tokio::test]
    async fn a_draft_has_no_numbers_and_the_active_summary_comes_with_the_list() {
        let (_c, pool) = super::common::pg().await;
        let (h, _) = mk(pool.clone());
        let c = ctx(Via::Interactive, 2);
        let g = goal(&h, &c, "g1").await;
        let s = super::lifecycle::story(&h, &c, "s").await;
        h.link_goal(&c, g.id, std::slice::from_ref(s.key()))
            .await
            .unwrap();
        let d = draft(&h).await;
        let (_, goals, links, summary) = h.sprint_view(d.id, at(4, 0)).await.unwrap();
        assert!(summary.is_none());
        assert_eq!(goals.len(), 1);
        assert_eq!(links.len(), 1);
        assert!(h.sprints(at(4, 0)).await.unwrap().active_summary.is_none());
        let a = h
            .open_sprint(&ctx(Via::Interactive, 3), d.id, d.version, None)
            .await
            .unwrap();
        let s = h
            .move_story(
                &ctx(Via::Interactive, 5),
                s.key(),
                s.version(),
                DeliveryState::InProgress,
                None,
                Some("test".into()),
            )
            .await
            .unwrap();
        h.move_story(
            &ctx(Via::Interactive, 6),
            s.key(),
            s.version(),
            DeliveryState::Done,
            Some(Resolution::Completed),
            None,
        )
        .await
        .unwrap();
        let list = h.sprints(at(7, 0)).await.unwrap();
        assert_eq!(list.active.as_ref().map(|x| x.id), Some(a.id));
        let sum = list.active_summary.unwrap();
        assert_eq!(sum.counts.completed, 1);
        assert_eq!(
            sum.demo.iter().map(|d| d.key.clone()).collect::<Vec<_>>(),
            vec![s.key().clone()]
        );
        assert_eq!(sum.demo[0].title, "s");
        // `now` ends the active window: a completion after it is not counted yet.
        assert_eq!(
            h.sprints(at(5, 0))
                .await
                .unwrap()
                .active_summary
                .unwrap()
                .counts
                .completed,
            0
        );
    }

    async fn chore(h: &Handlers, idem: String, epic: Option<reeve_protocol::Key>) -> Story {
        h.create_story(
            &ctx(Via::Interactive, 2),
            &idem,
            &idem,
            Kind::Chore,
            epic,
            "",
            vec!["x".into()],
            vec![],
            None,
            None,
        )
        .await
        .unwrap()
    }

    /// With an active sprint, two goals and links to an initiative and a story, listing 3 or 12 stories sends the same
    /// number of statements: goals resolve in one statement, never per story.
    #[tokio::test]
    async fn list_stories_with_goals_costs_the_same_for_three_and_twelve_stories() {
        let (_c, pool) = super::common::pg_counted().await;
        let (h, _) = mk(pool.clone());
        let me = ctx(Via::Interactive, 2);
        let i = h
            .create_initiative(&me, "i", "I", "", "Now", None, None, None)
            .await
            .unwrap();
        let e = h
            .create_epic(&me, "e", "E", Some(i.key().clone()), "", None)
            .await
            .unwrap();
        let mut keys = Vec::new();
        for n in 0..3 {
            keys.push(
                chore(&h, format!("a{n}"), Some(e.key().clone()))
                    .await
                    .key()
                    .clone(),
            );
        }
        let g1 = goal(&h, &me, "g1").await;
        let g2 = goal(&h, &me, "g2").await;
        h.link_goal(&me, g1.id, &[i.key().clone()]).await.unwrap();
        h.link_goal(&me, g2.id, std::slice::from_ref(&keys[0]))
            .await
            .unwrap();
        let d = draft(&h).await;
        h.open_sprint(&ctx(Via::Interactive, 3), d.id, d.version, None)
            .await
            .unwrap();
        let list = || {
            let h = h.clone();
            async move {
                h.list_stories(
                    &StoryFilter::default(),
                    false,
                    reeve_protocol::DoneFilter::All,
                    at(4, 0),
                )
                .await
                .unwrap();
            }
        };
        list().await; // warm up
        let three = super::common::statements(&pool, list).await;
        for n in 0..9 {
            let epic = (n % 2 == 0).then(|| e.key().clone());
            chore(&h, format!("b{n}"), epic).await;
        }
        let all = h
            .list_stories(
                &StoryFilter::default(),
                false,
                reeve_protocol::DoneFilter::All,
                at(4, 0),
            )
            .await
            .unwrap();
        assert_eq!(all.len(), 12);
        assert_eq!(all.iter().filter(|s| !s.story.goals.is_empty()).count(), 8);
        let twelve = super::common::statements(&pool, list).await;
        assert_eq!(three, twelve);
    }

    /// The sprint view costs the same for 1 goal over 3 stories and 4 goals over 12.
    #[tokio::test]
    async fn sprint_view_costs_the_same_for_one_goal_and_four() {
        let (_c, pool) = super::common::pg_counted().await;
        let (h, _) = mk(pool.clone());
        let me = ctx(Via::Interactive, 2);
        let e = h.create_epic(&me, "e", "E", None, "", None).await.unwrap();
        let g1 = goal(&h, &me, "g1").await;
        h.link_goal(&me, g1.id, &[e.key().clone()]).await.unwrap();
        let mut stories = Vec::new();
        for n in 0..3 {
            stories.push(chore(&h, format!("a{n}"), Some(e.key().clone())).await);
        }
        let d = draft(&h).await;
        let a = h
            .open_sprint(&ctx(Via::Interactive, 3), d.id, d.version, None)
            .await
            .unwrap();
        let start = |s: Story| {
            let h = h.clone();
            async move {
                h.move_story(
                    &ctx(Via::Interactive, 4),
                    s.key(),
                    s.version(),
                    DeliveryState::InProgress,
                    None,
                    Some("test".into()),
                )
                .await
                .unwrap()
            }
        };
        start(stories[0].clone()).await;
        let view = || {
            let h = h.clone();
            async move {
                h.sprint_view(a.id, at(10, 0)).await.unwrap();
            }
        };
        view().await; // warm up
        let one = super::common::statements(&pool, view).await;
        for n in 0..3 {
            let g = h
                .add_goal(
                    &ctx(Via::Interactive, 5),
                    &format!("h{n}"),
                    "o",
                    "c",
                    None,
                    "",
                    true,
                )
                .await
                .unwrap()
                .0;
            let mut keys = Vec::new();
            for m in 0..3 {
                let s = chore(&h, format!("b{n}{m}"), None).await;
                keys.push(s.key().clone());
                if m == 0 {
                    start(s).await;
                }
            }
            h.link_goal(&ctx(Via::Interactive, 5), g.id, &keys)
                .await
                .unwrap();
        }
        let (_, goals, _, sum) = h.sprint_view(a.id, at(10, 0)).await.unwrap();
        assert_eq!(goals.len(), 4);
        assert_eq!(
            sum.unwrap()
                .counts
                .goals
                .iter()
                .map(|g| g.in_flight + g.todo)
                .sum::<u32>(),
            12
        );
        let four = super::common::statements(&pool, view).await;
        assert_eq!(one, four);
    }
}
