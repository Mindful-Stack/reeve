//! The SQL behind the sprint, goal and link methods (kb/ADR-0014). Free functions the `SprintRepo` impl delegates to.
//! Nothing here reads the clock: every timestamp is passed in.
use super::*;

#[async_trait]
#[allow(clippy::too_many_arguments)]
impl SprintRepo for PgPlanningRepo {
    async fn sprint_in_state(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        state: reeve_protocol::SprintState,
    ) -> Result<Option<Sprint>, RepoError> {
        sprint_in_state(tx, state).await
    }
    async fn sprint_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<Sprint, RepoError> {
        sprint_for_update(tx, id).await
    }
    async fn get_sprint_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<Sprint, RepoError> {
        get_sprint_in(tx, id).await
    }
    async fn sprints_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<Sprint>, RepoError> {
        sprints_in(tx).await
    }
    async fn planned_ends_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<Sprint>, RepoError> {
        planned_ends_in(tx).await
    }
    async fn insert_draft(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        starts_on: NaiveDate,
        ends_on: NaiveDate,
        now: DateTime<Utc>,
    ) -> Result<Option<Sprint>, RepoError> {
        insert_draft(tx, starts_on, ends_on, now).await
    }
    async fn save_sprint(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        s: &Sprint,
    ) -> Result<(), RepoError> {
        save_sprint(tx, s).await
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
        insert_goal(tx, sprint_id, outcome, check, owner, notes, idem, now).await
    }
    async fn goal_for_update(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<Goal, RepoError> {
        goal_for_update(tx, id).await
    }
    async fn get_goal_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<Goal, RepoError> {
        get_goal_in(tx, id).await
    }
    async fn save_goal(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        g: &Goal,
    ) -> Result<(), RepoError> {
        save_goal(tx, g).await
    }
    async fn delete_goal(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: i64,
    ) -> Result<(), RepoError> {
        delete_goal(tx, id).await
    }
    async fn goals_of_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        sprint_ids: &[i64],
    ) -> Result<Vec<Goal>, RepoError> {
        goals_of_in(tx, sprint_ids).await
    }
    async fn goals_by_ids_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        ids: &[i64],
    ) -> Result<Vec<Goal>, RepoError> {
        goals_by_ids_in(tx, ids).await
    }
    async fn insert_goal_link(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        goal_id: i64,
        item: &Key,
        by: &str,
        now: DateTime<Utc>,
    ) -> Result<bool, RepoError> {
        insert_goal_link(tx, goal_id, item, by, now).await
    }
    async fn delete_goal_link(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        goal_id: i64,
        item: &Key,
    ) -> Result<bool, RepoError> {
        delete_goal_link(tx, goal_id, item).await
    }
    async fn links_of_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        goal_ids: &[i64],
    ) -> Result<Vec<GoalLink>, RepoError> {
        links_of_in(tx, goal_ids).await
    }
}

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
async fn sprint_in_state(tx: Tx<'_, '_>, state: SprintState) -> Result<Option<Sprint>, RepoError> {
    let row = sqlx::query(&format!(
        "SELECT {SPRINT_COLS} FROM sprints WHERE state = $1"
    ))
    .bind(state_str(state))
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.as_ref().map(sprint_from).transpose()?)
}

async fn sprint_for_update(tx: Tx<'_, '_>, id: i64) -> Result<Sprint, RepoError> {
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

async fn get_sprint_in(tx: Tx<'_, '_>, id: i64) -> Result<Sprint, RepoError> {
    let row = sqlx::query(&format!("SELECT {SPRINT_COLS} FROM sprints WHERE id = $1"))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?;
    row.as_ref()
        .map(sprint_from)
        .transpose()?
        .ok_or(RepoError::NotFound)
}

async fn sprints_in(tx: Tx<'_, '_>) -> Result<Vec<Sprint>, RepoError> {
    let rows = sqlx::query(&format!(
        "SELECT {SPRINT_COLS} FROM sprints ORDER BY id DESC"
    ))
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.iter().map(sprint_from).collect::<Result<_, _>>()?)
}

async fn planned_ends_in(tx: Tx<'_, '_>) -> Result<Vec<Sprint>, RepoError> {
    let rows = sqlx::query(&format!(
        "SELECT {SPRINT_COLS} FROM sprints WHERE state IN ('active', 'draft')"
    ))
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.iter().map(sprint_from).collect::<Result<_, _>>()?)
}

async fn insert_draft(
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

async fn save_sprint(tx: Tx<'_, '_>, s: &Sprint) -> Result<(), RepoError> {
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
async fn insert_goal(
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

async fn goal_for_update(tx: Tx<'_, '_>, id: i64) -> Result<Goal, RepoError> {
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

async fn get_goal_in(tx: Tx<'_, '_>, id: i64) -> Result<Goal, RepoError> {
    let row = sqlx::query(&format!("SELECT {GOAL_COLS} FROM goals WHERE id = $1"))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?;
    row.as_ref()
        .map(goal_from)
        .transpose()?
        .ok_or(RepoError::NotFound)
}

async fn save_goal(tx: Tx<'_, '_>, g: &Goal) -> Result<(), RepoError> {
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

async fn delete_goal(tx: Tx<'_, '_>, id: i64) -> Result<(), RepoError> {
    let res = sqlx::query("DELETE FROM goals WHERE id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    if res.rows_affected() == 0 {
        return Err(RepoError::NotFound);
    }
    Ok(())
}

async fn goals_of_in(tx: Tx<'_, '_>, sprint_ids: &[i64]) -> Result<Vec<Goal>, RepoError> {
    let rows = sqlx::query(&format!(
        "SELECT {GOAL_COLS} FROM goals WHERE sprint_id = ANY($1) ORDER BY sprint_id, id"
    ))
    .bind(sprint_ids)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.iter().map(goal_from).collect::<Result<_, _>>()?)
}

async fn goals_by_ids_in(tx: Tx<'_, '_>, ids: &[i64]) -> Result<Vec<Goal>, RepoError> {
    let rows = sqlx::query(&format!(
        "SELECT {GOAL_COLS} FROM goals WHERE id = ANY($1) ORDER BY id"
    ))
    .bind(ids)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.iter().map(goal_from).collect::<Result<_, _>>()?)
}

async fn insert_goal_link(
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

async fn delete_goal_link(tx: Tx<'_, '_>, goal_id: i64, item: &Key) -> Result<bool, RepoError> {
    let res = sqlx::query("DELETE FROM goal_links WHERE goal_id = $1 AND item_key = $2")
        .bind(goal_id)
        .bind(item.to_string())
        .execute(&mut **tx)
        .await?;
    Ok(res.rows_affected() == 1)
}

async fn links_of_in(tx: Tx<'_, '_>, goal_ids: &[i64]) -> Result<Vec<GoalLink>, RepoError> {
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
