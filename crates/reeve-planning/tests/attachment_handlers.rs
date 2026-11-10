//! Attachment handlers (kb/ADR-0022, kb/ADR-0023): upload, delete, and the claim inside every Markdown-saving write.
//! The sink writes its entries into a real table on the caller's transaction, so a rollback is visible as missing rows,
//! and it can be told to fail on an action to prove the whole write rolls back.
mod common;

mod attachment_handlers {
    use super::common;
    use async_trait::async_trait;
    use bytes::Bytes;
    use chrono::{Duration, TimeZone, Utc};
    use reeve_planning::*;
    use reeve_protocol::{ItemType, Key, Kind, Via};
    use sqlx::PgPool;
    use std::sync::{Arc, Mutex};
    use uuid::Uuid;

    /// Records each entry into `test_activity` on the caller's transaction; fails on `fail_on` once `skip` such
    /// entries have been written.
    struct DbSink {
        fail_on: Mutex<Option<(&'static str, usize)>>,
    }
    #[async_trait]
    impl ActivitySink for DbSink {
        async fn record(
            &self,
            tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
            rec: ActivityRecord,
        ) -> Result<(), RepoError> {
            {
                let mut f = self.fail_on.lock().unwrap();
                if let Some((action, skip)) = f.as_mut()
                    && *action == rec.action
                {
                    if *skip == 0 {
                        return Err(RepoError::NotFound);
                    }
                    *skip -= 1;
                }
            }
            sqlx::query("INSERT INTO test_activity (entity_type, entity_key, action, actor, via, payload) VALUES ($1,$2,$3,$4,$5,$6)")
                .bind(rec.entity_type)
                .bind(&rec.entity_key)
                .bind(rec.action)
                .bind(&rec.actor)
                .bind(format!("{:?}", rec.via))
                .bind(&rec.payload)
                .execute(&mut **tx)
                .await?;
            Ok(())
        }
    }

    struct World {
        _db: reeve_testdb::TestDb,
        pool: PgPool,
        h: Handlers,
        sink: Arc<DbSink>,
    }

    async fn world() -> World {
        let (db, pool) = common::pg().await;
        sqlx::query("CREATE TABLE test_activity (id BIGSERIAL PRIMARY KEY, entity_type TEXT, entity_key TEXT, action TEXT, actor TEXT, via TEXT, payload JSONB)")
            .execute(&pool)
            .await
            .unwrap();
        let sink = Arc::new(DbSink {
            fail_on: Mutex::new(None),
        });
        let h = Handlers {
            ports: make_planning_ports(pool.clone()),
            activity: sink.clone(),
            pool: pool.clone(),
            settings: Settings {
                key_prefix: "ABC".into(),
                horizon_buckets: vec!["Now".into(), "Later".into()],
                repos: vec![],
                done_window_days: 14,
                sprint_length_days: 14,
                upkeep_line: 20,
            },
        };
        World {
            _db: db,
            pool,
            h,
            sink,
        }
    }

    fn ctx(actor: &str) -> Ctx {
        Ctx {
            actor: actor.into(),
            via: Via::Interactive,
            now: Utc.with_ymd_and_hms(2026, 11, 10, 9, 0, 0).unwrap(),
        }
    }

    fn png() -> Bytes {
        let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        Bytes::from(out.into_inner())
    }

