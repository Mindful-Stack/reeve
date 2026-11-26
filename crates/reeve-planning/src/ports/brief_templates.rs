use super::*;
use crate::domain::brief_template::BriefTemplate;
use reeve_protocol::BriefTemplateLevel;

/// A stored brief template: its id, the template, whether it is its level's default, and the version an edit
/// names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBriefTemplate {
    pub id: i64,
    pub template: BriefTemplate,
    pub is_default: bool,
    pub version: i64,
}

/// Brief templates: rows of their own that name no item. No method reads or writes an item row, a version of an
/// item or a readiness fact, nothing here reads the clock, and no statement joins another table. The table holds a
/// handful of rows, so there is one reader, of every row or of one level's; the handlers find a template by id, by
/// name or by its mark in that answer. `handler/moves.rs` may not reach this port (the isolation lint).
#[async_trait]
pub trait BriefTemplateRepo: Send + Sync {
    /// Serialises template writes: a transaction-scoped advisory lock, taken first by every write, so "is this name
    /// free" and "which template is the default" are read and written without a race.
    async fn lock_brief_templates(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<(), RepoError>;
    /// Stores a new template (not the default, version 1) and answers its id. The caller has checked, under the
    /// lock, that the level has no template with this name key.
    async fn insert_brief_template(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        t: &BriefTemplate,
    ) -> Result<i64, RepoError>;
    /// Writes the name, its key and the body of the template `id` and raises its version by one. The level is not
    /// written: it never changes. NotFound when no row has the id.
    async fn save_brief_template(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
        t: &BriefTemplate,
    ) -> Result<(), RepoError>;
    /// Deletes the template `id`. NotFound when no row has the id.
    async fn delete_brief_template(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<(), RepoError>;
    /// Makes `id` the only default of `level`, or with `None` leaves the level without one. No version changes.
    /// NotFound when `id` is not a template of that level.
    async fn set_default_brief_template(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        level: BriefTemplateLevel,
        id: Option<i64>,
    ) -> Result<(), RepoError>;
    /// Every template, or every template of one level, by id. One statement.
    async fn brief_templates_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        level: Option<BriefTemplateLevel>,
    ) -> Result<Vec<StoredBriefTemplate>, RepoError>;
}
