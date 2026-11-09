mod common;
use chrono::{TimeZone, Utc};
use reeve_identity::{AccessKind, IdentityConfig, Role, make_access_store};
use std::time::Duration;

fn floor() -> IdentityConfig {
    IdentityConfig {
        allowed_domains: vec!["example.com".into()],
        allowed_emails: vec![],
        admins: vec!["dan@example.com".into()],
    }
}

#[tokio::test]
async fn writes_in_a_transaction_are_read_back_by_load_in_and_by_load_after_commit() {
    let (_db, pool) = common::pg().await;
    let store = make_access_store(pool.clone());
    let now = Utc.with_ymd_and_hms(2026, 11, 9, 9, 0, 0).unwrap();
    assert!(
        !store
            .load(&floor())
            .await
            .unwrap()
            .is_allowed("g@other.org")
    );
    let mut tx = pool.begin().await.unwrap();
    store
        .insert_entry(
            &mut tx,
            AccessKind::Email,
            "g@other.org",
            "dan@example.com",
            now,
        )
        .await
        .unwrap();
    store
        .insert_entry(
            &mut tx,
            AccessKind::Domain,
            "partner.org",
            "dan@example.com",
            now,
        )
        .await
        .unwrap();
    store
        .insert_admin(&mut tx, "g@other.org", "dan@example.com", now)
        .await
        .unwrap();
    let inside = store.load_in(&mut tx, &floor()).await.unwrap();
    assert!(inside.is_allowed("g@other.org") && inside.is_allowed("x@partner.org"));
    assert_eq!(inside.role_for("g@other.org"), Role::Admin);
    // not visible to another connection before commit
    assert!(
        !store
            .load(&floor())
            .await
            .unwrap()
            .is_allowed("g@other.org")
    );
    tx.commit().await.unwrap();
    let after = store.load(&floor()).await.unwrap();
    assert_eq!(after.role_for("g@other.org"), Role::Admin);
    assert_eq!(after.admin_grants()[0].added_at, now);

    let mut tx = pool.begin().await.unwrap();
    assert!(
        store
            .delete_entry(&mut tx, AccessKind::Email, "g@other.org")
            .await
            .unwrap()
    );
    assert!(
        !store
            .delete_entry(&mut tx, AccessKind::Email, "g@other.org")
            .await
            .unwrap()
    );
    assert!(store.delete_admin(&mut tx, "g@other.org").await.unwrap());
    assert!(!store.delete_admin(&mut tx, "g@other.org").await.unwrap());
    tx.commit().await.unwrap();
    let gone = store.load(&floor()).await.unwrap();
    assert!(!gone.is_allowed("g@other.org") && gone.is_allowed("x@partner.org"));
}

#[tokio::test]
async fn lock_blocks_a_second_writer_until_the_first_transaction_ends() {
    let (_db, pool) = common::pg().await;
    let store = make_access_store(pool.clone());
    let mut first = pool.begin().await.unwrap();
    store.lock(&mut first).await.unwrap();
    let (store2, pool2) = (store.clone(), pool.clone());
    let second = tokio::spawn(async move {
        let mut tx = pool2.begin().await.unwrap();
        store2.lock(&mut tx).await.unwrap();
        tx.commit().await.unwrap();
    });
    // Wait until the second writer is really queued on the advisory lock (pg_locks is server-wide, so scope it).
    let mut conn = pool.acquire().await.unwrap();
    let mut waiting = false;
    for _ in 0..250 {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND NOT granted \
             AND database = (SELECT oid FROM pg_database WHERE datname = current_database())",
        )
        .fetch_one(&mut *conn)
        .await
        .unwrap();
        if n >= 1 {
            waiting = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drop(conn);
    assert!(waiting, "no writer waited on the lock");
    assert!(
        !second.is_finished(),
        "the second lock returned while the first transaction held it"
    );
    first.commit().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), second)
        .await
        .expect("released at commit")
        .unwrap();
}

#[tokio::test]
async fn hand_inserted_values_must_be_normalised() {
    let (_db, pool) = common::pg().await;
    for sql in [
        "INSERT INTO allowed_entries VALUES ('domain','Partner.ORG','x',now())",
        "INSERT INTO allowed_entries VALUES ('email',' g@other.org','x',now())",
        "INSERT INTO allowed_entries VALUES ('email','','x',now())",
        "INSERT INTO admin_grants VALUES ('Eve@gone.org','x',now())",
    ] {
        let err = sqlx::query(sql).execute(&pool).await.expect_err(sql);
        let code = err.as_database_error().and_then(|e| e.code());
        assert_eq!(code.as_deref(), Some("23514"), "{sql}: {err}");
    }
    sqlx::query("INSERT INTO allowed_entries VALUES ('domain','partner.org','x',now())")
        .execute(&pool)
        .await
        .unwrap();
}
