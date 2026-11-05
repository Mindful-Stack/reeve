//! The SQL behind the sprint, goal and link methods of `PlanningRepo` (kb/ADR-0014). Free functions the repository
//! delegates to, so `repo.rs` does not grow. Nothing here reads the clock: every timestamp is passed in.
use crate::{
    domain::sprint::{Goal, GoalLink, Sprint, StoryTimes},
    ports::RepoError,
    repo::{bad, delivery_of, key, resolution_of},
};
use chrono::{DateTime, NaiveDate, Utc};
use reeve_protocol::{Confidence, GoalVia, ItemType, Key, SprintState, Verdict};
use sqlx::{Postgres, Row, Transaction};
use std::collections::HashMap;

type Tx<'a, 'b> = &'a mut Transaction<'b, Postgres>;

fn state_str(s: SprintState) -> &'static str {
    match s {
        SprintState::Draft => "draft",
        SprintState::Active => "active",
        SprintState::Closed => "closed",
    }
}
fn sprint_state_of(s: &str) -> Result<SprintState, sqlx::Error> {
    match s {
        "draft" => Ok(SprintState::Draft),
        "active" => Ok(SprintState::Active),
        "closed" => Ok(SprintState::Closed),
        _ => Err(bad("sprint state", s)),
    }
}
fn confidence_str(c: Confidence) -> &'static str {
    match c {
        Confidence::OnTrack => "on_track",
        Confidence::AtRisk => "at_risk",
        Confidence::OffTrack => "off_track",
    }
}
fn confidence_of(s: Option<&str>) -> Result<Option<Confidence>, sqlx::Error> {
    match s {
        None => Ok(None),
        Some("on_track") => Ok(Some(Confidence::OnTrack)),
        Some("at_risk") => Ok(Some(Confidence::AtRisk)),
        Some("off_track") => Ok(Some(Confidence::OffTrack)),
        Some(other) => Err(bad("confidence", other)),
    }
}
fn verdict_str(v: Verdict) -> &'static str {
    match v {
        Verdict::Met => "met",
        Verdict::PartlyMet => "partly_met",
        Verdict::Missed => "missed",
    }
}
fn verdict_of(s: Option<&str>) -> Result<Option<Verdict>, sqlx::Error> {
    match s {
        None => Ok(None),
        Some("met") => Ok(Some(Verdict::Met)),
        Some("partly_met") => Ok(Some(Verdict::PartlyMet)),
        Some("missed") => Ok(Some(Verdict::Missed)),
        Some(other) => Err(bad("verdict", other)),
    }
}
fn via_of(s: &str) -> Result<GoalVia, sqlx::Error> {
    match s {
        "story" => Ok(GoalVia::Story),
        "epic" => Ok(GoalVia::Epic),
        "initiative" => Ok(GoalVia::Initiative),
        _ => Err(bad("goal via", s)),
    }
}

const SPRINT_COLS: &str = "id, state, starts_on, ends_on, opened_at, closed_at, version";
const GOAL_COLS: &str = "id, sprint_id, outcome, check_text, owner, notes, confidence, confidence_note, confidence_at, \
     suggested_confidence, suggested_note, suggested_at, suggested_by, verdict, verdict_note, version";

fn sprint_from(r: &sqlx::postgres::PgRow) -> Result<Sprint, sqlx::Error> {
    Ok(Sprint {
        id: r.get("id"),
        state: sprint_state_of(r.get("state"))?,
        starts_on: r.get("starts_on"),
        ends_on: r.get("ends_on"),
        opened_at: r.get("opened_at"),
        closed_at: r.get("closed_at"),
        version: r.get("version"),
    })
}
fn goal_from(r: &sqlx::postgres::PgRow) -> Result<Goal, sqlx::Error> {
    Ok(Goal {
        id: r.get("id"),
        sprint_id: r.get("sprint_id"),
        outcome: r.get("outcome"),
        check: r.get("check_text"),
        owner: r.get("owner"),
        notes: r.get("notes"),
        confidence: confidence_of(r.get::<Option<String>, _>("confidence").as_deref())?,
        confidence_note: r.get("confidence_note"),
        confidence_at: r.get("confidence_at"),
        suggested_confidence: confidence_of(
            r.get::<Option<String>, _>("suggested_confidence")
                .as_deref(),
        )?,
        suggested_note: r.get("suggested_note"),
        suggested_at: r.get("suggested_at"),
        suggested_by: r.get("suggested_by"),
        verdict: verdict_of(r.get::<Option<String>, _>("verdict").as_deref())?,
        verdict_note: r.get("verdict_note"),
        version: r.get("version"),
    })
}
fn story_times_from(r: &sqlx::postgres::PgRow) -> Result<StoryTimes, sqlx::Error> {
    Ok(StoryTimes {
        key: key(r.get("key")),
        title: r.get("title"),
        epic: r.get::<Option<String>, _>("epic_key").as_deref().map(key),
        initiative: r
            .get::<Option<String>, _>("initiative_key")
            .as_deref()
            .map(key),
        delivery: delivery_of(r.get("delivery"))?,
        resolution: resolution_of(r.get::<Option<String>, _>("resolution").as_deref())?,
        started_at: r.get("started_at"),
        done_at: r.get("done_at"),
    })
}

