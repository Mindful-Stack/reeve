use crate::{
    domain::{
        decision::*,
        items::*,
        pull_request::*,
        readiness::*,
        sprint::{Goal, GoalLink, Sprint, StoryTimes},
        urgency::{Lineage, MarkFact, UrgentMark},
    },
    ports::*,
    repo_sprint, repo_urgent,
};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use reeve_protocol::{
    ClosureKind, DeliveryState, ItemType, Key, Kind, PrStatus, Resolution, Reversibility, TokenKind,
};
use sqlx::{PgPool, Postgres, Row, Transaction};
use std::{collections::HashMap, sync::Arc};

pub(crate) struct PgPlanningRepo {
    pool: PgPool,
}
impl PgPlanningRepo {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}
pub fn make_planning_repo(pool: PgPool) -> Arc<dyn PlanningRepo> {
    Arc::new(PgPlanningRepo::new(pool))
}

fn kind_str(k: Kind) -> &'static str {
    match k {
        Kind::Story => "story",
        Kind::Bug => "bug",
        Kind::Chore => "chore",
    }
}
pub(crate) fn bad(what: &str, s: &str) -> sqlx::Error {
    sqlx::Error::Decode(format!("unknown {what} {s:?}").into())
}
fn kind_of(s: &str) -> Result<Kind, sqlx::Error> {
    match s {
        "story" => Ok(Kind::Story),
        "bug" => Ok(Kind::Bug),
        "chore" => Ok(Kind::Chore),
        _ => Err(bad("kind", s)),
    }
}
fn delivery_str(d: DeliveryState) -> &'static str {
    match d {
        DeliveryState::Todo => "todo",
        DeliveryState::InProgress => "in_progress",
        DeliveryState::InReview => "in_review",
        DeliveryState::Testing => "testing",
        DeliveryState::Done => "done",
    }
}
pub(crate) fn delivery_of(s: &str) -> Result<DeliveryState, sqlx::Error> {
    match s {
        "todo" => Ok(DeliveryState::Todo),
        "in_progress" => Ok(DeliveryState::InProgress),
        "in_review" => Ok(DeliveryState::InReview),
        "testing" => Ok(DeliveryState::Testing),
        "done" => Ok(DeliveryState::Done),
        _ => Err(bad("delivery state", s)),
    }
}
fn resolution_str(r: Resolution) -> &'static str {
    match r {
        Resolution::Completed => "completed",
        Resolution::Dropped => "dropped",
    }
}
pub(crate) fn resolution_of(s: Option<&str>) -> Result<Option<Resolution>, sqlx::Error> {
    match s {
        None => Ok(None),
        Some("completed") => Ok(Some(Resolution::Completed)),
        Some("dropped") => Ok(Some(Resolution::Dropped)),
        Some(other) => Err(bad("resolution", other)),
    }
}
fn reversibility_str(r: Reversibility) -> &'static str {
    match r {
        Reversibility::OneWay => "one_way",
        Reversibility::TwoWay => "two_way",
    }
}
fn closure_kind_str(k: ClosureKind) -> &'static str {
    match k {
        ClosureKind::Decided => "decided",
        ClosureKind::Withdrawn => "withdrawn",
    }
}
fn token_kind_str(t: TokenKind) -> &'static str {
    match t {
        TokenKind::Interactive => "interactive",
        TokenKind::Automation => "automation",
    }
}
fn reversibility_of(s: Option<&str>) -> Result<Option<Reversibility>, sqlx::Error> {
    match s {
        None => Ok(None),
        Some("one_way") => Ok(Some(Reversibility::OneWay)),
        Some("two_way") => Ok(Some(Reversibility::TwoWay)),
        Some(other) => Err(bad("reversibility", other)),
    }
}
fn closure_kind_of(s: Option<&str>) -> Result<Option<ClosureKind>, sqlx::Error> {
    match s {
        None => Ok(None),
        Some("decided") => Ok(Some(ClosureKind::Decided)),
        Some("withdrawn") => Ok(Some(ClosureKind::Withdrawn)),
        Some(other) => Err(bad("closure kind", other)),
    }
}
fn token_kind_of(s: Option<&str>) -> Result<Option<TokenKind>, sqlx::Error> {
    match s {
        None => Ok(None),
        Some("interactive") => Ok(Some(TokenKind::Interactive)),
        Some("automation") => Ok(Some(TokenKind::Automation)),
        Some(other) => Err(bad("token kind", other)),
    }
}
fn item_type_str(t: ItemType) -> &'static str {
    match t {
        ItemType::Initiative => "initiative",
        ItemType::Epic => "epic",
        ItemType::Story => "story",
    }
}
pub(crate) fn key(s: &str) -> Key {
    s.parse().expect("stored keys are valid")
}
fn mark(at: Option<DateTime<Utc>>, by: Option<String>) -> Option<DiscoveryMark> {
    match (at, by) {
        (Some(at), Some(by)) => Some(DiscoveryMark { at, by }),
        _ => None,
    }
}

