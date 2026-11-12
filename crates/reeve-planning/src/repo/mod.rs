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
};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use reeve_protocol::{
    ClosureKind, Confidence, DeliveryState, GoalVia, ItemType, Key, Kind, PrStatus, Resolution,
    Reversibility, SprintState, StorySort, TokenKind, Verdict,
};
use sqlx::{PgPool, Postgres, Row, Transaction};
use std::{collections::HashMap, sync::Arc};

mod attachments;
mod comments;
mod decisions;
mod epics;
mod initiatives;
mod keys;
mod promise;
mod pull_requests;
mod reads;
mod sprints;
mod stories;
mod urgency;

pub(crate) struct PgPlanningRepo {
    pool: PgPool,
}
impl PgPlanningRepo {
    pub(crate) fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}
pub fn make_planning_ports(pool: PgPool) -> PlanningPorts {
    let r = Arc::new(PgPlanningRepo::new(pool));
    PlanningPorts {
        keys: r.clone(),
        attachments: r.clone(),
        initiatives: r.clone(),
        epics: r.clone(),
        stories: r.clone(),
        decisions: r.clone(),
        comments: r.clone(),
        pull_requests: r.clone(),
        sprints: r.clone(),
        urgency: r.clone(),
        promises: r.clone(),
        reads: r,
    }
}

type Tx<'a, 'b> = &'a mut Transaction<'b, Postgres>;

/// `text` as an ILIKE substring pattern (used with `ESCAPE '\\'`): `%`, `_` and `\\` in it match only themselves.
fn like_pattern(text: &str) -> String {
    let mut pattern = String::from("%");
    for ch in text.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    pattern.push('%');
    pattern
}

fn kind_str(k: Kind) -> &'static str {
    match k {
        Kind::Story => "story",
        Kind::Bug => "bug",
        Kind::Chore => "chore",
    }
}
fn bad(what: &str, s: &str) -> sqlx::Error {
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
fn delivery_of(s: &str) -> Result<DeliveryState, sqlx::Error> {
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
fn resolution_of(s: Option<&str>) -> Result<Option<Resolution>, sqlx::Error> {
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
fn key(s: &str) -> Key {
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
        (Some(repo), Some(path), Some(record_id)) => {
            Some(RecordRef::from_persisted(repo, path, record_id))
        }
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
    Ok(RequiredDecision::from_persisted(
        r.get("id"),
        key(r.get("on_key")),
        on_type,
        r.get("title"),
        r.get("decider"),
        r.get("due"),
        reversibility,
        record,
        r.get("pr_url"),
        serde_json::from_value(r.get::<serde_json::Value, _>("check_result"))
            .map_err(|e| sqlx::Error::Decode(e.into()))?,
        r.get("checked_at"),
        closed,
        r.get("created_at"),
        r.get("version"),
    ))
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