pub(crate) async fn sprint_in_state(
    tx: Tx<'_, '_>,
    state: SprintState,
) -> Result<Option<Sprint>, RepoError> {
    let row = sqlx::query(&format!(
        "SELECT {SPRINT_COLS} FROM sprints WHERE state = $1"
    ))
    .bind(state_str(state))
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.as_ref().map(sprint_from).transpose()?)
}

pub(crate) async fn sprint_for_update(tx: Tx<'_, '_>, id: i64) -> Result<Sprint, RepoError> {
    let row = sqlx::query(&format!(
        "SELECT {SPRINT_COLS} FROM sprints WHERE id = $1 FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    row.as_ref()
        .map(sprint_from)
        .transpose()?
        .ok_or(RepoError::NotFound)
}

pub(crate) async fn get_sprint_in(tx: Tx<'_, '_>, id: i64) -> Result<Sprint, RepoError> {
    let row = sqlx::query(&format!("SELECT {SPRINT_COLS} FROM sprints WHERE id = $1"))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?;
    row.as_ref()
        .map(sprint_from)
        .transpose()?
        .ok_or(RepoError::NotFound)
}

pub(crate) async fn sprints_in(tx: Tx<'_, '_>) -> Result<Vec<Sprint>, RepoError> {
    let rows = sqlx::query(&format!(
        "SELECT {SPRINT_COLS} FROM sprints ORDER BY id DESC"
    ))
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.iter().map(sprint_from).collect::<Result<_, _>>()?)
}

pub(crate) async fn insert_draft(
    tx: Tx<'_, '_>,
    starts_on: NaiveDate,
    ends_on: NaiveDate,
    now: DateTime<Utc>,
) -> Result<Option<Sprint>, RepoError> {
    let row = sqlx::query(&format!(
        "INSERT INTO sprints (state, starts_on, ends_on, created_at) VALUES ('draft', $1, $2, $3) \
         ON CONFLICT DO NOTHING RETURNING {SPRINT_COLS}"
    ))
    .bind(starts_on)
    .bind(ends_on)
    .bind(now)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.as_ref().map(sprint_from).transpose()?)
}

pub(crate) async fn save_sprint(tx: Tx<'_, '_>, s: &Sprint) -> Result<(), RepoError> {
    let res = sqlx::query(
        "UPDATE sprints SET state=$2, starts_on=$3, ends_on=$4, opened_at=$5, closed_at=$6, version=version+1 \
         WHERE id=$1 AND version=$7",
    )
    .bind(s.id)
    .bind(state_str(s.state))
    .bind(s.starts_on)
    .bind(s.ends_on)
    .bind(s.opened_at)
    .bind(s.closed_at)
    .bind(s.version)
    .execute(&mut **tx)
    .await?;
    if res.rows_affected() == 1 {
        return Ok(());
    }
    conflict_or_missing(tx, "sprints", s.id).await
}