fn initiative_from(r: &sqlx::postgres::PgRow) -> Initiative {
    Initiative::from_persisted(
        key(r.get("key")),
        r.get("title"),
        r.get("brief"),
        r.get("horizon"),
        r.get("target_month"),
        r.get("owner"),
        mark(r.get("discovery_at"), r.get("discovery_by")),
        r.get("external_ref"),
        r.get("version"),
    )
}
fn epic_from(r: &sqlx::postgres::PgRow) -> Epic {
    Epic::from_persisted(
        key(r.get("key")),
        r.get("title"),
        r.get::<Option<String>, _>("initiative_key")
            .as_deref()
            .map(key),
        r.get("brief"),
        mark(r.get("discovery_at"), r.get("discovery_by")),
        r.get("external_ref"),
        r.get("version"),
    )
}
fn story_from(r: &sqlx::postgres::PgRow) -> Result<Story, sqlx::Error> {
    let criteria: Vec<reeve_protocol::CriterionDto> =
        serde_json::from_value(r.get::<serde_json::Value, _>("criteria"))
            .map_err(|e| sqlx::Error::Decode(e.into()))?;
    Ok(Story::from_persisted(
        key(r.get("key")),
        r.get("title"),
        kind_of(r.get("kind"))?,
        r.get::<Option<String>, _>("epic_key").as_deref().map(key),
        r.get("description"),
        criteria
            .into_iter()
            .map(|c| Criterion {
                text: c.text,
                done: c.done,
            })
            .collect(),
        r.get("owner"),
        r.get("labels"),
        r.get("rank"),
        delivery_of(r.get("delivery"))?,
        resolution_of(r.get::<Option<String>, _>("resolution").as_deref())?,
        r.get("external_ref"),
        r.get("version"),
    ))
}
fn decision_from(r: &sqlx::postgres::PgRow) -> Result<RequiredDecision, sqlx::Error> {
    let record = match (
        r.get::<Option<String>, _>("record_repo"),
        r.get::<Option<String>, _>("record_path"),
        r.get::<Option<String>, _>("record_id"),
    ) {
        (Some(repo), Some(path), Some(record_id)) => Some(RecordRef {
            repo,
            path,
            record_id,
        }),
        _ => None,
    };
    let closed = match (
        r.get::<Option<DateTime<Utc>>, _>("closed_at"),
        r.get::<Option<String>, _>("closed_by"),
        r.get::<Option<String>, _>("closed_reason"),
    ) {
        (Some(at), Some(by), Some(reason)) => Some(Closed {
            at,
            by,
            reason,
            kind: closure_kind_of(r.get::<Option<String>, _>("closed_kind").as_deref())?,
            via: token_kind_of(r.get::<Option<String>, _>("closed_via").as_deref())?,
        }),
        _ => None,
    };
    let reversibility = reversibility_of(r.get::<Option<String>, _>("reversibility").as_deref())?;
    let on_type: String = r.get("on_type");
    let on_type = match on_type.as_str() {
        "story" => ItemType::Story,
        "epic" => ItemType::Epic,
        "initiative" => ItemType::Initiative,
        other => return Err(bad("item type", other)),
    };
    Ok(RequiredDecision {
        id: r.get("id"),
        on: key(r.get("on_key")),
        on_type,
        title: r.get("title"),
        decider: r.get("decider"),
        due: r.get("due"),
        reversibility,
        record,
        pr_url: r.get("pr_url"),
        check: serde_json::from_value(r.get::<serde_json::Value, _>("check_result"))
            .map_err(|e| sqlx::Error::Decode(e.into()))?,
        checked_at: r.get("checked_at"),
        closed,
        created_at: r.get("created_at"),
        version: r.get("version"),
    })
}
fn criteria_json(c: &[Criterion]) -> serde_json::Value {
    serde_json::to_value(
        c.iter()
            .map(|c| reeve_protocol::CriterionDto {
                text: c.text.clone(),
                done: c.done,
            })
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

const INITIATIVE_COLS: &str = "key, title, brief, horizon, target_month, owner, discovery_at, discovery_by, external_ref, version";
const EPIC_COLS: &str =
    "key, title, initiative_key, brief, discovery_at, discovery_by, external_ref, version";
const STORY_COLS: &str = "key, title, kind, epic_key, description, criteria, owner, labels, rank, delivery, resolution, external_ref, version";
const COMMENT_COLS: &str =
    "id, story_key, decision_id, initiative_key, epic_key, author, body, at, external_ref";
/// Shorter keys first, then by text: under one prefix this is numeric order (`TST-9` before `TST-10`). Keys under
/// different prefixes (after a `key_prefix` change) interleave by length; the order is still deterministic.
const BY_KEY_NUMBER: &str = "length(key), key";
const DECISION_COLS: &str = "id, on_type, on_key, title, decider, due, record_repo, record_path, record_id, pr_url, check_result, checked_at, closed_at, closed_by, closed_reason, created_at, version, reversibility, closed_kind, closed_via";

#[async_trait]
#[allow(clippy::too_many_arguments)]
impl PlanningRepo for PgPlanningRepo {
    async fn next_key(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        prefix: &str,
    ) -> Result<Key, RepoError> {
        let n: i64 = sqlx::query_scalar("INSERT INTO key_sequence (prefix, last) VALUES ($1, 1) ON CONFLICT (prefix) DO UPDATE SET last = key_sequence.last + 1 RETURNING last").bind(prefix).fetch_one(&mut **tx).await?;
        Ok(Key::new(prefix, n as u64).expect("prefix validated by config"))
    }
    async fn find_by_idempotency(
        &self,
        item: ItemType,
        idem: &str,
    ) -> Result<Option<Key>, RepoError> {
        let table = match item {
            ItemType::Initiative => "initiatives",
            ItemType::Epic => "epics",
            ItemType::Story => "stories",
        };
        let k: Option<String> = sqlx::query_scalar(&format!(
            "SELECT key FROM {table} WHERE idempotency_key = $1"
        ))
        .bind(idem)
        .fetch_optional(&self.pool)
        .await?;
        Ok(k.as_deref().map(key))
    }
    async fn search_items(
        &self,
        text: &str,
        exact: Option<&Key>,
        limit: i64,
    ) -> Result<Vec<SearchRow>, RepoError> {
        let mut pattern = String::from("%");
        for ch in text.chars() {
            if matches!(ch, '%' | '_' | '\\') {
                pattern.push('\\');
            }
            pattern.push(ch);
        }
        pattern.push('%');
        let rows = sqlx::query(
            "SELECT * FROM ( \
               SELECT 'initiative' AS t, key, NULL::text AS kind, title, brief AS text, external_ref, created_at FROM initiatives \
               UNION ALL SELECT 'epic', key, NULL, title, brief, external_ref, created_at FROM epics \
               UNION ALL SELECT 'story', key, kind, title, description, external_ref, created_at FROM stories \
             ) x WHERE key = $3 OR title ILIKE $1 ESCAPE '\\' OR text ILIKE $1 ESCAPE '\\' \
               OR external_ref ILIKE $1 ESCAPE '\\' \
             ORDER BY CASE WHEN key = $3 THEN 0 WHEN lower(external_ref) = lower($4) THEN 1 \
               WHEN title ILIKE $1 ESCAPE '\\' THEN 2 ELSE 3 END, created_at DESC, key DESC LIMIT $2",
        )
        .bind(&pattern)
        .bind(limit)
        .bind(exact.map(|k| k.to_string()))
        .bind(text)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                let t: String = r.get("t");
                let item = match t.as_str() {
                    "initiative" => ItemType::Initiative,
                    "epic" => ItemType::Epic,
                    _ => ItemType::Story,
                };
                let kind: Option<String> = r.get("kind");
                Ok(SearchRow {
                    item,
                    key: key(r.get("key")),
                    kind: kind.as_deref().map(kind_of).transpose()?,
                    title: r.get("title"),
                    text: r.get("text"),
                    external_ref: r.get("external_ref"),
                })
            })
            .collect()
    }
    async fn find_by_idempotency_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        idem: &str,
    ) -> Result<Option<Key>, RepoError> {
        let table = match item {
            ItemType::Initiative => "initiatives",
            ItemType::Epic => "epics",
            ItemType::Story => "stories",
        };
        let k: Option<String> = sqlx::query_scalar(&format!(
            "SELECT key FROM {table} WHERE idempotency_key = $1"
        ))
        .bind(idem)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(k.as_deref().map(key))
    }
    async fn key_exists(&self, item: ItemType, k: &Key) -> Result<bool, RepoError> {
        let table = match item {
            ItemType::Initiative => "initiatives",
            ItemType::Epic => "epics",
            ItemType::Story => "stories",
        };
        Ok(sqlx::query_scalar(&format!(
            "SELECT EXISTS (SELECT 1 FROM {table} WHERE key = $1)"
        ))
        .bind(k.to_string())
        .fetch_one(&self.pool)
        .await?)
    }
    async fn item_type_of_key(&self, k: &Key) -> Result<Option<ItemType>, RepoError> {
        let t: Option<String> = sqlx::query_scalar(
            "SELECT t FROM (\
               SELECT 'story' AS t, 0 AS o FROM stories WHERE key = $1 \
               UNION ALL SELECT 'epic', 1 FROM epics WHERE key = $1 \
               UNION ALL SELECT 'initiative', 2 FROM initiatives WHERE key = $1 \
             ) x ORDER BY o LIMIT 1",
        )
        .bind(k.to_string())
        .fetch_optional(&self.pool)
        .await?;
        t.map(|t| match t.as_str() {
            "story" => Ok(ItemType::Story),
            "epic" => Ok(ItemType::Epic),
            "initiative" => Ok(ItemType::Initiative),
            other => Err(bad("item type", other).into()),
        })
        .transpose()
    }
    async fn find_by_external_ref(
        &self,
        item: ItemType,
        external_ref: &str,
    ) -> Result<Option<Key>, RepoError> {
        let table = match item {
            ItemType::Initiative => "initiatives",
            ItemType::Epic => "epics",
            ItemType::Story => "stories",
        };
        let k: Option<String> =
            sqlx::query_scalar(&format!("SELECT key FROM {table} WHERE external_ref = $1"))
                .bind(external_ref)
                .fetch_optional(&self.pool)
                .await?;
        Ok(k.as_deref().map(key))
    }
    async fn find_by_external_ref_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        external_ref: &str,
    ) -> Result<Option<Key>, RepoError> {
        let table = match item {
            ItemType::Initiative => "initiatives",
            ItemType::Epic => "epics",
            ItemType::Story => "stories",
        };
        let k: Option<String> =
            sqlx::query_scalar(&format!("SELECT key FROM {table} WHERE external_ref = $1"))
                .bind(external_ref)
                .fetch_optional(&mut **tx)
                .await?;
        Ok(k.as_deref().map(key))
    }
    async fn lock_external_ref(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        external_ref: &str,
    ) -> Result<(), RepoError> {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('reeve_import:' || $1))")
            .bind(external_ref)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }
    async fn insert_initiative(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        i: &Initiative,
        idem: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, RepoError> {
        let r = sqlx::query("INSERT INTO initiatives (key, title, brief, horizon, target_month, owner, external_ref, discovery_at, discovery_by, version, idempotency_key, created_at, updated_at) VALUES ($1,$2,$3,$4,$5,$6,$9,$10,$11,1,$7,$8,$8) ON CONFLICT (idempotency_key) DO NOTHING")
            .bind(i.key().to_string()).bind(i.title()).bind(i.brief()).bind(i.horizon()).bind(i.target_month()).bind(i.owner()).bind(idem).bind(now).bind(i.external_ref()).bind(i.discovery().map(|d| d.at)).bind(i.discovery().map(|d| d.by.as_str())).execute(&mut **tx).await?;
        Ok(r.rows_affected() == 1)
    }
    async fn get_initiative(&self, k: &Key) -> Result<Initiative, RepoError> {
        get_initiative_on(&self.pool, k).await
    }
    async fn get_initiative_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        k: &Key,
    ) -> Result<Initiative, RepoError> {
        get_initiative_on(&mut **tx, k).await
    }
    async fn list_initiatives(&self) -> Result<Vec<Initiative>, RepoError> {
        Ok(sqlx::query(&format!(
            "SELECT {INITIATIVE_COLS} FROM initiatives ORDER BY created_at, {BY_KEY_NUMBER}"
        ))
        .fetch_all(&self.pool)
        .await?
        .iter()
        .map(initiative_from)
        .collect())
    }
    async fn update_initiative(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        i: &Initiative,
        expected: i64,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError> {
        let res = sqlx::query("UPDATE initiatives SET title=$2, brief=$3, horizon=$4, target_month=$5, owner=$6, discovery_at=$7, discovery_by=$8, version=version+1, updated_at=$9, external_ref=$11 WHERE key=$1 AND version=$10")
            .bind(i.key().to_string()).bind(i.title()).bind(i.brief()).bind(i.horizon()).bind(i.target_month()).bind(i.owner()).bind(i.discovery().map(|d| d.at)).bind(i.discovery().map(|d| d.by.as_str())).bind(now).bind(expected).bind(i.external_ref()).execute(&mut **tx).await?;
        if res.rows_affected() == 1 {
            return Ok(());
        }
        let current: Option<i64> =
            sqlx::query_scalar("SELECT version FROM initiatives WHERE key = $1")
                .bind(i.key().to_string())
                .fetch_optional(&mut **tx)
                .await?;
        Err(current
            .map(|c| RepoError::Conflict { current: c })
            .unwrap_or(RepoError::NotFound))
    }
    async fn story_counts_under_initiative(&self, k: &Key) -> Result<StoryCounts, RepoError> {
        story_counts_on(&self.pool, k).await
    }
    async fn story_counts_under_initiative_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        k: &Key,
    ) -> Result<StoryCounts, RepoError> {
        story_counts_on(&mut **tx, k).await
    }
    async fn story_counts_by_initiative(&self) -> Result<HashMap<Key, StoryCounts>, RepoError> {
        let rows = sqlx::query("SELECT e.initiative_key, count(*) FILTER (WHERE s.resolution = 'completed') AS completed, count(*) FILTER (WHERE s.resolution = 'dropped') AS dropped, count(*) AS total FROM stories s JOIN epics e ON e.key = s.epic_key WHERE e.initiative_key IS NOT NULL GROUP BY e.initiative_key")
            .fetch_all(&self.pool)
            .await?;
        let mut out = HashMap::new();
        for r in rows {
            out.insert(
                key(&r.get::<String, _>("initiative_key")),
                StoryCounts {
                    completed: r.get::<i64, _>("completed") as u32,
                    dropped: r.get::<i64, _>("dropped") as u32,
                    total: r.get::<i64, _>("total") as u32,
                },
            );
        }
        Ok(out)
    }
    async fn insert_epic(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        e: &Epic,
        idem: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, RepoError> {
        let r = sqlx::query("INSERT INTO epics (key, title, initiative_key, brief, external_ref, discovery_at, discovery_by, version, idempotency_key, created_at, updated_at) VALUES ($1,$2,$3,$4,$7,$8,$9,1,$5,$6,$6) ON CONFLICT (idempotency_key) DO NOTHING")
            .bind(e.key().to_string()).bind(e.title()).bind(e.initiative().map(|k| k.to_string())).bind(e.brief()).bind(idem).bind(now).bind(e.external_ref()).bind(e.discovery().map(|d| d.at)).bind(e.discovery().map(|d| d.by.as_str())).execute(&mut **tx).await?;
        Ok(r.rows_affected() == 1)
    }
    async fn get_epic(&self, k: &Key) -> Result<Epic, RepoError> {
        get_epic_on(&self.pool, k).await
    }
    async fn get_epic_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        k: &Key,
    ) -> Result<Epic, RepoError> {
        get_epic_on(&mut **tx, k).await
    }
    async fn list_epics(&self, initiative: Option<&Key>) -> Result<Vec<Epic>, RepoError> {
        Ok(sqlx::query(&format!("SELECT {EPIC_COLS} FROM epics WHERE ($1::text IS NULL OR initiative_key = $1) ORDER BY created_at, {BY_KEY_NUMBER}")).bind(initiative.map(|k| k.to_string())).fetch_all(&self.pool).await?.iter().map(epic_from).collect())
    }
    async fn update_epic(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        e: &Epic,
        expected: i64,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError> {
        let res = sqlx::query("UPDATE epics SET title=$2, initiative_key=$3, brief=$4, discovery_at=$5, discovery_by=$6, version=version+1, updated_at=$7, external_ref=$9 WHERE key=$1 AND version=$8")
            .bind(e.key().to_string()).bind(e.title()).bind(e.initiative().map(|k| k.to_string())).bind(e.brief()).bind(e.discovery().map(|d| d.at)).bind(e.discovery().map(|d| d.by.as_str())).bind(now).bind(expected).bind(e.external_ref()).execute(&mut **tx).await?;
        if res.rows_affected() == 1 {
            return Ok(());
        }
        let current: Option<i64> = sqlx::query_scalar("SELECT version FROM epics WHERE key = $1")
            .bind(e.key().to_string())
            .fetch_optional(&mut **tx)
            .await?;
        Err(current
            .map(|c| RepoError::Conflict { current: c })
            .unwrap_or(RepoError::NotFound))
    }
    async fn insert_story(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        s: &Story,
        idem: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, RepoError> {
        let r = sqlx::query("INSERT INTO stories (key, title, kind, epic_key, description, criteria, owner, labels, rank, delivery, resolution, external_ref, version, idempotency_key, created_at, updated_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,1,$13,$14,$14) ON CONFLICT (idempotency_key) DO NOTHING")
            .bind(s.key().to_string()).bind(s.title()).bind(kind_str(s.kind())).bind(s.epic().map(|k| k.to_string())).bind(s.description()).bind(criteria_json(s.criteria())).bind(s.owner()).bind(s.labels().to_vec()).bind(s.rank()).bind(delivery_str(s.delivery())).bind(s.resolution().map(resolution_str)).bind(s.external_ref()).bind(idem).bind(now).execute(&mut **tx).await?;
        Ok(r.rows_affected() == 1)
    }
    async fn get_story(&self, k: &Key) -> Result<Story, RepoError> {
        let r = sqlx::query(&format!("SELECT {STORY_COLS} FROM stories WHERE key = $1"))
            .bind(k.to_string())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(RepoError::NotFound)?;
        Ok(story_from(&r)?)
    }
    async fn get_story_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        k: &Key,
    ) -> Result<Story, RepoError> {
        let r = sqlx::query(&format!(
            "SELECT {STORY_COLS} FROM stories WHERE key = $1 FOR UPDATE"
        ))
        .bind(k.to_string())
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(RepoError::NotFound)?;
        Ok(story_from(&r)?)
    }
    async fn list_stories(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        f: &StoryFilter,
        done_since: Option<DateTime<Utc>>,
    ) -> Result<Vec<Story>, RepoError> {
        let rows = sqlx::query(&format!("SELECT {STORY_COLS} FROM stories WHERE ($1::text IS NULL OR epic_key = $1) AND ($2::text IS NULL OR owner = $2) AND ($3::text IS NULL OR delivery = $3) AND (NOT $4 OR (kind = 'story' AND epic_key IS NULL)) AND ($5::text IS NULL OR $5 = ANY(labels)) AND ($6::timestamptz IS NULL OR delivery <> 'done' OR done_at >= $6) ORDER BY CASE delivery WHEN 'todo' THEN 0 WHEN 'in_progress' THEN 1 WHEN 'in_review' THEN 2 WHEN 'testing' THEN 3 ELSE 4 END, rank, length(key), key"))
            .bind(f.epic.as_ref().map(|k| k.to_string())).bind(&f.owner).bind(f.delivery.map(delivery_str)).bind(f.untriaged).bind(&f.label).bind(done_since).fetch_all(&mut **tx).await?;
        Ok(rows.iter().map(story_from).collect::<Result<Vec<_>, _>>()?)
    }
    async fn update_story(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        s: &Story,
        rank: Option<i32>,
        expected: i64,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError> {
        // rank is written only when the caller supplied one: a content edit that read the story before a rerank must not
        // restore the stale rank (re-ranks change neighbours' ranks without bumping their version).
        let res = sqlx::query("UPDATE stories SET title=$2, kind=$3, epic_key=$4, description=$5, criteria=$6, owner=$7, labels=$8, rank=COALESCE($9, rank), version=version+1, updated_at=$10, external_ref=$12 WHERE key=$1 AND version=$11")
            .bind(s.key().to_string()).bind(s.title()).bind(kind_str(s.kind())).bind(s.epic().map(|k| k.to_string())).bind(s.description()).bind(criteria_json(s.criteria())).bind(s.owner()).bind(s.labels().to_vec()).bind(rank).bind(now).bind(expected).bind(s.external_ref()).execute(&mut **tx).await?;
        if res.rows_affected() == 1 {
            return Ok(());
        }
        let current: Option<i64> = sqlx::query_scalar("SELECT version FROM stories WHERE key = $1")
            .bind(s.key().to_string())
            .fetch_optional(&mut **tx)
            .await?;
        Err(current
            .map(|c| RepoError::Conflict { current: c })
            .unwrap_or(RepoError::NotFound))
    }
    async fn set_delivery(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        k: &Key,
        expected: i64,
        to: DeliveryState,
        resolution: Option<Resolution>,
        owner_if_none: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError> {
        let res = sqlx::query("UPDATE stories SET delivery=$2, resolution=$3, owner=COALESCE(owner, $4), version=version+1, updated_at=$5, done_at = CASE WHEN $2 <> 'done' THEN NULL WHEN delivery = 'done' THEN done_at ELSE $5 END, started_at = CASE WHEN $2 IN ('in_progress', 'in_review', 'testing') AND delivery IN ('todo', 'done') THEN $5 ELSE started_at END WHERE key=$1 AND version=$6")
            .bind(k.to_string()).bind(delivery_str(to)).bind(resolution.map(resolution_str)).bind(owner_if_none).bind(now).bind(expected).execute(&mut **tx).await?;
        if res.rows_affected() == 1 {
            return Ok(());
        }
        let current: Option<i64> = sqlx::query_scalar("SELECT version FROM stories WHERE key = $1")
            .bind(k.to_string())
            .fetch_optional(&mut **tx)
            .await?;
        Err(current
            .map(|c| RepoError::Conflict { current: c })
            .unwrap_or(RepoError::NotFound))
    }
    async fn todo_column_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<Key>, RepoError> {
        // FOR NO KEY UPDATE, not FOR UPDATE: a rank update touches no key column, and FOR UPDATE would conflict with the
        // FOR KEY SHARE locks add_block's story_blocks foreign-key checks take on both stories, which can deadlock (40P01).
        let ks: Vec<String> = sqlx::query_scalar("SELECT key FROM (SELECT key, rank FROM stories WHERE delivery = 'todo' ORDER BY key FOR NO KEY UPDATE) t ORDER BY rank, length(key), key")
            .fetch_all(&mut **tx)
            .await?;
        Ok(ks.iter().map(|k| key(k)).collect())
    }
    async fn set_ranks(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        ranks: &[(Key, i32)],
    ) -> Result<(), RepoError> {
        let keys: Vec<String> = ranks.iter().map(|(k, _)| k.to_string()).collect();
        let values: Vec<i32> = ranks.iter().map(|(_, r)| *r).collect();
        sqlx::query("UPDATE stories s SET rank = v.rank FROM (SELECT * FROM unnest($1::text[], $2::int[]) AS u(key, rank)) v WHERE s.key = v.key AND s.rank <> v.rank")
            .bind(keys)
            .bind(values)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }
    async fn touch_story(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        k: &Key,
        expected: i64,
        now: DateTime<Utc>,
    ) -> Result<(), RepoError> {
        let res = sqlx::query(
            "UPDATE stories SET version=version+1, updated_at=$2 WHERE key=$1 AND version=$3",
        )
        .bind(k.to_string())
        .bind(now)
        .bind(expected)
        .execute(&mut **tx)
        .await?;
        if res.rows_affected() == 1 {
            return Ok(());
        }
        let current: Option<i64> = sqlx::query_scalar("SELECT version FROM stories WHERE key = $1")
            .bind(k.to_string())
            .fetch_optional(&mut **tx)
            .await?;
        Err(current
            .map(|c| RepoError::Conflict { current: c })
            .unwrap_or(RepoError::NotFound))
    }
    async fn add_block(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: &Key,
        blocker: &Key,
    ) -> Result<bool, RepoError> {
        let r = sqlx::query("INSERT INTO story_blocks (story_key, blocker_key) VALUES ($1,$2) ON CONFLICT DO NOTHING").bind(story.to_string()).bind(blocker.to_string()).execute(&mut **tx).await?;
        Ok(r.rows_affected() == 1)
    }
    async fn remove_block(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: &Key,
        blocker: &Key,
    ) -> Result<bool, RepoError> {
        let r = sqlx::query("DELETE FROM story_blocks WHERE story_key = $1 AND blocker_key = $2")
            .bind(story.to_string())
            .bind(blocker.to_string())
            .execute(&mut **tx)
            .await?;
        Ok(r.rows_affected() == 1)
    }
    async fn blockers_of_many(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        stories: &[Key],
    ) -> Result<HashMap<Key, Vec<Key>>, RepoError> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT story_key, blocker_key FROM story_blocks WHERE story_key = ANY($1) ORDER BY story_key, length(blocker_key), blocker_key",
        )
        .bind(stories.iter().map(Key::to_string).collect::<Vec<_>>())
        .fetch_all(&mut **tx)
        .await?;
        let mut out: HashMap<Key, Vec<Key>> = HashMap::new();
        for (s, b) in rows {
            out.entry(key(&s)).or_default().push(key(&b));
        }
        Ok(out)
    }
    async fn stories_by_keys(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        keys: &[Key],
    ) -> Result<Vec<Story>, RepoError> {
        let rows = sqlx::query(&format!(
            "SELECT {STORY_COLS} FROM stories WHERE key = ANY($1)"
        ))
        .bind(keys.iter().map(Key::to_string).collect::<Vec<_>>())
        .fetch_all(&mut **tx)
        .await?;
        let mut by_key: HashMap<Key, Story> = HashMap::new();
        for r in &rows {
            let s = story_from(r)?;
            by_key.insert(s.key().clone(), s);
        }
        Ok(keys.iter().filter_map(|k| by_key.remove(k)).collect())
    }
    async fn readiness_inputs(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        stories: &[Key],
    ) -> Result<HashMap<Key, ReadinessInput>, RepoError> {
        let stories = self.stories_by_keys(tx, stories).await?;
        let epic_keys: Vec<Key> = distinct(stories.iter().filter_map(|s| s.epic().cloned()));
        let epics: HashMap<Key, Epic> = sqlx::query(&format!(
            "SELECT {EPIC_COLS} FROM epics WHERE key = ANY($1)"
        ))
        .bind(epic_keys.iter().map(Key::to_string).collect::<Vec<_>>())
        .fetch_all(&mut **tx)
        .await?
        .iter()
        .map(|r| {
            let e = epic_from(r);
            (e.key().clone(), e)
        })
        .collect();
        let initiative_keys: Vec<Key> =
            distinct(epics.values().filter_map(|e| e.initiative().cloned()));
        let initiatives: HashMap<Key, Initiative> = sqlx::query(&format!(
            "SELECT {INITIATIVE_COLS} FROM initiatives WHERE key = ANY($1)"
        ))
        .bind(
            initiative_keys
                .iter()
                .map(Key::to_string)
                .collect::<Vec<_>>(),
        )
        .fetch_all(&mut **tx)
        .await?
        .iter()
        .map(|r| {
            let i = initiative_from(r);
            (i.key().clone(), i)
        })
        .collect();
        let on_keys: Vec<String> = stories
            .iter()
            .map(|s| s.key())
            .chain(epic_keys.iter())
            .chain(initiative_keys.iter())
            .map(Key::to_string)
            .collect();
        let rows = sqlx::query("SELECT on_key, id, title, check_result, closed_at, record_path IS NOT NULL AS has_record, record_repo FROM required_decisions WHERE on_key = ANY($1) ORDER BY on_key, id")
            .bind(on_keys)
            .fetch_all(&mut **tx)
            .await?;
        let mut facts: HashMap<Key, Vec<DecisionFact>> = HashMap::new();
        for r in &rows {
            let fact = DecisionFact {
                id: r.get("id"),
                title: r.get("title"),
                check: serde_json::from_value(r.get::<serde_json::Value, _>("check_result"))
                    .map_err(|e| sqlx::Error::Decode(e.into()))?,
                closed: r.get::<Option<DateTime<Utc>>, _>("closed_at").is_some(),
                has_record: r.get("has_record"),
                record_repo: r.get("record_repo"),
                repo_configured: true,
            };
            facts
                .entry(key(r.get::<&str, _>("on_key")))
                .or_default()
                .push(fact);
        }
        let blocker_rows: Vec<(String, String)> = sqlx::query_as("SELECT b.story_key, b.blocker_key FROM story_blocks b JOIN stories x ON x.key = b.blocker_key WHERE b.story_key = ANY($1) AND x.delivery <> 'done' ORDER BY b.story_key, length(b.blocker_key), b.blocker_key")
            .bind(stories.iter().map(|s| s.key().to_string()).collect::<Vec<_>>())
            .fetch_all(&mut **tx)
            .await?;
        let mut blockers: HashMap<Key, Vec<Key>> = HashMap::new();
        for (s, b) in blocker_rows {
            blockers.entry(key(&s)).or_default().push(key(&b));
        }
        let mut out = HashMap::new();
        for s in &stories {
            let mut ancestors = Vec::new();
            if let Some(ek) = s.epic() {
                let e = epics
                    .get(ek)
                    .ok_or(RepoError::Db(sqlx::Error::RowNotFound))?;
                ancestors.push(AncestorFact {
                    key: ek.clone(),
                    discovery_complete: e.discovery().is_some(),
                    decisions: facts.get(ek).cloned().unwrap_or_default(),
                });
                if let Some(ik) = e.initiative() {
                    let i = initiatives
                        .get(ik)
                        .ok_or(RepoError::Db(sqlx::Error::RowNotFound))?;
                    ancestors.push(AncestorFact {
                        key: ik.clone(),
                        discovery_complete: i.discovery().is_some(),
                        decisions: facts.get(ik).cloned().unwrap_or_default(),
                    });
                }
            }
            out.insert(
                s.key().clone(),
                ReadinessInput {
                    kind: s.kind(),
                    has_epic: s.epic().is_some(),
                    criteria_count: s.criteria().len(),
                    ancestors,
                    own_decisions: facts.get(s.key()).cloned().unwrap_or_default(),
                    open_blockers: blockers.remove(s.key()).unwrap_or_default(),
                },
            );
        }
        Ok(out)
    }
    async fn insert_decision(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        on: ItemType,
        on_key: &Key,
        title: &str,
        decider: &str,
        due: Option<NaiveDate>,
        reversibility: Option<Reversibility>,
        idem: &str,
        now: DateTime<Utc>,
    ) -> Result<(RequiredDecision, bool), RepoError> {
        let inserted = sqlx::query(&format!("INSERT INTO required_decisions (on_type, on_key, title, decider, due, reversibility, idempotency_key, created_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT (idempotency_key) DO NOTHING RETURNING {DECISION_COLS}"))
            .bind(item_type_str(on)).bind(on_key.to_string()).bind(title).bind(decider).bind(due).bind(reversibility.map(reversibility_str)).bind(idem).bind(now).fetch_optional(&mut **tx).await?;
        if let Some(r) = inserted {
            return Ok((decision_from(&r)?, true));
        }
        let r = sqlx::query(&format!(
            "SELECT {DECISION_COLS} FROM required_decisions WHERE idempotency_key = $1"
        ))
        .bind(idem)
        .fetch_one(&mut **tx)
        .await?;
        Ok((decision_from(&r)?, false))
    }
    async fn get_decision(&self, id: i64) -> Result<RequiredDecision, RepoError> {
        let r = sqlx::query(&format!(
            "SELECT {DECISION_COLS} FROM required_decisions WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(RepoError::NotFound)?;
        Ok(decision_from(&r)?)
    }
    async fn get_decision_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<RequiredDecision, RepoError> {
        let r = sqlx::query(&format!(
            "SELECT {DECISION_COLS} FROM required_decisions WHERE id = $1 FOR UPDATE"
        ))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(RepoError::NotFound)?;
        Ok(decision_from(&r)?)
    }
    async fn list_decisions_on(&self, on_key: &Key) -> Result<Vec<RequiredDecision>, RepoError> {
        decisions_on(&self.pool, on_key).await
    }
    async fn list_decisions_under_initiative(
        &self,
        initiative: &Key,
    ) -> Result<Vec<RequiredDecision>, RepoError> {
        decisions_under_initiative(&self.pool, initiative).await
    }
    async fn list_decisions_on_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        on_key: &Key,
    ) -> Result<Vec<RequiredDecision>, RepoError> {
        decisions_on(&mut **tx, on_key).await
    }
    async fn list_decisions_under_initiative_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        initiative: &Key,
    ) -> Result<Vec<RequiredDecision>, RepoError> {
        decisions_under_initiative(&mut **tx, initiative).await
    }
    async fn list_all_decisions(
        &self,
        include_closed: bool,
        with_record_only: bool,
    ) -> Result<Vec<RequiredDecision>, RepoError> {
        all_decisions(&self.pool, include_closed, with_record_only).await
    }
    async fn list_all_decisions_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        include_closed: bool,
    ) -> Result<Vec<RequiredDecision>, RepoError> {
        all_decisions(&mut **tx, include_closed, false).await
    }
    async fn save_decision(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        d: &RequiredDecision,
    ) -> Result<(), RepoError> {
        let res = sqlx::query("UPDATE required_decisions SET title=$2, decider=$3, due=$4, record_repo=$5, record_path=$6, record_id=$7, pr_url=$8, check_result=$9, checked_at=$10, closed_at=$11, closed_by=$12, closed_reason=$13, reversibility=$15, closed_kind=$16, closed_via=$17, version=version+1 WHERE id=$1 AND version=$14")
            .bind(d.id).bind(&d.title).bind(&d.decider).bind(d.due).bind(d.record.as_ref().map(|r| &r.repo)).bind(d.record.as_ref().map(|r| &r.path)).bind(d.record.as_ref().map(|r| &r.record_id)).bind(&d.pr_url)
            .bind(serde_json::to_value(&d.check).unwrap()).bind(d.checked_at).bind(d.closed.as_ref().map(|c| c.at)).bind(d.closed.as_ref().map(|c| &c.by)).bind(d.closed.as_ref().map(|c| &c.reason)).bind(d.version).bind(d.reversibility.map(reversibility_str)).bind(d.closed.as_ref().and_then(|c| c.kind).map(closure_kind_str)).bind(d.closed.as_ref().and_then(|c| c.via).map(token_kind_str)).execute(&mut **tx).await?;
        if res.rows_affected() == 1 {
            return Ok(());
        }
        let current: Option<i64> =
            sqlx::query_scalar("SELECT version FROM required_decisions WHERE id = $1")
                .bind(d.id)
                .fetch_optional(&mut **tx)
                .await?;
        Err(current
            .map(|c| RepoError::Conflict { current: c })
            .unwrap_or(RepoError::NotFound))
    }
    async fn touch_decision_checked(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
        at: DateTime<Utc>,
    ) -> Result<(), RepoError> {
        let res = sqlx::query("UPDATE required_decisions SET checked_at = $2 WHERE id = $1")
            .bind(id)
            .bind(at)
            .execute(&mut **tx)
            .await?;
        if res.rows_affected() == 1 {
            Ok(())
        } else {
            Err(RepoError::NotFound)
        }
    }
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
    ) -> Result<(i64, bool), RepoError> {
        // Two unique keys can conflict (external_ref for imports, idempotency_key for callers), so DO NOTHING has no target
        // and the existing row is looked up by whichever key was supplied.
        let inserted: Option<i64> = sqlx::query_scalar("INSERT INTO comments (story_key, decision_id, author, body, at, external_ref, idempotency_key) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING RETURNING id")
            .bind(story.map(|k| k.to_string())).bind(decision).bind(author).bind(body).bind(at).bind(external_ref).bind(idempotency_key).fetch_optional(&mut **tx).await?;
        if let Some(id) = inserted {
            return Ok((id, true));
        }
        let id: i64 = sqlx::query_scalar(
            "SELECT id FROM comments WHERE idempotency_key = $1 OR external_ref = $2 ORDER BY id LIMIT 1",
        )
        .bind(idempotency_key)
        .bind(external_ref)
        .fetch_one(&mut **tx)
        .await?;
        Ok((id, false))
    }
    async fn list_comments_for_story(
        &self,
        story: &Key,
    ) -> Result<Vec<reeve_protocol::CommentDto>, RepoError> {
        comments(&self.pool, "story_key = $1", story.to_string()).await
    }
    async fn list_comments_for_story_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: &Key,
    ) -> Result<Vec<reeve_protocol::CommentDto>, RepoError> {
        comments(&mut **tx, "story_key = $1", story.to_string()).await
    }
    async fn list_comments_for_decision(
        &self,
        id: i64,
    ) -> Result<Vec<reeve_protocol::CommentDto>, RepoError> {
        comments(&self.pool, "decision_id = $1::bigint", id.to_string()).await
    }
    async fn insert_item_comment(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        on: &Key,
        author: &str,
        body: &str,
        idempotency_key: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<(i64, bool), RepoError> {
        let column = match item {
            ItemType::Initiative => "initiative_key",
            ItemType::Epic => "epic_key",
            ItemType::Story => "story_key",
        };
        let inserted: Option<i64> = sqlx::query_scalar(&format!("INSERT INTO comments ({column}, author, body, at, idempotency_key) VALUES ($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING RETURNING id"))
            .bind(on.to_string()).bind(author).bind(body).bind(at).bind(idempotency_key).fetch_optional(&mut **tx).await?;
        if let Some(id) = inserted {
            return Ok((id, true));
        }
        let id: i64 = sqlx::query_scalar(
            "SELECT id FROM comments WHERE idempotency_key = $1 ORDER BY id LIMIT 1",
        )
        .bind(idempotency_key)
        .fetch_one(&mut **tx)
        .await?;
        Ok((id, false))
    }
    async fn list_comments_for_item(
        &self,
        item: ItemType,
        on: &Key,
    ) -> Result<Vec<reeve_protocol::CommentDto>, RepoError> {
        let column = match item {
            ItemType::Initiative => "initiative_key = $1",
            ItemType::Epic => "epic_key = $1",
            ItemType::Story => "story_key = $1",
        };
        comments(&self.pool, column, on.to_string()).await
    }

    async fn pull_requests_for_many(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        stories: &[Key],
    ) -> Result<HashMap<Key, Vec<PullRequest>>, RepoError> {
        let rows = sqlx::query(
            "SELECT story_key, repo, number, url, title, status, updated_at FROM story_prs \
             WHERE story_key = ANY($1) ORDER BY story_key, repo, number",
        )
        .bind(stories.iter().map(Key::to_string).collect::<Vec<_>>())
        .fetch_all(&mut **tx)
        .await?;
        let mut out: HashMap<Key, Vec<PullRequest>> = HashMap::new();
        for r in &rows {
            out.entry(key(r.get::<&str, _>("story_key")))
                .or_default()
                .push(pull_request_from(r)?);
        }
        Ok(out)
    }
    async fn upsert_pull_request(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        story: &Key,
        pr: &PullRequest,
    ) -> Result<PrChange, RepoError> {
        let incoming = story;
        let story = story.to_string();
        // A concurrent first delivery waits here on the unique index until the winner commits, then sees the
        // conflict and falls through to the locking path below: no unique violation ever reaches the caller.
        let inserted: Option<String> = sqlx::query_scalar(
            "INSERT INTO story_prs (story_key, repo, number, url, title, status, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (repo, number) DO NOTHING RETURNING story_key",
        )
        .bind(&story)
        .bind(&pr.repo)
        .bind(pr.number)
        .bind(&pr.url)
        .bind(&pr.title)
        .bind(pr_status_str(pr.status))
        .bind(pr.updated_at)
        .fetch_optional(&mut **tx)
        .await?;
        if inserted.is_some() {
            return Ok(PrChange::Linked);
        }
        let r = sqlx::query(
            "SELECT story_key, repo, number, url, title, status, updated_at FROM story_prs \
             WHERE repo = $1 AND number = $2 FOR UPDATE",
        )
        .bind(&pr.repo)
        .bind(pr.number)
        .fetch_one(&mut **tx)
        .await?;
        let stored_story = key(r.get::<&str, _>("story_key"));
        let stored = pull_request_from(&r)?;
        let change = decide(&stored_story, &stored, incoming, pr);
        if matches!(
            change,
            PrChange::Updated | PrChange::Touched | PrChange::Relinked { .. }
        ) {
            sqlx::query(
                "UPDATE story_prs SET story_key = $3, url = $4, title = $5, status = $6, updated_at = $7 \
                 WHERE repo = $1 AND number = $2",
            )
            .bind(&pr.repo)
            .bind(pr.number)
            .bind(&story)
            .bind(&pr.url)
            .bind(&pr.title)
            .bind(pr_status_str(pr.status))
            .bind(pr.updated_at)
            .execute(&mut **tx)
            .await?;
        }
        Ok(change)
    }

    async fn initiatives_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<Initiative>, RepoError> {
        Ok(sqlx::query(&format!(
            "SELECT {INITIATIVE_COLS} FROM initiatives ORDER BY {BY_KEY_NUMBER}"
        ))
        .fetch_all(&mut **tx)
        .await?
        .iter()
        .map(initiative_from)
        .collect())
    }
    async fn epics_in(&self, tx: &mut Transaction<'_, Postgres>) -> Result<Vec<Epic>, RepoError> {
        Ok(sqlx::query(&format!(
            "SELECT {EPIC_COLS} FROM epics ORDER BY {BY_KEY_NUMBER}"
        ))
        .fetch_all(&mut **tx)
        .await?
        .iter()
        .map(epic_from)
        .collect())
    }
    async fn stories_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<Story>, RepoError> {
        let rows = sqlx::query(&format!(
            "SELECT {STORY_COLS} FROM stories ORDER BY {BY_KEY_NUMBER}"
        ))
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(story_from).collect::<Result<Vec<_>, _>>()?)
    }
    async fn decisions_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<RequiredDecision>, RepoError> {
        Ok(sqlx::query(&format!(
            "SELECT {DECISION_COLS} FROM required_decisions ORDER BY id"
        ))
        .fetch_all(&mut **tx)
        .await?
        .iter()
        .map(decision_from)
        .collect::<Result<Vec<_>, _>>()?)
    }
    async fn comments_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<reeve_protocol::CommentDto>, RepoError> {
        let rows = sqlx::query(&format!("SELECT {COMMENT_COLS} FROM comments ORDER BY id"))
            .fetch_all(&mut **tx)
            .await?;
        Ok(rows.iter().map(comment_from).collect())
    }
    async fn sprint_in_state(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        state: reeve_protocol::SprintState,
    ) -> Result<Option<Sprint>, RepoError> {
        repo_sprint::sprint_in_state(tx, state).await
    }
    async fn sprint_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<Sprint, RepoError> {
        repo_sprint::sprint_for_update(tx, id).await
    }
    async fn get_sprint_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<Sprint, RepoError> {
        repo_sprint::get_sprint_in(tx, id).await
    }
    async fn sprints_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<Sprint>, RepoError> {
        repo_sprint::sprints_in(tx).await
    }
    async fn insert_draft(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        starts_on: NaiveDate,
        ends_on: NaiveDate,
        now: DateTime<Utc>,
    ) -> Result<Option<Sprint>, RepoError> {
        repo_sprint::insert_draft(tx, starts_on, ends_on, now).await
    }
    async fn save_sprint(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        s: &Sprint,
    ) -> Result<(), RepoError> {
        repo_sprint::save_sprint(tx, s).await
    }
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
    ) -> Result<(Goal, bool), RepoError> {
        repo_sprint::insert_goal(tx, sprint_id, outcome, check, owner, notes, idem, now).await
    }
    async fn goal_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<Goal, RepoError> {
        repo_sprint::goal_for_update(tx, id).await
    }
    async fn get_goal_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<Goal, RepoError> {
        repo_sprint::get_goal_in(tx, id).await
    }
    async fn save_goal(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        g: &Goal,
    ) -> Result<(), RepoError> {
        repo_sprint::save_goal(tx, g).await
    }
    async fn delete_goal(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<(), RepoError> {
        repo_sprint::delete_goal(tx, id).await
    }
    async fn goals_of_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        sprint_ids: &[i64],
    ) -> Result<Vec<Goal>, RepoError> {
        repo_sprint::goals_of_in(tx, sprint_ids).await
    }
    async fn goals_by_ids_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        ids: &[i64],
    ) -> Result<Vec<Goal>, RepoError> {
        repo_sprint::goals_by_ids_in(tx, ids).await
    }
    async fn insert_goal_link(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        goal_id: i64,
        item: &Key,
        by: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, RepoError> {
        repo_sprint::insert_goal_link(tx, goal_id, item, by, now).await
    }
    async fn delete_goal_link(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        goal_id: i64,
        item: &Key,
    ) -> Result<bool, RepoError> {
        repo_sprint::delete_goal_link(tx, goal_id, item).await
    }
    async fn urgent_marks_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<MarkFact>, RepoError> {
        repo_urgent::urgent_marks_in(tx).await
    }
    async fn urgency_lineage_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        epics: &[Key],
        initiatives: &[Key],
    ) -> Result<Lineage, RepoError> {
        repo_urgent::urgency_lineage_in(tx, epics, initiatives).await
    }
    async fn set_urgent_mark(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        key: &Key,
        mark: &UrgentMark,
    ) -> Result<bool, RepoError> {
        repo_urgent::set_urgent_mark(tx, item, key, mark).await
    }
    async fn clear_urgent_mark(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: ItemType,
        key: &Key,
    ) -> Result<bool, RepoError> {
        repo_urgent::clear_urgent_mark(tx, item, key).await
    }
    async fn links_of_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        goal_ids: &[i64],
    ) -> Result<Vec<GoalLink>, RepoError> {
        repo_sprint::links_of_in(tx, goal_ids).await
    }
    async fn item_types_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        keys: &[Key],
    ) -> Result<HashMap<Key, ItemType>, RepoError> {
        repo_sprint::item_types_in(tx, keys).await
    }
    async fn active_goals_of_stories(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        stories: &[Key],
    ) -> Result<HashMap<Key, Vec<(i64, String, reeve_protocol::GoalVia)>>, RepoError> {
        repo_sprint::active_goals_of_stories(tx, stories).await
    }
    async fn sprint_story_times(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        linked: &[Key],
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<StoryTimes>, RepoError> {
        repo_sprint::sprint_story_times(tx, linked, from, to).await
    }
}

