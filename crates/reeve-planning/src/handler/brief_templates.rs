//! Brief templates in the handlers: listing, reading, creating, editing, removing and the default mark. Prefill
//! only: nothing here reads or writes an item, and nothing here is read by a move or by readiness (the isolation
//! lint bans these names in `handler/moves.rs`). Who may write is the route's to decide (admins); planning is not
//! told the role.
use super::dto::brief_template_dto;
use super::*;
use crate::domain::brief_template::{BriefTemplate, name_key};
use crate::ports::StoredBriefTemplate;
use reeve_protocol::{BriefTemplateDto, BriefTemplateLevel};
use sqlx::{Postgres, Transaction};

/// The entity every brief template entry is logged on; its key is the template's id, as a goal's is.
const ENTITY: &str = "brief_template";

/// List order: initiative templates first, then by name (as names compare), then by id. Ordered here, not in SQL,
/// so the order does not depend on the database's collation.
fn ordered(mut rows: Vec<StoredBriefTemplate>) -> Vec<StoredBriefTemplate> {
    rows.sort_by_cached_key(|r| (r.template.level(), r.template.name_key(), r.id));
    rows
}

fn by_id(rows: &[StoredBriefTemplate], id: i64) -> Result<&StoredBriefTemplate, HandlerError> {
    rows.iter()
        .find(|r| r.id == id)
        .ok_or(HandlerError::NotFound)
}

/// The template of `level` whose name compares equal to the one `key` was made from.
fn named<'a>(
    rows: &'a [StoredBriefTemplate],
    level: BriefTemplateLevel,
    key: &str,
) -> Option<&'a StoredBriefTemplate> {
    rows.iter()
        .find(|r| r.template.level() == level && r.template.name_key() == key)
}

/// The refusal of a name another template of the level has, on a create and on a rename alike. It is shown as it
/// is to the admin who typed the name, so it names the template that has it and says what to do.
fn taken(there: &StoredBriefTemplate) -> HandlerError {
    HandlerError::Validation(format!(
        "the {} brief template #{} is already named {:?}: edit that one, or choose another name",
        there.template.level().wire(),
        there.id,
        there.template.name()
    ))
}

