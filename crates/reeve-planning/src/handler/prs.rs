use super::*;
use crate::domain::pull_request::{PrChange, PullRequest};
use reeve_protocol::Key;

impl Handlers {
    /// Records a pull-request event against a story: links, re-links or updates the cached row, or ignores a stale or
    /// repeated event. Never moves the story and never touches its version (kb/ADR-0002 R4); attributed with the
    /// caller's `Via`. One entry per visible change, none when ignored or merely touched (a newer event with the same status, title and url only advances `updated_at`); a re-link also writes `pr_unlinked` on the story the
    /// pull request leaves, naming the story it went to (the new story's `pr_linked` names the one it came from).
    /// `updated_at` is truncated to the microseconds PostgreSQL stores before it is compared, so a redelivered event
    /// with sub-microsecond digits is still recognised as a repeat. The repository is lower-cased first
    /// ([`PullRequest::canonical`]): the row, its lookup and the activity payload all use that spelling.
    pub async fn record_pull_request(
        &self,
        c: &Ctx,
        story: &Key,
        pr: PullRequest,
    ) -> Result<PrChange, HandlerError> {
        let pr = pr.at_storage_precision().canonical();
        self.ports.stories.get_story(story).await?;
        let mut tx = self.pool.begin().await?;
        let change = self
            .ports
            .pull_requests
            .upsert_pull_request(&mut tx, story, &pr)
            .await?;
        let payload = |extra: Option<(&str, &Key)>| {
            let mut p = serde_json::json!({
                "repo": pr.repo,
                "number": pr.number,
                "status": pr.status,
                "url": pr.url,
                "title": pr.title,
            });
            if let Some((field, k)) = extra {
                p[field] = serde_json::Value::String(k.to_string());
            }
            p
        };
        let mut entries = Vec::new();
        match &change {
            PrChange::Linked => entries.push((story.clone(), "pr_linked", payload(None))),
            PrChange::Relinked { from } => {
                entries.push((from.clone(), "pr_unlinked", payload(Some(("to", story)))));
                entries.push((story.clone(), "pr_linked", payload(Some(("from", from)))));
            }
            PrChange::Updated => entries.push((story.clone(), "pr_updated", payload(None))),
            PrChange::Touched | PrChange::Ignored => {}
        }
        for (key, action, payload) in entries {
            self.activity
                .record(
                    &mut tx,
                    self.rec(c, "story", &key.to_string(), action, payload),
                )
                .await?;
        }
        tx.commit().await?;
        Ok(change)
    }
}