/// After an UPDATE that matched nothing: the row's current version (Conflict) or NotFound.
async fn conflict_or_missing(tx: Tx<'_, '_>, table: &str, id: i64) -> Result<(), RepoError> {
    let current: Option<i64> =
        sqlx::query_scalar(&format!("SELECT version FROM {table} WHERE id = $1"))
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?;
    Err(current
        .map(|c| RepoError::Conflict { current: c })
        .unwrap_or(RepoError::NotFound))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_goal(
    tx: Tx<'_, '_>,
    sprint_id: i64,
    outcome: &str,
    check: &str,
    owner: &str,
    notes: &str,
    idem: &str,
    now: DateTime<Utc>,
) -> Result<(Goal, bool), RepoError> {
    let row = sqlx::query(&format!(
        "INSERT INTO goals (sprint_id, outcome, check_text, owner, notes, idempotency_key, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (idempotency_key) DO NOTHING RETURNING {GOAL_COLS}"
    ))
    .bind(sprint_id)
    .bind(outcome)
    .bind(check)
    .bind(owner)
    .bind(notes)
    .bind(idem)
    .bind(now)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(r) = row {
        return Ok((goal_from(&r)?, true));
    }
    let r = sqlx::query(&format!(
        "SELECT {GOAL_COLS} FROM goals WHERE idempotency_key = $1"
    ))
    .bind(idem)
    .fetch_one(&mut **tx)
    .await?;
    Ok((goal_from(&r)?, false))
}

pub(crate) async fn goal_for_update(tx: Tx<'_, '_>, id: i64) -> Result<Goal, RepoError> {
    let row = sqlx::query(&format!(
        "SELECT {GOAL_COLS} FROM goals WHERE id = $1 FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    row.as_ref()
        .map(goal_from)
        .transpose()?
        .ok_or(RepoError::NotFound)
}

pub(crate) async fn get_goal_in(tx: Tx<'_, '_>, id: i64) -> Result<Goal, RepoError> {
    let row = sqlx::query(&format!("SELECT {GOAL_COLS} FROM goals WHERE id = $1"))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?;
    row.as_ref()
        .map(goal_from)
        .transpose()?
        .ok_or(RepoError::NotFound)
}

pub(crate) async fn save_goal(tx: Tx<'_, '_>, g: &Goal) -> Result<(), RepoError> {
    let res = sqlx::query(
        "UPDATE goals SET outcome=$2, check_text=$3, owner=$4, notes=$5, confidence=$6, confidence_note=$7, \
         confidence_at=$8, suggested_confidence=$9, suggested_note=$10, suggested_at=$11, suggested_by=$12, \
         verdict=$13, verdict_note=$14, version=version+1 WHERE id=$1 AND version=$15",
    )
    .bind(g.id)
    .bind(&g.outcome)
    .bind(&g.check)
    .bind(&g.owner)
    .bind(&g.notes)
    .bind(g.confidence.map(confidence_str))
    .bind(&g.confidence_note)
    .bind(g.confidence_at)
    .bind(g.suggested_confidence.map(confidence_str))
    .bind(&g.suggested_note)
    .bind(g.suggested_at)
    .bind(&g.suggested_by)
    .bind(g.verdict.map(verdict_str))
    .bind(&g.verdict_note)
    .bind(g.version)
    .execute(&mut **tx)
    .await?;
    if res.rows_affected() == 1 {
        return Ok(());
    }
    conflict_or_missing(tx, "goals", g.id).await
}

pub(crate) async fn delete_goal(tx: Tx<'_, '_>, id: i64) -> Result<(), RepoError> {
    let res = sqlx::query("DELETE FROM goals WHERE id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    if res.rows_affected() == 0 {
        return Err(RepoError::NotFound);
    }
    Ok(())
}

pub(crate) async fn goals_of_in(
    tx: Tx<'_, '_>,
    sprint_ids: &[i64],
) -> Result<Vec<Goal>, RepoError> {
    let rows = sqlx::query(&format!(
        "SELECT {GOAL_COLS} FROM goals WHERE sprint_id = ANY($1) ORDER BY sprint_id, id"
    ))
    .bind(sprint_ids)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.iter().map(goal_from).collect::<Result<_, _>>()?)
}

pub(crate) async fn goals_by_ids_in(tx: Tx<'_, '_>, ids: &[i64]) -> Result<Vec<Goal>, RepoError> {
    let rows = sqlx::query(&format!(
        "SELECT {GOAL_COLS} FROM goals WHERE id = ANY($1) ORDER BY id"
    ))
    .bind(ids)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.iter().map(goal_from).collect::<Result<_, _>>()?)
}

pub(crate) async fn insert_goal_link(
    tx: Tx<'_, '_>,
    goal_id: i64,
    item: &Key,
    by: &str,
    now: DateTime<Utc>,
) -> Result<bool, RepoError> {
    let res = sqlx::query(
        "INSERT INTO goal_links (goal_id, item_key, linked_at, linked_by) VALUES ($1, $2, $3, $4) \
         ON CONFLICT DO NOTHING",
    )
    .bind(goal_id)
    .bind(item.to_string())
    .bind(now)
    .bind(by)
    .execute(&mut **tx)
    .await?;
    Ok(res.rows_affected() == 1)
}

pub(crate) async fn delete_goal_link(
    tx: Tx<'_, '_>,
    goal_id: i64,
    item: &Key,
) -> Result<bool, RepoError> {
    let res = sqlx::query("DELETE FROM goal_links WHERE goal_id = $1 AND item_key = $2")
        .bind(goal_id)
        .bind(item.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(res.rows_affected() == 1)
}

pub(crate) async fn links_of_in(
    tx: Tx<'_, '_>,
    goal_ids: &[i64],
) -> Result<Vec<GoalLink>, RepoError> {
    let rows = sqlx::query(
        "SELECT goal_id, item_key FROM goal_links WHERE goal_id = ANY($1) \
         ORDER BY goal_id, length(item_key), item_key",
    )
    .bind(goal_ids)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .iter()
        .map(|r| GoalLink {
            goal_id: r.get("goal_id"),
            item_key: key(r.get("item_key")),
        })
        .collect())
}

pub(crate) async fn item_types_in(
    tx: Tx<'_, '_>,
    keys: &[Key],
) -> Result<HashMap<Key, ItemType>, RepoError> {
    let keys: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
    let rows = sqlx::query(
        "SELECT DISTINCT ON (key) key, t FROM (\
           SELECT key, 'story' AS t, 0 AS o FROM stories WHERE key = ANY($1) \
           UNION ALL SELECT key, 'epic', 1 FROM epics WHERE key = ANY($1) \
           UNION ALL SELECT key, 'initiative', 2 FROM initiatives WHERE key = ANY($1) \
         ) x ORDER BY key, o",
    )
    .bind(&keys)
    .fetch_all(&mut **tx)
    .await?;
    let mut out = HashMap::with_capacity(rows.len());
    for r in rows {
        let t = match r.get::<&str, _>("t") {
            "story" => ItemType::Story,
            "epic" => ItemType::Epic,
            "initiative" => ItemType::Initiative,
            other => return Err(bad("item type", other).into()),
        };
        out.insert(key(r.get("key")), t);
    }
    Ok(out)
}

pub(crate) async fn active_goals_of_stories(
    tx: Tx<'_, '_>,
    stories: &[Key],
) -> Result<HashMap<Key, Vec<(i64, String, GoalVia)>>, RepoError> {
    let keys: Vec<String> = stories.iter().map(|k| k.to_string()).collect();
    let rows = sqlx::query(
        "SELECT s.key, g.id, g.outcome, CASE gl.item_key WHEN s.key THEN 'story' WHEN s.epic_key THEN 'epic' ELSE 'initiative' END AS via \
         FROM stories s LEFT JOIN epics e ON e.key = s.epic_key \
         JOIN goal_links gl ON gl.item_key IN (s.key, s.epic_key, e.initiative_key) \
         JOIN goals g ON g.id = gl.goal_id JOIN sprints sp ON sp.id = g.sprint_id AND sp.state = 'active' \
         WHERE s.key = ANY($1) \
         ORDER BY s.key, CASE gl.item_key WHEN s.key THEN 0 WHEN s.epic_key THEN 1 ELSE 2 END, g.id",
    )
    .bind(&keys)
    .fetch_all(&mut **tx)
    .await?;
    let mut out: HashMap<Key, Vec<(i64, String, GoalVia)>> = HashMap::new();
    for r in rows {
        let e = out.entry(key(r.get("key"))).or_default();
        let id: i64 = r.get("id");
        if !e.iter().any(|(g, ..)| *g == id) {
            e.push((id, r.get("outcome"), via_of(r.get("via"))?));
        }
    }
    Ok(out)
}

pub(crate) async fn sprint_story_times(
    tx: Tx<'_, '_>,
    linked: &[Key],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<StoryTimes>, RepoError> {
    let linked: Vec<String> = linked.iter().map(|k| k.to_string()).collect();
    let rows = sqlx::query(
        "SELECT s.key, s.title, s.epic_key, e.initiative_key, s.delivery, s.resolution, s.started_at, s.done_at \
         FROM stories s LEFT JOIN epics e ON e.key = s.epic_key \
         WHERE s.key = ANY($1) OR s.epic_key = ANY($1) OR e.initiative_key = ANY($1) \
            OR (s.done_at >= $2 AND s.done_at < $3) \
         ORDER BY length(s.key), s.key",
    )
    .bind(&linked)
    .bind(from)
    .bind(to)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .iter()
        .map(story_times_from)
        .collect::<Result<_, _>>()?)
}