impl Handlers {
    /// Every brief template, or one level's, in list order, from one read-only snapshot. Every member may read.
    pub async fn brief_templates(
        &self,
        level: Option<BriefTemplateLevel>,
    ) -> Result<Vec<BriefTemplateDto>, HandlerError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = self.brief_template_dtos_in(&mut tx, level).await;
        end_read(tx, out).await
    }

    /// The same list on the caller's transaction: what the export reads on its snapshot. One statement.
    pub(crate) async fn brief_template_dtos_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        level: Option<BriefTemplateLevel>,
    ) -> Result<Vec<BriefTemplateDto>, HandlerError> {
        let rows = self
            .ports
            .brief_templates
            .brief_templates_in(tx, level)
            .await?;
        Ok(ordered(rows).iter().map(brief_template_dto).collect())
    }

    /// One template by id. NotFound when there is none.
    pub async fn brief_template(&self, id: i64) -> Result<BriefTemplateDto, HandlerError> {
        self.brief_templates(None)
            .await?
            .into_iter()
            .find(|t| t.id == id)
            .ok_or(HandlerError::NotFound)
    }

    /// The text a new brief of `level` starts from: with a name, the body of that level's template with that name
    /// (compared as names are), and with none, the body of the level's default. None when there is no such
    /// template. This is all a create ever reads of a template; nothing of it is kept on the item.
    pub async fn brief_template_body(
        &self,
        level: BriefTemplateLevel,
        name: Option<&str>,
    ) -> Result<Option<String>, HandlerError> {
        let key = name.map(name_key);
        Ok(self
            .brief_templates(Some(level))
            .await?
            .into_iter()
            .find(|t| match &key {
                Some(key) => name_key(&t.name) == *key,
                None => t.is_default,
            })
            .map(|t| t.body))
    }

    /// Stores a new template. A bad name or body is a validation error. The create is idempotent by content
    /// (level, name, body), which is stronger than a key for a load from files: one that exists (same level, same
    /// name as names compare, same body) is answered as it is and nothing is written, so loading the same files
    /// again succeeds and changes nothing; the same name with another body is a validation error that points at
    /// the edit. Logged `brief_template_created` with the level and the name, never the body.
    pub async fn create_brief_template(
        &self,
        c: &Ctx,
        level: BriefTemplateLevel,
        name: &str,
        body: &str,
    ) -> Result<BriefTemplateDto, HandlerError> {
        let t = BriefTemplate::new(level, name, body)?;
        let mut tx = self.pool.begin().await?;
        self.ports
            .brief_templates
            .lock_brief_templates(&mut tx)
            .await?;
        let rows = self
            .ports
            .brief_templates
            .brief_templates_in(&mut tx, Some(level))
            .await?;
        if let Some(there) = named(&rows, level, &t.name_key()) {
            if there.template.body() != t.body() {
                return Err(taken(there));
            }
            let dto = brief_template_dto(there);
            tx.rollback().await?;
            return Ok(dto);
        }
        let id = self
            .ports
            .brief_templates
            .insert_brief_template(&mut tx, &t)
            .await?;
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    ENTITY,
                    &id.to_string(),
                    "brief_template_created",
                    serde_json::json!({"level": level.wire(), "name": t.name()}),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(brief_template_dto(&StoredBriefTemplate {
            id,
            template: t,
            is_default: false,
            version: 1,
        }))
    }

    /// Changes a template's name, its body or both, under the version the caller read (Conflict with the current
    /// one otherwise). The level never changes. An edit that changes nothing writes nothing. A name another
    /// template of the level has is a validation error. Logged `brief_template_edited` with the fields that
    /// changed and, on a rename, the old name; never the body. No brief made from the template changes.
    pub async fn edit_brief_template(
        &self,
        c: &Ctx,
        id: i64,
        version: i64,
        name: Option<String>,
        body: Option<String>,
    ) -> Result<BriefTemplateDto, HandlerError> {
        let mut tx = self.pool.begin().await?;
        self.ports
            .brief_templates
            .lock_brief_templates(&mut tx)
            .await?;
        let rows = self
            .ports
            .brief_templates
            .brief_templates_in(&mut tx, None)
            .await?;
        let cur = by_id(&rows, id)?;
        if cur.version != version {
            return Err(HandlerError::Conflict {
                current: cur.version,
            });
        }
        let level = cur.template.level();
        let next = BriefTemplate::new(
            level,
            name.as_deref().unwrap_or(cur.template.name()),
            body.as_deref().unwrap_or(cur.template.body()),
        )?;
        let mut fields = Vec::new();
        if next.name() != cur.template.name() {
            fields.push("name");
        }
        if next.body() != cur.template.body() {
            fields.push("body");
        }
        if fields.is_empty() {
            let dto = brief_template_dto(cur);
            tx.rollback().await?;
            return Ok(dto);
        }
        if let Some(other) = named(&rows, level, &next.name_key())
            && other.id != id
        {
            return Err(taken(other));
        }
        self.ports
            .brief_templates
            .save_brief_template(&mut tx, id, &next)
            .await?;
        let mut payload =
            serde_json::json!({"level": level.wire(), "name": next.name(), "fields": fields});
        if fields.contains(&"name") {
            payload["from"] = cur.template.name().into();
        }
        self.activity
            .record(
                &mut tx,
                self.rec(c, ENTITY, &id.to_string(), "brief_template_edited", payload),
            )
            .await?;
        let dto = brief_template_dto(&StoredBriefTemplate {
            id,
            template: next,
            is_default: cur.is_default,
            version: version + 1,
        });
        tx.commit().await?;
        Ok(dto)
    }

    /// Removes a template. NotFound when there is none with the id. If it was its level's default, the level has
    /// none afterwards. Logged `brief_template_removed`. No brief made from the template changes.
    pub async fn remove_brief_template(&self, c: &Ctx, id: i64) -> Result<(), HandlerError> {
        let mut tx = self.pool.begin().await?;
        self.ports
            .brief_templates
            .lock_brief_templates(&mut tx)
            .await?;
        let rows = self
            .ports
            .brief_templates
            .brief_templates_in(&mut tx, None)
            .await?;
        let cur = by_id(&rows, id)?;
        self.ports
            .brief_templates
            .delete_brief_template(&mut tx, id)
            .await?;
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    ENTITY,
                    &id.to_string(),
                    "brief_template_removed",
                    serde_json::json!({
                        "level": cur.template.level().wire(),
                        "name": cur.template.name(),
                        "was_default": cur.is_default,
                    }),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Makes a template its level's default (`on`), taking the mark from the level's previous default in the same
    /// transaction, or takes the mark off it. A mark that is already as asked writes nothing. Takes no version and
    /// changes none. Logged `brief_template_default_set` (with the id it replaced, or null) or
    /// `brief_template_default_cleared`.
    pub async fn mark_brief_template_default(
        &self,
        c: &Ctx,
        id: i64,
        on: bool,
    ) -> Result<BriefTemplateDto, HandlerError> {
        let mut tx = self.pool.begin().await?;
        self.ports
            .brief_templates
            .lock_brief_templates(&mut tx)
            .await?;
        let rows = self
            .ports
            .brief_templates
            .brief_templates_in(&mut tx, None)
            .await?;
        let cur = by_id(&rows, id)?;
        if cur.is_default == on {
            let dto = brief_template_dto(cur);
            tx.rollback().await?;
            return Ok(dto);
        }
        let level = cur.template.level();
        let about = serde_json::json!({"level": level.wire(), "name": cur.template.name()});
        let (action, payload) = if on {
            let replaced = rows
                .iter()
                .find(|r| r.template.level() == level && r.is_default)
                .map(|r| r.id);
            let mut payload = about;
            payload["replaced"] = serde_json::json!(replaced);
            ("brief_template_default_set", payload)
        } else {
            ("brief_template_default_cleared", about)
        };
        self.ports
            .brief_templates
            .set_default_brief_template(&mut tx, level, on.then_some(id))
            .await?;
        self.activity
            .record(
                &mut tx,
                self.rec(c, ENTITY, &id.to_string(), action, payload),
            )
            .await?;
        let dto = brief_template_dto(&StoredBriefTemplate {
            is_default: on,
            ..cur.clone()
        });
        tx.commit().await?;
        Ok(dto)
    }
}
