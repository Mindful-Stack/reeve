//! The attachment storage (kb/ADR-0022) against PostgreSQL: chunks, the claim and its classification, the sweep,
//! deletes and idempotency. Everything sits in `mod attachments` so `cargo nextest run … attachments` selects it all.
mod common;

mod attachments {
    use super::common;
    use chrono::{DateTime, Duration, TimeZone, Utc};
    use reeve_planning::Attachment;
    use reeve_planning::{
        AttachmentOwner, AttachmentRepo, CHUNK, ClaimOutcome, ImageType, ItemType, chunks,
        make_planning_ports,
    };
    use reeve_protocol::Key;
    use sqlx::PgPool;
    use std::sync::Arc;
    use uuid::Uuid;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 10, 9, 0, 0).unwrap()
    }
    fn k(s: &str) -> Key {
        s.parse().unwrap()
    }

    /// An initiative GRA-1, an epic GRA-2 and stories GRA-3 and GRA-4, inserted directly.
    async fn items(pool: &PgPool) {
        let t = now();
        sqlx::query("INSERT INTO initiatives (key, title, horizon, created_at, updated_at) VALUES ('GRA-1','I','Now',$1,$1)")
            .bind(t).execute(pool).await.unwrap();
        sqlx::query(
            "INSERT INTO epics (key, title, created_at, updated_at) VALUES ('GRA-2','E',$1,$1)",
        )
        .bind(t)
        .execute(pool)
        .await
        .unwrap();
        for key in ["GRA-3", "GRA-4"] {
            sqlx::query("INSERT INTO stories (key, title, kind, created_at, updated_at) VALUES ($1,'S','story',$2,$2)")
                .bind(key).bind(t).execute(pool).await.unwrap();
        }
    }

    /// Stores `bytes` as an unclaimed upload by `by` at `at`, in chunks; returns its id.
    async fn upload(
        repo: &Arc<dyn AttachmentRepo>,
        pool: &PgPool,
        by: &str,
        at: DateTime<Utc>,
        bytes: &[u8],
        idem: Option<&str>,
    ) -> Uuid {
        let a = Attachment::new(Uuid::new_v4(), by, at, "shot.png", ImageType::Png, bytes);
        let mut tx = pool.begin().await.unwrap();
        assert!(repo.insert_attachment(&mut tx, &a, idem).await.unwrap());
        for (n, c) in chunks(bytes).enumerate() {
            repo.insert_chunk(&mut tx, a.id(), n as i32, c)
                .await
                .unwrap();
        }
        tx.commit().await.unwrap();
        a.id()
    }

    async fn setup() -> (reeve_testdb::TestDb, PgPool, Arc<dyn AttachmentRepo>) {
        let (db, pool) = common::pg().await;
        items(&pool).await;
        let repo = make_planning_ports(pool.clone()).attachments;
        (db, pool, repo)
    }

    async fn claim(
        repo: &Arc<dyn AttachmentRepo>,
        pool: &PgPool,
        ids: &[Uuid],
        actor: &str,
        item: ItemType,
        key: &str,
    ) -> ClaimOutcome {
        let mut tx = pool.begin().await.unwrap();
        let out = repo
            .claim_attachments(&mut tx, ids, actor, item, &k(key))
            .await
            .unwrap();
        tx.commit().await.unwrap();
        out
    }

    #[tokio::test]
    async fn chunks_round_trip_in_order() {
        let (_db, pool, repo) = setup().await;
        let bytes: Vec<u8> = (0..CHUNK * 2 + 5).map(|i| (i % 251) as u8).collect();
        let id = upload(&repo, &pool, "a@x", now(), &bytes, None).await;
        let mut back = Vec::new();
        for n in 0..3 {
            back.extend_from_slice(&repo.attachment_chunk(id, n).await.unwrap().unwrap());
        }
        assert_eq!(back, bytes, "lost or reordered chunks");
        assert!(repo.attachment_chunk(id, 3).await.unwrap().is_none());
        let a = repo.get_attachment(id).await.unwrap();
        assert_eq!(a.byte_size(), bytes.len() as i64);
        assert_eq!(a.owner(), &AttachmentOwner::Unclaimed);
    }

    #[tokio::test]
    async fn a_claim_owns_no_ops_ignores_and_refuses() {
        let (_db, pool, repo) = setup().await;
        let a = upload(&repo, &pool, "a@x", now(), b"a", None).await;
        let b = upload(&repo, &pool, "a@x", now(), b"b", None).await;
        let others = upload(&repo, &pool, "b@x", now(), b"c", None).await;

        // Owned, returned in the order asked.
        assert_eq!(
            claim(&repo, &pool, &[b, a], "a@x", ItemType::Story, "GRA-3").await,
            ClaimOutcome::Claimed(vec![(b, "shot.png".into()), (a, "shot.png".into())])
        );
        assert_eq!(
            repo.get_attachment(a).await.unwrap().owner(),
            &AttachmentOwner::Item(ItemType::Story, k("GRA-3"))
        );
        // Already this item's: a no-op. Gone: ignored.
        assert_eq!(
            claim(
                &repo,
                &pool,
                &[a, Uuid::new_v4()],
                "a@x",
                ItemType::Story,
                "GRA-3"
            )
            .await,
            ClaimOutcome::Claimed(vec![])
        );
        // Another story's, the same key under another item type, and another person's upload: refused.
        assert_eq!(
            claim(&repo, &pool, &[a], "a@x", ItemType::Story, "GRA-4").await,
            ClaimOutcome::Refused
        );
        assert_eq!(
            claim(&repo, &pool, &[others], "a@x", ItemType::Story, "GRA-3").await,
            ClaimOutcome::Refused
        );
        let e = upload(&repo, &pool, "a@x", now(), b"e", None).await;
        claim(&repo, &pool, &[e], "a@x", ItemType::Epic, "GRA-2").await;
        assert_eq!(
            claim(&repo, &pool, &[e], "a@x", ItemType::Story, "GRA-2").await,
            ClaimOutcome::Refused,
            "an epic's attachment claimed for a story"
        );
        assert_eq!(
            repo.get_attachment(others).await.unwrap().owner(),
            &AttachmentOwner::Unclaimed
        );
    }

    #[tokio::test]
    async fn concurrent_claims_of_one_upload_have_one_winner() {
        let (_db, pool, repo) = setup().await;
        let id = upload(&repo, &pool, "a@x", now(), b"x", None).await;
        let mut first = pool.begin().await.unwrap();
        assert_eq!(
            repo.claim_attachments(&mut first, &[id], "a@x", ItemType::Story, &k("GRA-3"))
                .await
                .unwrap(),
            ClaimOutcome::Claimed(vec![(id, "shot.png".into())])
        );
        let (repo2, pool2) = (repo.clone(), pool.clone());
        let second = tokio::spawn(async move {
            let mut tx = pool2.begin().await.unwrap();
            let out = repo2
                .claim_attachments(&mut tx, &[id], "a@x", ItemType::Story, &k("GRA-4"))
                .await
                .unwrap();
            tx.commit().await.unwrap();
            out
        });
        // Wait until the second claim really waits on the row lock the first holds.
        let mut queued = false;
        for _ in 0..250 {
            let n: i64 = sqlx::query_scalar(
                // A row-lock wait is on a transaction id, which pg_locks lists without a database; the waiting
                // backend's own row in pg_stat_activity has one.
                "SELECT count(*) FROM pg_stat_activity WHERE wait_event_type = 'Lock' \
                 AND datname = current_database()",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            if n >= 1 {
                queued = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(queued, "the second claim never waited on the first");
        first.commit().await.unwrap();
        assert_eq!(second.await.unwrap(), ClaimOutcome::Refused);
        assert_eq!(
            repo.get_attachment(id).await.unwrap().owner(),
            &AttachmentOwner::Item(ItemType::Story, k("GRA-3"))
        );
    }

    #[tokio::test]
    async fn the_sweep_takes_only_old_unclaimed_uploads() {
        let (_db, pool, repo) = setup().await;
        let old = upload(&repo, &pool, "a@x", now() - Duration::days(8), b"old", None).await;
        let young = upload(
            &repo,
            &pool,
            "a@x",
            now() - Duration::days(6),
            b"young",
            None,
        )
        .await;
        let kept = upload(
            &repo,
            &pool,
            "a@x",
            now() - Duration::days(8),
            b"kept",
            None,
        )
        .await;
        claim(&repo, &pool, &[kept], "a@x", ItemType::Story, "GRA-3").await;
        let mut tx = pool.begin().await.unwrap();
        let n = repo
            .sweep_unclaimed_attachments(&mut tx, now() - Duration::days(7))
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(n, 1);
        assert!(repo.get_attachment(old).await.is_err());
        assert!(
            repo.attachment_chunk(old, 0).await.unwrap().is_none(),
            "chunks left"
        );
        assert!(repo.get_attachment(young).await.is_ok());
        assert!(repo.get_attachment(kept).await.is_ok());
    }

    #[tokio::test]
    async fn a_delete_returns_the_row_and_takes_its_chunks() {
        let (_db, pool, repo) = setup().await;
        let id = upload(&repo, &pool, "a@x", now(), &vec![7u8; CHUNK + 1], None).await;
        claim(&repo, &pool, &[id], "a@x", ItemType::Epic, "GRA-2").await;
        let mut tx = pool.begin().await.unwrap();
        let gone = repo.delete_attachment(&mut tx, id).await.unwrap().unwrap();
        tx.commit().await.unwrap();
        assert_eq!(
            gone.owner(),
            &AttachmentOwner::Item(ItemType::Epic, k("GRA-2"))
        );
        assert!(repo.attachment_chunk(id, 1).await.unwrap().is_none());
        let mut tx = pool.begin().await.unwrap();
        assert!(repo.delete_attachment(&mut tx, id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn idempotency_keys_are_unique_across_people_and_lists_are_oldest_first() {
        let (_db, pool, repo) = setup().await;
        let first = upload(&repo, &pool, "a@x", now(), b"1", Some("k1")).await;
        let a = Attachment::new(Uuid::new_v4(), "b@x", now(), "x.png", ImageType::Png, b"2");
        let mut tx = pool.begin().await.unwrap();
        assert!(
            !repo
                .insert_attachment(&mut tx, &a, Some("k1"))
                .await
                .unwrap()
        );
        drop(tx);
        assert_eq!(
            repo.find_attachment_by_idempotency("k1")
                .await
                .unwrap()
                .unwrap()
                .id(),
            first
        );
        let later = upload(
            &repo,
            &pool,
            "a@x",
            now() + Duration::minutes(1),
            b"3",
            None,
        )
        .await;
        claim(
            &repo,
            &pool,
            &[later, first],
            "a@x",
            ItemType::Initiative,
            "GRA-1",
        )
        .await;
        let listed: Vec<Uuid> = repo
            .list_attachments_for(ItemType::Initiative, &k("GRA-1"))
            .await
            .unwrap()
            .iter()
            .map(|a| a.id())
            .collect();
        assert_eq!(listed, vec![first, later]);
    }
}
