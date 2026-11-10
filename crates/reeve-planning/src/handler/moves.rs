use super::*;
use crate::domain::{delivery::*, items::*};
use crate::ports::RepoError;
use reeve_protocol::{DeliveryState, Key, Resolution, TokenKind};

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
        let cur = match self.ports.stories.get_story_for_update(&mut tx, key).await {
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
        if cur.version() != version {
            tx.rollback().await?;
            return Err(HandlerError::Conflict {
                current: cur.version(),
            });
        }
        let readiness = self.readiness_in(&mut tx, key).await?;
        let req = MoveRequest {
            from: cur.delivery(),
            current_resolution: cur.resolution(),
            to,
            ready: readiness.ready,
            resolution,
            override_reason,
            token_kind: if c.via != reeve_protocol::Via::Interactive {
                TokenKind::Automation
            } else {
                TokenKind::Interactive
            },
        };
        let refused = match validate_move(&req) {
            Ok(e) => Ok(e),
            Err(MoveRefused::NotReady) => Err(readiness.reasons.clone()),
            Err(MoveRefused::AutomationCannotOverride) => {
                let mut r = readiness.reasons.clone();
                r.push(reeve_protocol::Reason::AutomationCannotOverride);
                Err(r)
            }
            Err(MoveRefused::OverrideNeedsReason) => {
                Err(vec![reeve_protocol::Reason::OverrideNeedsReason])
            }
            Err(MoveRefused::ResolutionRequired) => {
                Err(vec![reeve_protocol::Reason::ResolutionRequired])
            }
            Err(MoveRefused::CompletedNeedsActiveOrigin) => {
                Err(vec![reeve_protocol::Reason::CompletedNeedsActiveOrigin])
            }
        };
        let effect = match refused {
            Ok(e) => e,
            Err(reasons) => {
                tx.rollback().await?;
                return Err(HandlerError::Refused(reasons));
            }
        };
        if effect == MoveEffect::NoOp {
            tx.rollback().await?;
            return Ok(cur);
        }
        let resolution = match effect {
            MoveEffect::Resolve(r) => Some(r),
            _ => None,
        };
        let owner_email = reeve_protocol::normalise_email(&c.actor);
        let owner_if_none = if to == DeliveryState::InProgress {
            Some(owner_email.as_str())
        } else {
            None
        };
        self.ports
            .stories
            .set_delivery(&mut tx, key, version, to, resolution, owner_if_none, c.now)
            .await?;
        if let MoveEffect::Override { reason } = &effect {
            self.activity
                .record(
                    &mut tx,
                    self.rec(
                        c,
                        "story",
                        &key.to_string(),
                        "override",
                        serde_json::json!({"from": cur.delivery(), "to": to, "reason": reason, "reasons": readiness.reasons}),
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
                    serde_json::json!({"from": cur.delivery(), "to": to, "resolution": resolution}),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(self.ports.stories.get_story(key).await?)
    }
}
