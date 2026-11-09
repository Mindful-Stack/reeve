use crate::domain::{
    decision::*,
    items::*,
    pull_request::*,
    readiness::ReadinessInput,
    sprint::{Goal, GoalLink, Sprint, StoryTimes},
    urgency::{Lineage, MarkFact, UrgentMark},
};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use reeve_protocol::{DeliveryState, ItemType, Key, Kind, Resolution};
use sqlx::{Postgres, Transaction};
use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("not found")]
    NotFound,
    #[error("version conflict: current is {current}")]
    Conflict { current: i64 },
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// One item a text search matched, with the fields the match is judged on (`PlanningRepo::search_items`).
#[derive(Debug, Clone)]
pub struct SearchRow {
    pub item: ItemType,
    pub key: Key,
    pub kind: Option<Kind>,
    pub title: String,
    /// An initiative's or an epic's brief, a story's description.
    pub text: String,
    pub external_ref: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct StoryFilter {
    pub epic: Option<Key>,
    pub owner: Option<String>,
    pub delivery: Option<DeliveryState>,
    pub untriaged: bool,
    pub label: Option<String>,
    /// Goal membership, applied after the stories are read (`goal=<id>|any|none`); combines with every other filter.
    pub goal: Option<GoalFilter>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalFilter {
    Id(i64),
    Any,
    None,
}

pub struct StoryCounts {
    pub completed: u32,
    pub dropped: u32,
    pub total: u32,
}

/// One port for the planning context. Mutations take the caller's transaction. Several methods take more than seven
/// arguments by design (the transaction, the key, the version and the fields); the allowance is deliberate and crate-wide for this trait.
#[async_trait]
#[allow(clippy::too_many_arguments)]
pub trait PlanningRepo: Send + Sync {
    async fn next_key(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        prefix: &str,
    ) -> Result<Key, RepoError>;

    async fn find_by_idempotency(
        &self,
        item: ItemType,
        idem: &str,
    ) -> Result<Option<Key>, RepoError>;

    /// Whether an item of this type has this key (whatever its prefix: items issued before a `key_prefix` change keep
    /// their keys).
    async fn key_exists(&self, item: ItemType, key: &Key) -> Result<bool, RepoError>;

    /// The type of the item that has this key, across every type, in one statement (story, epic, initiative; the key
    /// sequence is shared, so at most one has it).
    async fn item_type_of_key(&self, key: &Key) -> Result<Option<ItemType>, RepoError>;

    /// The key of the item whose `external_ref` (a Jira key such as `JPD-77`; UNIQUE per table) is this one.
    async fn find_by_external_ref(
        &self,
        item: ItemType,
        external_ref: &str,
    ) -> Result<Option<Key>, RepoError>;

    /// The same lookup inside the caller's transaction (the import reads it under `lock_external_ref`).
    /// `find_by_idempotency` on the caller's transaction: a check made while it holds a lock takes no second connection.
    /// Initiatives, epics and stories whose title, brief or description, or external reference contains `text`
    /// (case-insensitive, taken literally: `%` and `_` are not wildcards), or whose key is `exact`; at most `limit`,
    /// in tiers: that key, an exact external reference, title matches, the rest; newest first within a tier.
    async fn search_items(
        &self,
        text: &str,
        exact: Option<&Key>,
        limit: i64,
    ) -> Result<Vec<SearchRow>, RepoError>;

    async fn find_by_idempotency_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        idem: &str,
    ) -> Result<Option<Key>, RepoError>;

    async fn find_by_external_ref_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        external_ref: &str,
    ) -> Result<Option<Key>, RepoError>;

    /// Serialises every import of one external reference, whatever the item type, until the transaction ends
    /// (`pg_advisory_xact_lock`). Taken first in the import transaction, before the key sequence and the activity lock.
    async fn lock_external_ref(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        external_ref: &str,
    ) -> Result<(), RepoError>;

    /// Returns false when a row with this idempotency key already exists (nothing inserted). Inserts write every field
    /// of the item, its discovery mark and external reference included (the import creates items in their state).
    async fn insert_initiative(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        i: &Initiative,
        idem: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, RepoError>;
    async fn get_initiative(&self, key: &Key) -> Result<Initiative, RepoError>;
    /// `get_initiative` on the caller's transaction.
    async fn get_initiative_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        key: &Key,
    ) -> Result<Initiative, RepoError>;
    async fn list_initiatives(&self) -> Result<Vec<Initiative>, RepoError>;
    async fn update_initiative(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        i: &Initiative,
        expected_version: i64,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError>;
    async fn story_counts_under_initiative(&self, key: &Key) -> Result<StoryCounts, RepoError>;
    /// Story counts for every initiative that has a story, in one statement; an initiative absent from the map has none.
    async fn story_counts_by_initiative(&self) -> Result<HashMap<Key, StoryCounts>, RepoError>;
    /// `story_counts_under_initiative` on the caller's transaction.
    async fn story_counts_under_initiative_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        key: &Key,
    ) -> Result<StoryCounts, RepoError>;