fn pr_status_str(s: PrStatus) -> &'static str {
    match s {
        PrStatus::Draft => "draft",
        PrStatus::Open => "open",
        PrStatus::Merged => "merged",
        PrStatus::Closed => "closed",
    }
}
fn pr_status_of(s: &str) -> Result<PrStatus, sqlx::Error> {
    match s {
        "draft" => Ok(PrStatus::Draft),
        "open" => Ok(PrStatus::Open),
        "merged" => Ok(PrStatus::Merged),
        "closed" => Ok(PrStatus::Closed),
        _ => Err(bad("pull request status", s)),
    }
}
fn pull_request_from(r: &sqlx::postgres::PgRow) -> Result<PullRequest, sqlx::Error> {
    Ok(PullRequest {
        repo: r.get("repo"),
        number: r.get("number"),
        url: r.get("url"),
        title: r.get("title"),
        status: pr_status_of(r.get::<&str, _>("status"))?,
        updated_at: r.get("updated_at"),
    })
}

/// The distinct keys of `it`, in first-seen order.
fn distinct(it: impl Iterator<Item = Key>) -> Vec<Key> {
    let mut seen = std::collections::HashSet::new();
    it.filter(|k| seen.insert(k.clone())).collect()
}

// Each read below exists once, generic over the executor: the pool method and its `_in` twin (the caller's
// transaction) both call it, so the two can never drift apart.

