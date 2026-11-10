use super::*;
use crate::domain::items::*;
use crate::ports::RepoError;
use reeve_protocol::{DeliveryState, Key, Reason, Resolution, TokenKind, Via};

impl Handlers {
    /// The gate. Readiness, the two rules, the state change and the activity entry all happen inside one transaction.
    pub async fn move_story(
        &self,
        c: &Ctx,
        key: &Key,
        version: i64,
        to: DeliveryState,
        resolution: Option<Resolution>,
        override_reason: Option<String>,
    ) -> Result<Story, HandlerError> {
        // One snapshot for every readiness fact (discovery marks, decisions, blockers) rather than READ COMMITTED's per-statement
        // snapshots: https://www.postgresql.org/docs/16/transaction-iso.html. The FOR UPDATE below fails with a serialization failure
        // (SQLSTATE 40001) if the story row changed after the snapshot; that is a version conflict.
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ")
            .await?;
        let mut story = match self.ports.stories.get_story_for_update(&mut tx, key).await {
            Ok(s) => s,
            Err(RepoError::Db(e))
                if e.as_database_error().and_then(|d| d.code()).as_deref() == Some("40001") =>
            {
                tx.rollback().await?;
                let current = self.ports.stories.get_story(key).await?.version();
                return Err(HandlerError::Conflict { current });
            }
            Err(e) => return Err(e.into()),
        };
        if story.version() != version {
            tx.rollback().await?;
            return Err(HandlerError::Conflict {
                current: story.version(),
            });
        }
        let readiness = self.readiness_in(&mut tx, key).await?;
        let token = if c.via != Via::Interactive {
            TokenKind::Automation
        } else {
            TokenKind::Interactive
        };
        let actor = reeve_protocol::normalise_email(&c.actor);
        let outcome = story.move_to(
            Move {
                to,
                resolution,
                override_reason,
            },
            readiness.ready,
            token,
            &actor,
        );
        let (from, override_reason) = match outcome {
            Ok(Moved::Unchanged) => {
                tx.rollback().await?;
                return Ok(story);
            }
            Ok(Moved::Moved {
                from,
                override_reason,
            }) => (from, override_reason),
            Err(e) => {
                tx.rollback().await?;
                return Err(HandlerError::Refused(match e {
                    MoveRefused::NotReady => readiness.reasons,
                    MoveRefused::AutomationCannotOverride => {
                        let mut reasons = readiness.reasons;
                        reasons.push(Reason::AutomationCannotOverride);
                        reasons
                    }
                    MoveRefused::OverrideNeedsReason => vec![Reason::OverrideNeedsReason],
                    MoveRefused::ResolutionRequired => vec![Reason::ResolutionRequired],
                    MoveRefused::CompletedNeedsActiveOrigin => {
                        vec![Reason::CompletedNeedsActiveOrigin]
                    }
                }));
            }
        };
        self.ports
            .stories
            .set_delivery(&mut tx, &story, version, c.now)
            .await?;
        // "from" is the state before the move (from Moved), never the story's state after it.
        if let Some(reason) = &override_reason {
            self.activity
                .record(
                    &mut tx,
                    self.rec(
                        c,
                        "story",
                        &key.to_string(),
                        "override",
                        serde_json::json!({"from": from, "to": story.delivery(), "reason": reason, "reasons": readiness.reasons}),
                    ),
                )
                .await?;
        }
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "story",
                    &key.to_string(),
                    "moved",
                    serde_json::json!({"from": from, "to": story.delivery(), "resolution": story.resolution()}),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(self.ports.stories.get_story(key).await?)
    }
}
