use super::*;
use crate::domain::promise::{Promisable, Promise, PromiseItem};

/// What the promise action needs from the item it promises, read under a row lock: what it is (a story with its
/// delivery, or an epic) and its current promise.
#[derive(Debug, Clone, PartialEq)]
pub struct PromiseTarget {
    pub item: Promisable,
    pub promise: Option<Promise>,
}

/// A story's promise facts: its own promise, its epic's key and promise (when the epic has one), and its completion.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StoryPromiseRow {
    pub own: Option<Promise>,
    pub epic: Option<(Key, Promise)>,
    pub done_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PromiseFacts {
    pub stories: HashMap<Key, StoryPromiseRow>,
    pub epics: HashMap<Key, Promise>,
}

/// A candidate for a sprint's promise list.
#[derive(Debug, Clone, PartialEq)]
pub struct PromisedStoryRow {
    pub key: Key,
    pub title: String,
    pub delivery: DeliveryState,
    pub resolution: Option<Resolution>,
    pub row: StoryPromiseRow,
}

/// The promise on a story or an epic (kb/ADR-0025): stored on the promised row only, never part of the item, never
/// versioned. The only writer of the promise columns.
#[async_trait]
pub trait PromiseRepo: Send + Sync {
    /// Locks the item's row (`FOR NO KEY UPDATE`: a promise never changes a key, so foreign-key inserts on the row
    /// stay unblocked) and returns what it is and its current promise; None when no item of that kind has the key.
    /// `PromiseItem` has no initiative (R1), so none can be asked for.
    async fn lock_for_promise(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: PromiseItem,
        key: &Key,
    ) -> Result<Option<PromiseTarget>, RepoError>;
    /// Writes the promise, replacing any; touches no other column. Ok(false) when the item does not exist.
    async fn set_promise(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: PromiseItem,
        key: &Key,
        p: &Promise,
    ) -> Result<bool, RepoError>;
    /// Clears the promise; Ok(false) when the item carried none or does not exist. Touches no other column.
    async fn clear_promise(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: PromiseItem,
        key: &Key,
    ) -> Result<bool, RepoError>;
}