async fn get_initiative_on<'e, E: sqlx::PgExecutor<'e>>(
    e: E,
    k: &Key,
) -> Result<Initiative, RepoError> {
    let r = sqlx::query(&format!(
        "SELECT {INITIATIVE_COLS} FROM initiatives WHERE key = $1"
    ))
    .bind(k.to_string())
    .fetch_optional(e)
    .await?
    .ok_or(RepoError::NotFound)?;
    Ok(initiative_from(&r))
}

async fn story_counts_on<'e, E: sqlx::PgExecutor<'e>>(
    e: E,
    k: &Key,
) -> Result<StoryCounts, RepoError> {
    let r = sqlx::query("SELECT count(*) FILTER (WHERE s.resolution = 'completed') AS completed, count(*) FILTER (WHERE s.resolution = 'dropped') AS dropped, count(*) AS total FROM stories s JOIN epics e ON e.key = s.epic_key WHERE e.initiative_key = $1").bind(k.to_string()).fetch_one(e).await?;
    Ok(StoryCounts {
        completed: r.get::<i64, _>("completed") as u32,
        dropped: r.get::<i64, _>("dropped") as u32,
        total: r.get::<i64, _>("total") as u32,
    })
}

async fn get_epic_on<'e, E: sqlx::PgExecutor<'e>>(e: E, k: &Key) -> Result<Epic, RepoError> {
    let r = sqlx::query(&format!("SELECT {EPIC_COLS} FROM epics WHERE key = $1"))
        .bind(k.to_string())
        .fetch_optional(e)
        .await?
        .ok_or(RepoError::NotFound)?;
    Ok(epic_from(&r))
}