    async fn insert_epic(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        e: &Epic,
        idem: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, RepoError>;
    async fn get_epic(&self, key: &Key) -> Result<Epic, RepoError>;
    /// `get_epic` on the caller's transaction.
    async fn get_epic_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        key: &Key,
    ) -> Result<Epic, RepoError>;
    async fn list_epics(&self, initiative: Option<&Key>) -> Result<Vec<Epic>, RepoError>;
    async fn update_epic(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        e: &Epic,
        expected_version: i64,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError>;

    async fn insert_story(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        s: &Story,
        idem: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, RepoError>;
    async fn get_story(&self, key: &Key) -> Result<Story, RepoError>;
    async fn get_story_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        key: &Key,
    ) -> Result<Story, RepoError>;
    /// The stories matching `f`, on the caller's transaction: by delivery column, then rank, then key number. With
    /// `done_since`, a done story is kept only if it entered done at or after that instant (one with no `done_at` is
    /// not); stories in other states are unaffected.
    async fn list_stories(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        f: &StoryFilter,
        done_since: Option<DateTime<Utc>>,
    ) -> Result<Vec<Story>, RepoError>;
    async fn update_story(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        s: &Story,
        rank: Option<i32>,
        expected_version: i64,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError>;
    async fn set_delivery(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        key: &Key,
        expected_version: i64,
        to: DeliveryState,
        resolution: Option<Resolution>,
        owner_if_none: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError>;
    /// The todo column in rank order (rank, then shorter key, then key), every row locked FOR UPDATE in key order so
    /// two concurrent re-ranks queue instead of deadlocking.
    async fn todo_column_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<Key>, RepoError>;
    /// Writes every (key, rank) pair in one statement. Ranks are ordering metadata: no version or `updated_at` changes.
    async fn set_ranks(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        ranks: &[(Key, i32)],
    ) -> Result<(), RepoError>;
    /// Bumps one story's version (and `updated_at`) at the expected version; Conflict when it moved on.
    async fn touch_story(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        key: &Key,
        expected_version: i64,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError>;
    async fn add_block(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: &Key,
        blocker: &Key,
    ) -> Result<bool, RepoError>;
    /// Deletes one edge; false when there was none (nothing deleted).
    async fn remove_block(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: &Key,
        blocker: &Key,
    ) -> Result<bool, RepoError>;
    /// The blockers of many stories in one statement on the caller's transaction, each list in key-number order
    /// (`ABC-9` before `ABC-10`); a story without blockers has no entry.
    async fn blockers_of_many(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        stories: &[Key],
    ) -> Result<HashMap<Key, Vec<Key>>, RepoError>;

    /// The stories with these keys, in the order given; a key with no row is skipped. One statement. Callers pass
    /// distinct keys.
    async fn stories_by_keys(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        keys: &[Key],
    ) -> Result<Vec<Story>, RepoError>;

    /// Readiness facts for many stories in a fixed number of statements, whatever their number, on the caller's
    /// transaction. A key with no story row has no entry.
    async fn readiness_inputs(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        stories: &[Key],
    ) -> Result<HashMap<Key, ReadinessInput>, RepoError>;

    /// Returns (decision, inserted). When the idempotency key already exists, returns the existing row and false.
    async fn insert_decision(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        on: ItemType,
        on_key: &Key,
        title: &str,
        decider: &str,
        due: Option<chrono::NaiveDate>,
        reversibility: Option<reeve_protocol::Reversibility>,
        idem: &str,
        now: DateTime<Utc>,
    ) -> Result<(RequiredDecision, bool), RepoError>;
    async fn get_decision(&self, id: i64) -> Result<RequiredDecision, RepoError>;
    async fn get_decision_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<RequiredDecision, RepoError>;
    async fn list_decisions_on(&self, on_key: &Key) -> Result<Vec<RequiredDecision>, RepoError>;
    /// The decisions on an initiative, on its epics and on their stories, closed ones included, oldest first:
    /// each read where it lives, never copied.
    async fn list_decisions_under_initiative(
        &self,
        initiative: &Key,
    ) -> Result<Vec<RequiredDecision>, RepoError>;
    /// `list_decisions_on` on the caller's transaction.
    async fn list_decisions_on_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        on_key: &Key,
    ) -> Result<Vec<RequiredDecision>, RepoError>;
    /// `list_all_decisions(include_closed, false)` on the caller's transaction.
    async fn list_all_decisions_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        include_closed: bool,
    ) -> Result<Vec<RequiredDecision>, RepoError>;
    /// `list_decisions_under_initiative` on the caller's transaction.
    async fn list_decisions_under_initiative_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        initiative: &Key,
    ) -> Result<Vec<RequiredDecision>, RepoError>;
    /// Every decision, oldest first: closed ones only when `include_closed`, and only those carrying a record when
    /// `with_record_only`.
    async fn list_all_decisions(
        &self,
        include_closed: bool,
        with_record_only: bool,
    ) -> Result<Vec<RequiredDecision>, RepoError>;
    /// Writes the row only if `version` still equals `d.version`, then bumps it; Conflict otherwise.
    async fn save_decision(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        d: &RequiredDecision,
    ) -> Result<(), RepoError>;

    /// Sets `checked_at` only, leaving the version as it is: for a check whose result equals the stored one. The caller
    /// holds the row lock (`get_decision_for_update`) and has compared the version.
    async fn touch_decision_checked(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
        at: DateTime<Utc>,
    ) -> Result<(), RepoError>;

    /// `at` is the comment's own date: the caller's `now`, or the original date of an imported comment.
    async fn insert_comment(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: Option<&Key>,
        decision: Option<i64>,
        author: &str,
        body: &str,
        external_ref: Option<&str>,
        idempotency_key: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<(i64, bool), RepoError>;
    async fn list_comments_for_story(
        &self,
        story: &Key,
    ) -> Result<Vec<reeve_protocol::CommentDto>, RepoError>;
    /// `list_comments_for_story` on the caller's transaction.
    async fn list_comments_for_story_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: &Key,
    ) -> Result<Vec<reeve_protocol::CommentDto>, RepoError>;
    async fn list_comments_for_decision(
        &self,
        id: i64,
    ) -> Result<Vec<reeve_protocol::CommentDto>, RepoError>;
    /// A comment on an initiative, an epic or a story, by its key; a repeated idempotency key answers the first
    /// comment's id and false.
    async fn insert_item_comment(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        on: &Key,
        author: &str,
        body: &str,
        idempotency_key: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<(i64, bool), RepoError>;
    async fn list_comments_for_item(
        &self,
        item: ItemType,
        on: &Key,
    ) -> Result<Vec<reeve_protocol::CommentDto>, RepoError>;

    /// The pull requests linked to many stories in one statement on the caller's transaction, each list by repo then
    /// number; a story without pull requests has no entry.
    async fn pull_requests_for_many(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        stories: &[Key],
    ) -> Result<HashMap<Key, Vec<PullRequest>>, RepoError>;
    /// Conflict-safe upsert keyed by (repo, number): `INSERT ... ON CONFLICT DO NOTHING RETURNING` (a returned row
    /// means Linked); otherwise lock the existing row `FOR UPDATE` and decide with `pull_request::decide`, writing
    /// only on Updated/Relinked. Never touches the story row (its version and `updated_at` stay as they were).
    async fn upsert_pull_request(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: &Key,
        pr: &PullRequest,
    ) -> Result<PrChange, RepoError>;
    // The export readers: every row of one table, read on the caller's transaction so the export is one
    // snapshot. Items order by key number, decisions and comments by id, all ascending.
    async fn initiatives_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<Initiative>, RepoError>;
    async fn epics_in(&self, tx: &mut Transaction<'_, Postgres>) -> Result<Vec<Epic>, RepoError>;
    async fn stories_in(&self, tx: &mut Transaction<'_, Postgres>)
    -> Result<Vec<Story>, RepoError>;
    async fn decisions_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<RequiredDecision>, RepoError>;
    async fn comments_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<reeve_protocol::CommentDto>, RepoError>;
    // Sprints, goals and links (kb/ADR-0014). Every mutation takes the caller's transaction and the handler's timestamp.
    /// The sprint in this state, if any (at most one draft and one active exist).
    async fn sprint_in_state(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        state: reeve_protocol::SprintState,
    ) -> Result<Option<Sprint>, RepoError>;
    /// Locks the sprint row; NotFound when absent.
    async fn sprint_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<Sprint, RepoError>;
    async fn get_sprint_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<Sprint, RepoError>;
    /// Every sprint, newest first.
    async fn sprints_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<Sprint>, RepoError>;
    /// Inserts a draft; `Ok(None)` when a draft already exists (the partial unique index; `ON CONFLICT DO NOTHING`).
    async fn insert_draft(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        starts_on: NaiveDate,
        ends_on: NaiveDate,
        now: DateTime<Utc>,
    ) -> Result<Option<Sprint>, RepoError>;
    /// Writes state, dates and timestamps at `s.version`, bumping it; Conflict otherwise. A second active sprint
    /// surfaces as a database unique violation (23505) the handler maps to `sprint_active`.
    async fn save_sprint(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        s: &Sprint,
    ) -> Result<(), RepoError>;
    /// Returns (goal, inserted); a repeated idempotency key answers the first goal and false.
    async fn insert_goal(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        sprint_id: i64,
        outcome: &str,
        check: &str,
        owner: &str,
        notes: &str,
        idem: &str,
        now: DateTime<Utc>,
    ) -> Result<(Goal, bool), RepoError>;
    /// Locks the goal row; NotFound when absent.
    async fn goal_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<Goal, RepoError>;
    async fn get_goal_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<Goal, RepoError>;
    /// Writes every mutable field at `g.version`, bumping it; Conflict otherwise.
    async fn save_goal(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        g: &Goal,
    ) -> Result<(), RepoError>;
    /// Deletes the goal and, by cascade, its links; NotFound when absent.
    async fn delete_goal(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<(), RepoError>;
    /// The goals of these sprints, by sprint then id, in one statement.
    async fn goals_of_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        sprint_ids: &[i64],
    ) -> Result<Vec<Goal>, RepoError>;
    /// The goals with these ids, in one statement.
    async fn goals_by_ids_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        ids: &[i64],
    ) -> Result<Vec<Goal>, RepoError>;
    /// False when the link already existed.
    async fn insert_goal_link(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        goal_id: i64,
        item: &Key,
        by: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, RepoError>;
    /// False when there was no such link.
    async fn delete_goal_link(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        goal_id: i64,
        item: &Key,
    ) -> Result<bool, RepoError>;
    /// The links of these goals, by goal then key number, in one statement.
    async fn links_of_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        goal_ids: &[i64],
    ) -> Result<Vec<GoalLink>, RepoError>;
    /// The item type of each key that names an item (story first, then epic, then initiative, as `item_type_of_key`),
    /// on the caller's transaction in one statement, so a handler holding row locks takes no second connection.
    async fn item_types_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        keys: &[Key],
    ) -> Result<HashMap<Key, ItemType>, RepoError>;
    /// The active sprint's goals each story serves (through itself, its epic or its epic's initiative), in one statement.
    async fn active_goals_of_stories(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        stories: &[Key],
    ) -> Result<HashMap<Key, Vec<(i64, String, reeve_protocol::GoalVia)>>, RepoError>;
    /// The stories a sprint's numbers need, in one statement: covered by `linked` keys (as story, epic or initiative),
    /// or completed in `[from, to)` (the upkeep share's denominator); `started_at` comes along for "in flight".
    async fn sprint_story_times(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        linked: &[Key],
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<StoryTimes>, RepoError>;
    /// Every stored urgent mark (active or expired) with its item's parent and done fact (kb/ADR-0017). One statement.
    async fn urgent_marks_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<MarkFact>, RepoError>;
    /// The lineage `urgency` needs: epics whose key is in `epics` or whose initiative is in `initiatives`, and the
    /// stories under `epics` or under those initiatives' epics. Two statements; none when both inputs are empty.
    async fn urgency_lineage_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        epics: &[Key],
        initiatives: &[Key],
    ) -> Result<Lineage, RepoError>;
    /// Writes the mark on one item, replacing any; touches no other column (not `version`, not `updated_at`).
    /// Ok(false) when no item of that type has the key.
    async fn set_urgent_mark(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        key: &Key,
        mark: &UrgentMark,
    ) -> Result<bool, RepoError>;
    /// Clears the mark; Ok(false) when the item carried none (or does not exist). Touches no other column.
    async fn clear_urgent_mark(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        key: &Key,
    ) -> Result<bool, RepoError>;
}

/// What the planning context needs from the activity log: defined here, implemented by reeve-activity.
#[derive(Debug, Clone)]
pub struct ActivityRecord {
    pub at: DateTime<Utc>,
    pub actor: String,
    pub via: reeve_protocol::Via,
    pub entity_type: &'static str,
    pub entity_key: String,
    pub action: &'static str,
    pub payload: serde_json::Value,
}

#[async_trait]
pub trait ActivitySink: Send + Sync {
    async fn record(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        rec: ActivityRecord,
    ) -> Result<(), RepoError>;
}
