use super::*;
use reeve_protocol::{CommentDto, Key};

impl Handlers {
    /// `idem` is the caller's idempotency key: a repeat returns the first comment's id and writes nothing.
    pub async fn comment_story(
        &self,
        c: &Ctx,
        story: &Key,
        body: &str,
        idem: Option<&str>,
    ) -> Result<i64, HandlerError> {
        self.comment_story_attaching(c, story, body, idem, &[])
            .await
    }

    /// `comment_story`, claiming `attachments` for the story in the comment's transaction (kb/ADR-0022 R2); a
    /// replayed comment claims nothing.
    pub async fn comment_story_attaching(
        &self,
        c: &Ctx,
        story: &Key,
        body: &str,
        idem: Option<&str>,
        attachments: &[Uuid],
    ) -> Result<i64, HandlerError> {
        let attachments = attachment_ids(attachments)?;
        if body.trim().is_empty() {
            return Err(HandlerError::Validation("empty comment".into()));
        }
        self.ports.stories.get_story(story).await?;
        let mut tx = self.pool.begin().await?;
        let (id, inserted) = self
            .ports
            .comments
            .insert_comment(
                &mut tx,
                Some(story),
                None,
                &c.actor,
                body.trim(),
                None,
                idem,
                c.now,
            )
            .await?;
        if !inserted {
            // A replayed key returns the comment it created and logs nothing new.
            tx.rollback().await?;
            return Ok(id);
        }
        self.claim_attachments(&mut tx, c, &attachments, ItemType::Story, story)
            .await?;
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "story",
                    &story.to_string(),
                    "commented",
                    serde_json::json!({"comment_id": id}),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(id)
    }

    /// `comment_decision`, refusing any attachment: a required decision holds no images (kb/ADR-0004).
    pub async fn comment_decision_attaching(
        &self,
        c: &Ctx,
        decision: i64,
        body: &str,
        idem: Option<&str>,
        attachments: &[Uuid],
    ) -> Result<i64, HandlerError> {
        if !attachments.is_empty() {
            return Err(HandlerError::Validation(
                "images can't be attached to decisions".into(),
            ));
        }
        self.comment_decision(c, decision, body, idem).await
    }

    pub async fn comment_decision(
        &self,
        c: &Ctx,
        decision: i64,
        body: &str,
        idem: Option<&str>,
    ) -> Result<i64, HandlerError> {
        if body.trim().is_empty() {
            return Err(HandlerError::Validation("empty comment".into()));
        }
        self.ports.decisions.get_decision(decision).await?;
        let mut tx = self.pool.begin().await?;
        let (id, inserted) = self
            .ports
            .comments
            .insert_comment(
                &mut tx,
                None,
                Some(decision),
                &c.actor,
                body.trim(),
                None,
                idem,
                c.now,
            )
            .await?;
        if !inserted {
            // A replayed key returns the comment it created and logs nothing new.
            tx.rollback().await?;
            return Ok(id);
        }
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "decision",
                    &decision.to_string(),
                    "commented",
                    serde_json::json!({"comment_id": id}),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(id)
    }

    /// A comment on any item by its key: a story's goes through `comment_story`, an initiative's or an epic's
    /// is logged on that item. `idem` as for `comment_story`.
    pub async fn comment_item(
        &self,
        c: &Ctx,
        item: ItemType,
        on: &Key,
        body: &str,
        idem: Option<&str>,
    ) -> Result<i64, HandlerError> {
        self.comment_item_attaching(c, item, on, body, idem, &[])
            .await
    }

    /// `comment_item`, claiming `attachments` for the item in the comment's transaction (kb/ADR-0022 R2).
    pub async fn comment_item_attaching(
        &self,
        c: &Ctx,
        item: ItemType,
        on: &Key,
        body: &str,
        idem: Option<&str>,
        attachments: &[Uuid],
    ) -> Result<i64, HandlerError> {
        let entity = match item {
            ItemType::Story => {
                return self
                    .comment_story_attaching(c, on, body, idem, attachments)
                    .await;
            }
            ItemType::Initiative => {
                self.ports.initiatives.get_initiative(on).await?;
                "initiative"
            }
            ItemType::Epic => {
                self.ports.epics.get_epic(on).await?;
                "epic"
            }
        };
        if body.trim().is_empty() {
            return Err(HandlerError::Validation("empty comment".into()));
        }
        let mut tx = self.pool.begin().await?;
        let (id, inserted) = self
            .ports
            .comments
            .insert_item_comment(&mut tx, item, on, &c.actor, body.trim(), idem, c.now)
            .await?;
        if !inserted {
            tx.rollback().await?;
            return Ok(id);
        }
        let attachments = attachment_ids(attachments)?;
        self.claim_attachments(&mut tx, c, &attachments, item, on)
            .await?;
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    entity,
                    &on.to_string(),
                    "commented",
                    serde_json::json!({"comment_id": id}),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(id)
    }

    pub async fn comments_for_item(
        &self,
        item: ItemType,
        on: &Key,
    ) -> Result<Vec<CommentDto>, HandlerError> {
        Ok(self.ports.comments.list_comments_for_item(item, on).await?)
    }

    pub async fn comments_for_story(&self, story: &Key) -> Result<Vec<CommentDto>, HandlerError> {
        Ok(self.ports.comments.list_comments_for_story(story).await?)
    }

    pub async fn comments_for_decision(&self, id: i64) -> Result<Vec<CommentDto>, HandlerError> {
        Ok(self.ports.comments.list_comments_for_decision(id).await?)
    }
}