async fn decisions_on<'e, E: sqlx::PgExecutor<'e>>(
    e: E,
    on_key: &Key,
) -> Result<Vec<RequiredDecision>, RepoError> {
    Ok(sqlx::query(&format!(
        "SELECT {DECISION_COLS} FROM required_decisions WHERE on_key = $1 ORDER BY id"
    ))
    .bind(on_key.to_string())
    .fetch_all(e)
    .await?
    .iter()
    .map(decision_from)
    .collect::<Result<Vec<_>, _>>()?)
}

async fn decisions_under_initiative<'e, E: sqlx::PgExecutor<'e>>(
    e: E,
    initiative: &Key,
) -> Result<Vec<RequiredDecision>, RepoError> {
    Ok(sqlx::query(&format!(
        "SELECT {DECISION_COLS} FROM required_decisions WHERE on_key = $1 \
         OR on_key IN (SELECT key FROM epics WHERE initiative_key = $1) \
         OR on_key IN (SELECT s.key FROM stories s JOIN epics e ON s.epic_key = e.key WHERE e.initiative_key = $1) \
         ORDER BY id"
    ))
    .bind(initiative.to_string())
    .fetch_all(e)
    .await?
    .iter()
    .map(decision_from)
    .collect::<Result<Vec<_>, _>>()?)
}

