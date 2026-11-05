mod common;
use chrono::{Duration, TimeZone, Utc};
use reeve_activity::{ActivityEntry, ActivityFilter, Via, make_activity_store};

#[tokio::test]
async fn records_inside_the_callers_transaction_and_lists_in_order() {
    let (_c, pool) = common::pg().await;
    let store = make_activity_store(pool.clone());
    let t0 = Utc.with_ymd_and_hms(2026, 10, 26, 9, 0, 0).unwrap();

    // A write inside a transaction that is rolled back must leave no row.
    let mut tx = pool.begin().await.unwrap();
    store
        .append(
            &mut tx,
            ActivityEntry::new(
                t0,
                "dan@example.com",
                Via::Interactive,
                "story",
                "ABC-1",
                "created",
                serde_json::json!({}),
            ),
        )
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        store.list(ActivityFilter::default()).await.unwrap().len(),
        0
    );

    // Two writes committed together are listed oldest first.
    let mut tx = pool.begin().await.unwrap();
    store
        .append(
            &mut tx,
            ActivityEntry::new(
                t0,
                "dan@example.com",
                Via::Interactive,
                "story",
                "ABC-1",
                "created",
                serde_json::json!({}),
            ),
        )
        .await
        .unwrap();
    store
        .append(
            &mut tx,
            ActivityEntry::new(
                t0,
                "bot@example.com",
                Via::Automation,
                "story",
                "ABC-1",
                "override",
                serde_json::json!({"reason":"urgent"}),
            ),
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    // Newest first, so a capped feed shows the latest entries.
    let all = store.list(ActivityFilter::default()).await.unwrap();
    assert_eq!(
        all.iter().map(|e| e.action.as_str()).collect::<Vec<_>>(),
        ["override", "created"]
    );

    // The override filter returns only overrides.
    let overrides = store
        .list(ActivityFilter {
            action: Some("override".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].via, Via::Automation);
    assert_eq!(overrides[0].payload, serde_json::json!({"reason":"urgent"}));

    // `since` is a boundary and `limit` caps the result.
    let mut tx = pool.begin().await.unwrap();
    store
        .append(
            &mut tx,
            ActivityEntry::new(
                t0 + Duration::hours(1),
                "dan@example.com",
                Via::Interactive,
                "story",
                "ABC-1",
                "edited",
                serde_json::json!({}),
            ),
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let recent = store
        .list(ActivityFilter {
            since: Some(t0 + Duration::minutes(30)),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].action, "edited");
    let limited = store
        .list(ActivityFilter {
            limit: Some(1),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(limited.len(), 1);
    assert_eq!(
        limited[0].action, "edited",
        "the cap keeps the newest entry"
    );
}

#[tokio::test]
async fn identity_events_are_recorded_as_user_entries_with_their_via() {
    use reeve_identity::IdentityEvent;
    let (_c, pool) = common::pg().await;
    let store = make_activity_store(pool.clone());
    let at = Utc.with_ymd_and_hms(2026, 10, 26, 9, 0, 0).unwrap();
    let mut tx = pool.begin().await.unwrap();
    reeve_identity::IdentityActivity::record(
        &*store,
        &mut tx,
        IdentityEvent {
            at,
            actor: "operator".into(),
            via: Via::Automation,
            subject: "dan@example.com".into(),
            action: "token_issued",
            payload: serde_json::json!({"token_id": 7}),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let got = store.list(ActivityFilter::default()).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].via, Via::Automation);
    assert_eq!(got[0].entity_type, "user");
    assert_eq!(got[0].entity_key, "dan@example.com");
    assert_eq!(got[0].action, "token_issued");
    assert_eq!(got[0].payload, serde_json::json!({"token_id": 7}));
}

fn entry(action: &str) -> ActivityEntry {
    ActivityEntry::new(
        Utc.with_ymd_and_hms(2026, 10, 26, 9, 0, 0).unwrap(),
        "dan@example.com",
        Via::Interactive,
        "story",
        "ABC-1",
        action,
        serde_json::json!({}),
    )
}

#[tokio::test]
async fn after_id_returns_later_entries_oldest_first_and_max_id_tracks_the_newest() {
    let (_c, pool) = common::pg().await;
    let store = make_activity_store(pool.clone());
    assert_eq!(
        store.max_id().await.unwrap(),
        0,
        "an empty log has max id 0"
    );
    let mut ids = Vec::new();
    for action in ["a", "b", "c"] {
        let mut tx = pool.begin().await.unwrap();
        ids.push(store.append(&mut tx, entry(action)).await.unwrap());
        tx.commit().await.unwrap();
    }
    assert_eq!(store.max_id().await.unwrap(), ids[2]);
    let after_first = store
        .list(ActivityFilter {
            after_id: Some(ids[0]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        after_first
            .iter()
            .map(|e| e.id.unwrap())
            .collect::<Vec<_>>(),
        [ids[1], ids[2]],
        "after_id excludes the cursor and replays ascending"
    );
    let after_last = store
        .list(ActivityFilter {
            after_id: Some(ids[2]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(after_last.is_empty());
}

#[tokio::test]
async fn a_second_writer_waits_for_the_first_to_commit_so_ids_become_visible_in_order() {
    let (_c, pool) = common::pg().await;
    let store = make_activity_store(pool.clone());
    let mut tx_a = pool.begin().await.unwrap();
    let id_a = store.append(&mut tx_a, entry("a")).await.unwrap();

    let mut tx_b = pool.begin().await.unwrap();
    {
        let fut = store.append(&mut tx_b, entry("b"));
        tokio::pin!(fut);
        // Pinned and timed out by reference: the same operation stays alive across the timeout.
        let blocked = tokio::time::timeout(std::time::Duration::from_secs(2), &mut fut).await;
        assert!(
            blocked.is_err(),
            "B's append must wait while A holds the activity lock"
        );
        tx_a.commit().await.unwrap();
        let id_b = tokio::time::timeout(std::time::Duration::from_secs(10), fut)
            .await
            .expect("B proceeds once A commits")
            .unwrap();
        assert!(id_b > id_a);
    }
    tx_b.commit().await.unwrap();
    assert_eq!(
        store.list(ActivityFilter::default()).await.unwrap().len(),
        2
    );
}

#[tokio::test]
async fn a_planning_record_with_via_system_is_stored_and_listed_as_system() {
    use reeve_planning::{ActivityRecord, ActivitySink};
    let (_c, pool) = common::pg().await;
    let store = make_activity_store(pool.clone());
    let at = Utc.with_ymd_and_hms(2026, 10, 26, 9, 0, 0).unwrap();
    let mut tx = pool.begin().await.unwrap();
    ActivitySink::record(
        &*store,
        &mut tx,
        ActivityRecord {
            at,
            actor: "github".into(),
            via: Via::System,
            entity_type: "story",
            entity_key: "ABC-1".into(),
            action: "pr_linked",
            payload: serde_json::json!({}),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let got = store.list(ActivityFilter::default()).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].via, Via::System);
    assert_eq!(got[0].actor, "github");
}
