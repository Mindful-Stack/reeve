//! Stories with a title like the one being written: the Report dialog's and the triage skill's duplicate check.
//! Advice only; it never writes.
use super::*;
use reeve_protocol::SimilarStory;

/// Below this pg_trgm similarity a title is not shown as similar. Tuned against the team's titles (see the repo test).
pub const SIMILAR_MIN: f32 = 0.3;

impl Handlers {
    pub async fn similar_stories(
        &self,
        q: &str,
        limit: Option<i64>,
        now: DateTime<Utc>,
    ) -> Result<Vec<SimilarStory>, HandlerError> {
        let q = q.trim();
        let n = q.chars().count();
        if !(8..=200).contains(&n) {
            return Err(HandlerError::Validation(
                "similar text must be 8..=200 characters".into(),
            ));
        }
        let limit = limit.unwrap_or(5);
        if !(1..=10).contains(&limit) {
            return Err(HandlerError::Validation("limit must be 1..=10".into()));
        }
        // An absurd window (unreachable while config bounds `done_window_days`) reaches back to the epoch, which
        // Postgres can store, rather than before the oldest representable timestamp.
        let since = chrono::Duration::try_days(self.settings.done_window_days.into())
            .and_then(|w| now.checked_sub_signed(w))
            .unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
        Ok(self
            .ports
            .stories
            .similar_stories(q, SIMILAR_MIN, since, limit)
            .await?
            .iter()
            .map(SimilarStory::from)
            .collect())
    }
}