async fn all_decisions<'e, E: sqlx::PgExecutor<'e>>(
    e: E,
    include_closed: bool,
    with_record_only: bool,
) -> Result<Vec<RequiredDecision>, RepoError> {
    Ok(sqlx::query(&format!("SELECT {DECISION_COLS} FROM required_decisions WHERE ($1 OR closed_at IS NULL) AND (NOT $2 OR record_path IS NOT NULL) ORDER BY created_at, id")).bind(include_closed).bind(with_record_only).fetch_all(e).await?.iter().map(decision_from).collect::<Result<Vec<_>, _>>()?)
}

async fn comments<'e, E: sqlx::PgExecutor<'e>>(
    e: E,
    where_clause: &str,
    arg: String,
) -> Result<Vec<reeve_protocol::CommentDto>, RepoError> {
    let rows = sqlx::query(&format!(
        "SELECT {COMMENT_COLS} FROM comments WHERE {where_clause} ORDER BY at, id"
    ))
    .bind(arg)
    .fetch_all(e)
    .await?;
    Ok(rows.iter().map(comment_from).collect())
}

fn comment_from(r: &sqlx::postgres::PgRow) -> reeve_protocol::CommentDto {
    reeve_protocol::CommentDto {
        id: r.get("id"),
        on_story: r.get::<Option<String>, _>("story_key").as_deref().map(key),
        on_decision: r.get("decision_id"),
        on_initiative: r
            .get::<Option<String>, _>("initiative_key")
            .as_deref()
            .map(key),
        on_epic: r.get::<Option<String>, _>("epic_key").as_deref().map(key),
        author: r.get("author"),
        body: r.get("body"),
        at: r.get("at"),
        external_ref: r.get("external_ref"),
    }
}
