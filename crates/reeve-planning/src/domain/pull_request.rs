use chrono::{DateTime, SubsecRound, Utc};
use reeve_protocol::{Key, PrStatus};

/// A pull request linked to a story: a cache of GitHub's state keyed by (repo, number). It is informational only;
/// nothing in planning reads its status to move a story (kb/ADR-0002 R4). `repo` is `owner/name` lower-cased (GitHub
/// names are case-insensitive, and its payloads do not always spell a repository the same way): see [`Self::canonical`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub repo: String,
    pub number: i64,
    pub url: String,
    pub title: String,
    pub status: PrStatus,
    pub updated_at: DateTime<Utc>,
}

impl PullRequest {
    /// The event as PostgreSQL stores it: `TIMESTAMPTZ` keeps microseconds, so a sub-microsecond `updated_at` is
    /// truncated before it is compared with the stored row (otherwise a redelivery would look newer than itself).
    pub fn at_storage_precision(mut self) -> Self {
        self.updated_at = self.updated_at.trunc_subsecs(6);
        self
    }

    /// The identity it is stored and looked up by: `repo` lower-cased, so `Mindful-Stack/Reeve` and
    /// `mindful-stack/reeve` are one pull request.
    pub fn canonical(mut self) -> Self {
        self.repo = self.repo.to_ascii_lowercase();
        self
    }
}

/// What recording a pull-request event did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrChange {
    /// First time this pull request was seen; a row now links it to the story.
    Linked,
    /// The pull request was linked to the story `from` and now links to this one.
    Relinked { from: Key },
    /// Same story; the stored status, title or url changed.
    Updated,
    /// Newer than what is stored but nothing visible changed (a push, a label, a review request): the row's
    /// `updated_at` advances, so a late older event cannot overwrite newer state, and no activity entry is written.
    Touched,
    /// Older than what is stored, or identical to it: nothing changed.
    Ignored,
}

/// The upsert rule for an event arriving at an existing row: older events are ignored; an event for the same story whose
/// (status, title, url) match the stored row is ignored at the same instant and `Touched` when newer; anything else
/// overwrites (last write wins on equal times).
pub fn decide(
    stored_story: &Key,
    stored: &PullRequest,
    incoming_story: &Key,
    incoming: &PullRequest,
) -> PrChange {
    if incoming.updated_at < stored.updated_at {
        return PrChange::Ignored;
    }
    let same_story = stored_story == incoming_story;
    if !same_story {
        return PrChange::Relinked {
            from: stored_story.clone(),
        };
    }
    let unchanged = stored.status == incoming.status
        && stored.title == incoming.title
        && stored.url == incoming.url;
    match (unchanged, incoming.updated_at == stored.updated_at) {
        (true, true) => PrChange::Ignored,
        (true, false) => PrChange::Touched,
        (false, _) => PrChange::Updated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn pr(status: PrStatus, secs: i64, title: &str) -> PullRequest {
        PullRequest {
            repo: "o/r".into(),
            number: 1,
            url: "u".into(),
            title: title.into(),
            status,
            updated_at: Utc.timestamp_opt(secs, 0).unwrap(),
        }
    }

    fn k(s: &str) -> Key {
        s.parse().unwrap()
    }

    #[test]
    fn storage_precision_truncates_to_microseconds() {
        let mut p = pr(PrStatus::Open, 10, "t");
        p.updated_at = Utc.timestamp_opt(10, 123_456_789).unwrap();
        assert_eq!(
            p.at_storage_precision().updated_at,
            Utc.timestamp_opt(10, 123_456_000).unwrap()
        );
    }

    #[test]
    fn decide_follows_the_upsert_rules() {
        let (a1, a2) = (&k("A-1"), &k("A-2"));
        let stored = pr(PrStatus::Open, 10, "t");
        assert_eq!(
            decide(a1, &stored, a1, &pr(PrStatus::Merged, 9, "t")),
            PrChange::Ignored
        );
        assert_eq!(
            decide(a1, &stored, a1, &pr(PrStatus::Open, 10, "t")),
            PrChange::Ignored
        );
        assert_eq!(
            decide(a1, &stored, a1, &pr(PrStatus::Open, 10, "t2")),
            PrChange::Updated
        );
        assert_eq!(
            decide(a1, &stored, a1, &pr(PrStatus::Merged, 11, "t")),
            PrChange::Updated
        );
        assert_eq!(
            decide(a1, &stored, a1, &pr(PrStatus::Open, 11, "t")),
            PrChange::Touched,
            "newer but identical: advance the time, write no entry"
        );
        assert_eq!(
            decide(a1, &stored, a1, &pr(PrStatus::Open, 11, "t2")),
            PrChange::Updated
        );
        assert_eq!(
            decide(a1, &stored, a2, &pr(PrStatus::Open, 10, "t")),
            PrChange::Relinked { from: k("A-1") }
        );
        assert_eq!(
            decide(a1, &stored, a2, &pr(PrStatus::Open, 9, "t")),
            PrChange::Ignored
        );
    }
}