    /// The entries written, as (entity_type, entity_key, action) in order.
    async fn entries(pool: &PgPool) -> Vec<(String, String, String)> {
        sqlx::query_as("SELECT entity_type, entity_key, action FROM test_activity ORDER BY id")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    async fn up(w: &World, who: &str) -> Uuid {
        w.h.upload_attachment(&ctx(who), "shot.png", None, png())
            .await
            .unwrap()
            .id()
    }

    fn owner_of(a: &Attachment) -> Option<(ItemType, Key)> {
        match a.owner() {
            AttachmentOwner::Item(t, k) => Some((*t, k.clone())),
            AttachmentOwner::Unclaimed => None,
        }
    }

    async fn owner(w: &World, id: Uuid) -> Option<(ItemType, Key)> {
        owner_of(&w.h.attachment_meta(id, "a@x").await.unwrap())
    }

    #[tokio::test]
    async fn an_upload_is_stripped_stored_and_not_logged() {
        let w = world().await;
        // A PNG with a text chunk: stored without it.
        let mut p = img_parts::png::Png::from_bytes(png()).unwrap();
        p.chunks_mut().insert(
            1,
            img_parts::png::PngChunk::new(*b"tEXt", Bytes::from_static(b"Comment\0secret")),
        );
        let input = p.encoder().bytes();
        let a =
            w.h.upload_attachment(&ctx("a@x"), "../shot.png", None, input.clone())
                .await
                .unwrap();
        let stored = w.h.attachment_chunk(a.id(), 0).await.unwrap().unwrap();
        assert!(stored.len() < input.len(), "the text chunk was stored");
        assert_eq!(a.byte_size(), stored.len() as i64);
        assert_eq!(a.filename(), "shot.png");
        assert_eq!(a.image_type(), ImageType::Png);
        assert!(
            entries(&w.pool).await.is_empty(),
            "an unclaimed upload was logged"
        );
    }

    #[tokio::test]
    async fn uploads_of_other_types_or_too_large_are_refused() {
        let w = world().await;
        let svg = Bytes::from_static(b"<svg xmlns='http://www.w3.org/2000/svg'/>");
        assert!(matches!(
            w.h.upload_attachment(&ctx("a@x"), "x.png", None, svg).await,
            Err(HandlerError::Validation(m)) if m == "only PNG, JPEG and WebP images can be attached"
        ));
        let big = Bytes::from(vec![0u8; MAX_BYTES + 1]);
        assert!(matches!(
            w.h.upload_attachment(&ctx("a@x"), "x.png", None, big).await,
            Err(HandlerError::Validation(m)) if m == "the file is larger than 10 MiB"
        ));
    }

    #[tokio::test]
    async fn a_replayed_upload_returns_the_first_and_only_to_its_uploader() {
        let w = world().await;
        let first =
            w.h.upload_attachment(&ctx("a@x"), "a.png", Some("k"), png())
                .await
                .unwrap();
        let mut other = png().to_vec();
        other.extend_from_slice(b"trailing bytes"); // different bytes, same key
        let again =
            w.h.upload_attachment(&ctx("a@x"), "b.png", Some("k"), Bytes::from(other))
                .await
                .unwrap();
        assert_eq!((again.id(), again.sha256()), (first.id(), first.sha256()));
        assert!(matches!(
            w.h.upload_attachment(&ctx("b@x"), "a.png", Some("k"), png())
                .await,
            Err(HandlerError::NotFound)
        ));
    }

    #[tokio::test]
    async fn an_upload_sweeps_week_old_unclaimed_uploads() {
        let w = world().await;
        let mut old = ctx("a@x");
        old.now -= Duration::days(8);
        let stale =
            w.h.upload_attachment(&old, "s.png", None, png())
                .await
                .unwrap();
        up(&w, "a@x").await;
        assert!(matches!(
            w.h.attachment_meta(stale.id(), "a@x").await,
            Err(HandlerError::NotFound)
        ));
    }

    #[tokio::test]
    async fn every_markdown_write_claims_its_images_and_logs_them_before_itself() {
        let w = world().await;
        let c = ctx("a@x");
        let (a, b) = (up(&w, "a@x").await, up(&w, "a@x").await);
        let ini =
            w.h.create_initiative_attaching(&c, "i", "I", "", "Now", None, None, None, &[a])
                .await
                .unwrap();
        let epic =
            w.h.create_epic_attaching(&c, "e", "E", None, "", None, &[b])
                .await
                .unwrap();
        let s = up(&w, "a@x").await;
        let story =
            w.h.create_story_attaching(
                &c,
                "s",
                "S",
                Kind::Story,
                None,
                "",
                vec![],
                vec![],
                None,
                None,
                &[s],
            )
            .await
            .unwrap();
        assert_eq!(
            owner(&w, a).await,
            Some((ItemType::Initiative, ini.key().clone()))
        );
        assert_eq!(
            owner(&w, b).await,
            Some((ItemType::Epic, epic.key().clone()))
        );
        assert_eq!(
            owner(&w, s).await,
            Some((ItemType::Story, story.key().clone()))
        );
        let log = entries(&w.pool).await;
        let ini_key = ini.key().to_string();
        assert_eq!(
            log[..2],
            [
                (
                    "initiative".into(),
                    ini_key.clone(),
                    "attachment_added".into()
                ),
                ("initiative".into(), ini_key, "created".into())
            ]
        );

        // Edits of each level.
        let (e1, e2, e3) = (
            up(&w, "a@x").await,
            up(&w, "a@x").await,
            up(&w, "a@x").await,
        );
        w.h.edit_initiative_attaching(
            &c,
            ini.key(),
            ini.version(),
            None,
            Some("x".into()),
            None,
            None,
            None,
            None,
            &[e1],
        )
        .await
        .unwrap();
        w.h.edit_epic_attaching(
            &c,
            epic.key(),
            epic.version(),
            None,
            Some("x".into()),
            None,
            None,
            &[e2],
        )
        .await
        .unwrap();
        let edited =
            w.h.edit_story_attaching(
                &c,
                story.key(),
                story.version(),
                None,
                Some("x".into()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                &[e3],
            )
            .await
            .unwrap();
        assert_eq!(
            edited.version(),
            story.version() + 1,
            "an edit bumps once, as it always did"
        );
        assert_eq!(owner(&w, e1).await.unwrap().0, ItemType::Initiative);
        assert_eq!(owner(&w, e2).await.unwrap().0, ItemType::Epic);
        assert_eq!(owner(&w, e3).await.unwrap().0, ItemType::Story);

        // Comments on each level.
        for (item, key) in [
            (ItemType::Initiative, ini.key()),
            (ItemType::Epic, epic.key()),
            (ItemType::Story, story.key()),
        ] {
            let i = up(&w, "a@x").await;
            w.h.comment_item_attaching(&c, item, key, "look", None, &[i])
                .await
                .unwrap();
            assert_eq!(owner(&w, i).await, Some((item, key.clone())));
        }
        let added = entries(&w.pool)
            .await
            .iter()
            .filter(|e| e.2 == "attachment_added")
            .count();
        assert_eq!(added, 9);
    }

    #[tokio::test]
    async fn a_replayed_create_claims_nothing_more() {
        let w = world().await;
        let c = ctx("a@x");
        let first = up(&w, "a@x").await;
        let mk = |ids: Vec<Uuid>| {
            let h = &w.h;
            let c = c.clone();
            async move {
                h.create_story_attaching(
                    &c,
                    "same",
                    "S",
                    Kind::Story,
                    None,
                    "",
                    vec![],
                    vec![],
                    None,
                    None,
                    &ids,
                )
                .await
                .unwrap()
            }
        };
        let s = mk(vec![first]).await;
        let fresh = up(&w, "a@x").await;
        let again = mk(vec![first, fresh]).await;
        assert_eq!(again.key(), s.key());
        assert_eq!(owner(&w, fresh).await, None, "a replay claimed an upload");
        let added = entries(&w.pool)
            .await
            .iter()
            .filter(|e| e.2 == "attachment_added")
            .count();
        assert_eq!(added, 1);
    }

    #[tokio::test]
    async fn a_refused_claim_writes_nothing_at_all() {
        let w = world().await;
        let theirs = up(&w, "b@x").await;
        let r =
            w.h.create_epic_attaching(&ctx("a@x"), "e", "E", None, "", None, &[theirs])
                .await;
        assert!(matches!(&r, Err(HandlerError::Validation(m)) if m.contains("already attached")));
        assert!(
            w.h.ports
                .keys
                .find_by_idempotency(ItemType::Epic, "e")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            owner_of(&w.h.attachment_meta(theirs, "b@x").await.unwrap()),
            None
        );
        assert!(entries(&w.pool).await.is_empty());
    }

    #[tokio::test]
    async fn a_failing_entry_rolls_back_the_item_the_claims_and_the_entries() {
        let w = world().await;
        let (a, b) = (up(&w, "a@x").await, up(&w, "a@x").await);
        // The first attachment_added is written, the second fails: everything of this write must go.
        *w.sink.fail_on.lock().unwrap() = Some(("attachment_added", 1));
        let r =
            w.h.create_story_attaching(
                &ctx("a@x"),
                "s",
                "S",
                Kind::Story,
                None,
                "",
                vec![],
                vec![],
                None,
                None,
                &[a, b],
            )
            .await;
        assert!(r.is_err());
        assert!(
            w.h.ports
                .keys
                .find_by_idempotency(ItemType::Story, "s")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(owner(&w, a).await, None);
        assert_eq!(owner(&w, b).await, None);
        assert!(
            entries(&w.pool).await.is_empty(),
            "an entry outlived its write"
        );
        // The uploads themselves were committed before, and stay.
        assert!(w.h.attachment_chunk(a, 0).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn decision_comments_refuse_images_and_claims_are_capped() {
        let w = world().await;
        let i = up(&w, "a@x").await;
        assert!(matches!(
            w.h.comment_decision_attaching(&ctx("a@x"), 1, "x", None, &[i]).await,
            Err(HandlerError::Validation(m)) if m == "images can't be attached to decisions"
        ));
        let many: Vec<Uuid> = (0..=MAX_CLAIM).map(|_| Uuid::new_v4()).collect();
        assert!(matches!(
            w.h.create_epic_attaching(&ctx("a@x"), "e", "E", None, "", None, &many)
                .await,
            Err(HandlerError::Validation(_))
        ));
    }

    #[tokio::test]
    async fn deletes_log_on_the_owner_only_when_claimed_and_leave_versions() {
        let w = world().await;
        let c = ctx("a@x");
        let story =
            w.h.create_story(
                &c,
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
        let i = up(&w, "a@x").await;
        w.h.comment_item_attaching(&c, ItemType::Story, story.key(), "see", None, &[i])
            .await
            .unwrap();
        // Another member may delete a claimed attachment; it is logged on its story with its filename.
        w.h.delete_attachment(&ctx("b@x"), i).await.unwrap();
        assert!(w.h.attachment_chunk(i, 0).await.unwrap().is_none());
        let log: Vec<(String, serde_json::Value)> = sqlx::query_as(
            "SELECT entity_key, payload FROM test_activity WHERE action = 'attachment_removed'",
        )
        .fetch_all(&w.pool)
        .await
        .unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].0, story.key().to_string());
        assert_eq!(log[0].1["filename"], "shot.png");

        // An unclaimed upload: only its uploader may delete it, and that is not logged.
        let u = up(&w, "a@x").await;
        assert!(matches!(
            w.h.delete_attachment(&ctx("b@x"), u).await,
            Err(HandlerError::NotFound)
        ));
        w.h.delete_attachment(&c, u).await.unwrap();
        let removed = entries(&w.pool)
            .await
            .iter()
            .filter(|e| e.2 == "attachment_removed")
            .count();
        assert_eq!(removed, 1);
        assert_eq!(
            w.h.get_story(story.key()).await.unwrap().version(),
            story.version(),
            "an upload, a claim through a comment or a delete touched the story"
        );
    }

    #[tokio::test]
    async fn only_the_uploader_sees_an_unclaimed_upload_and_lists_hold_claimed_ones() {
        let w = world().await;
        let c = ctx("a@x");
        let i = up(&w, "a@x").await;
        assert!(w.h.attachment_meta(i, "a@x").await.is_ok());
        assert!(matches!(
            w.h.attachment_meta(i, "b@x").await,
            Err(HandlerError::NotFound)
        ));
        let epic =
            w.h.create_epic_attaching(&c, "e", "E", None, "", None, &[i])
                .await
                .unwrap();
        assert!(w.h.attachment_meta(i, "b@x").await.is_ok());
        let listed =
            w.h.list_attachments(ItemType::Epic, epic.key())
                .await
                .unwrap();
        assert_eq!(
            listed.iter().map(Attachment::id).collect::<Vec<_>>(),
            vec![i]
        );
        assert!(matches!(
            w.h.list_attachments(ItemType::Epic, &"ABC-999".parse().unwrap())
                .await,
            Err(HandlerError::NotFound)
        ));
    }
}
