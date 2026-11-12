use super::*;
use crate::domain::queue::LineageItem;

#[async_trait]
impl PlanningReads for PgPlanningRepo {
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
    async fn search_items(
        &self,
        text: &str,
        exact: Option<&Key>,
        limit: i64,
    ) -> Result<Vec<SearchRow>, RepoError> {
        let pattern = like_pattern(text);
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
    async fn story_counts_under_initiative_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        k: &Key,
    ) -> Result<StoryCounts, RepoError> {
        story_counts_on(&mut **tx, k).await
    }
    async fn story_counts_by_initiative_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<HashMap<Key, StoryCounts>, RepoError> {
        let rows = sqlx::query("SELECT e.initiative_key, count(*) FILTER (WHERE s.resolution = 'completed') AS completed, count(*) FILTER (WHERE s.resolution = 'dropped') AS dropped, count(*) AS total FROM stories s JOIN epics e ON e.key = s.epic_key WHERE e.initiative_key IS NOT NULL GROUP BY e.initiative_key")
            .fetch_all(&mut **tx)
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
    async fn list_decisions_under_initiative(
        &self,
        initiative: &Key,
    ) -> Result<Vec<RequiredDecision>, RepoError> {
        decisions_under_initiative(&self.pool, initiative).await
    }
    async fn list_decisions_under_initiative_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        initiative: &Key,
    ) -> Result<Vec<RequiredDecision>, RepoError> {
        decisions_under_initiative(&mut **tx, initiative).await
    }
    async fn active_goals_of_stories(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        stories: &[Key],
    ) -> Result<HashMap<Key, Vec<(i64, String, reeve_protocol::GoalVia)>>, RepoError> {
        goals_of_stories(tx, stories, "active").await
    }
    async fn draft_goals_of_stories(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        stories: &[Key],
    ) -> Result<HashMap<Key, Vec<(i64, String, reeve_protocol::GoalVia)>>, RepoError> {
        goals_of_stories(tx, stories, "draft").await
    }
    async fn decision_lineage_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        items: &[Key],
    ) -> Result<HashMap<Key, Vec<LineageItem>>, RepoError> {
        decision_lineage(tx, items).await
    }
    async fn sprint_story_times(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        linked: &[Key],
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<StoryTimes>, RepoError> {
        sprint_story_times(tx, linked, from, to).await
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
    async fn attachments_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<crate::domain::attachment::Attachment>, RepoError> {
        super::attachments::claimed_in(tx).await
    }
    async fn urgency_lineage_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        epics: &[Key],
        initiatives: &[Key],
    ) -> Result<Lineage, RepoError> {
        urgency_lineage_in(tx, epics, initiatives).await
    }
    async fn promise_facts_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        stories: &[Key],
        epics: &[Key],
    ) -> Result<PromiseFacts, RepoError> {
        promise_facts_in(tx, stories, epics).await
    }
    async fn promised_between_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        after: Option<NaiveDate>,
        upto: NaiveDate,
    ) -> Result<Vec<PromisedStoryRow>, RepoError> {
        promised_between_in(tx, after, upto).await
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

/// The goals of the sprint in `state` each story serves, in one statement.
async fn goals_of_stories(
    tx: Tx<'_, '_>,
    stories: &[Key],
    state: &str,
) -> Result<HashMap<Key, Vec<(i64, String, GoalVia)>>, RepoError> {
    let keys: Vec<String> = stories.iter().map(|k| k.to_string()).collect();
    let rows = sqlx::query(
        "SELECT s.key, g.id, g.outcome, CASE gl.item_key WHEN s.key THEN 'story' WHEN s.epic_key THEN 'epic' ELSE 'initiative' END AS via \
         FROM stories s LEFT JOIN epics e ON e.key = s.epic_key \
         JOIN goal_links gl ON gl.item_key IN (s.key, s.epic_key, e.initiative_key) \
         JOIN goals g ON g.id = gl.goal_id JOIN sprints sp ON sp.id = g.sprint_id AND sp.state = $2 \
         WHERE s.key = ANY($1) \
         ORDER BY s.key, CASE gl.item_key WHEN s.key THEN 0 WHEN s.epic_key THEN 1 ELSE 2 END, g.id",
    )
    .bind(&keys)
    .bind(state)
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

/// Each item's lineage, nearest first, in one statement: an epic, then its initiative; an initiative alone; a story
/// starts at its epic (never itself). Each row carries the initiative's horizon (an epic without one: None).
async fn decision_lineage(
    tx: Tx<'_, '_>,
    items: &[Key],
) -> Result<HashMap<Key, Vec<LineageItem>>, RepoError> {
    let keys: Vec<String> = items.iter().map(|k| k.to_string()).collect();
    let rows = sqlx::query(
        "WITH k AS (SELECT unnest($1::text[]) AS key) \
         SELECT k.key AS item, x.key, x.level, x.marked, x.horizon FROM k \
         JOIN LATERAL ( \
           SELECT e.key, 'epic' AS level, e.discovery_at IS NOT NULL AS marked, i.horizon, 0 AS ord \
             FROM epics e LEFT JOIN initiatives i ON i.key = e.initiative_key \
            WHERE e.key = k.key OR e.key = (SELECT s.epic_key FROM stories s WHERE s.key = k.key) \
           UNION ALL \
           SELECT i.key, 'initiative', i.discovery_at IS NOT NULL, i.horizon, 1 \
             FROM initiatives i \
            WHERE i.key = k.key \
               OR i.key = (SELECT e.initiative_key FROM epics e WHERE e.key = k.key) \
               OR i.key = (SELECT e.initiative_key FROM epics e JOIN stories s ON s.epic_key = e.key WHERE s.key = k.key) \
         ) x ON true \
         ORDER BY length(k.key), k.key, x.ord",
    )
    .bind(&keys)
    .fetch_all(&mut **tx)
    .await?;
    let mut out: HashMap<Key, Vec<LineageItem>> = HashMap::new();
    for r in rows {
        let level = match r.get::<String, _>("level").as_str() {
            "epic" => ItemType::Epic,
            "initiative" => ItemType::Initiative,
            other => return Err(bad("lineage level", other).into()),
        };
        out.entry(key(r.get("item")))
            .or_default()
            .push(LineageItem {
                key: key(r.get("key")),
                level,
                discovery_marked: r.get("marked"),
                initiative_horizon: r.get("horizon"),
            });
    }
    Ok(out)
}

async fn sprint_story_times(
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

async fn urgency_lineage_in(
    tx: Tx<'_, '_>,
    epics: &[Key],
    initiatives: &[Key],
) -> Result<Lineage, RepoError> {
    if epics.is_empty() && initiatives.is_empty() {
        return Ok(Lineage::default());
    }
    let e: Vec<String> = epics.iter().map(Key::to_string).collect();
    let i: Vec<String> = initiatives.iter().map(Key::to_string).collect();
    let epic_rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT key, initiative_key FROM epics WHERE key = ANY($1) OR initiative_key = ANY($2)",
    )
    .bind(&e)
    .bind(&i)
    .fetch_all(&mut **tx)
    .await?;
    let story_rows: Vec<(String, String, bool)> = sqlx::query_as(
        "SELECT s.key, s.epic_key, s.delivery = 'done' FROM stories s
          WHERE s.epic_key = ANY($1)
             OR s.epic_key IN (SELECT key FROM epics WHERE initiative_key = ANY($2))",
    )
    .bind(&e)
    .bind(&i)
    .fetch_all(&mut **tx)
    .await?;
    Ok(Lineage {
        epics: epic_rows
            .into_iter()
            .map(|(k, i)| (key(&k), i.map(|i| key(&i))))
            .collect(),
        stories: story_rows
            .into_iter()
            .map(|(s, e, d)| (key(&s), key(&e), d))
            .collect(),
    })
}

fn story_promise_row(r: &sqlx::postgres::PgRow) -> Result<StoryPromiseRow, sqlx::Error> {
    let epic = match (
        r.try_get::<Option<String>, _>("epic_key")?,
        promise_cols(r, "e_")?,
    ) {
        (Some(k), Some(p)) => Some((key(&k), p)),
        _ => None,
    };
    Ok(StoryPromiseRow {
        own: promise_cols(r, "p_")?,
        epic,
        done_at: r.try_get("done_at")?,
    })
}

/// Story and epic promise facts in one `UNION ALL` statement; `kind` tells the rows apart.
async fn promise_facts_in(
    tx: Tx<'_, '_>,
    stories: &[Key],
    epics: &[Key],
) -> Result<PromiseFacts, RepoError> {
    if stories.is_empty() && epics.is_empty() {
        return Ok(PromiseFacts::default());
    }
    let s: Vec<String> = stories.iter().map(Key::to_string).collect();
    let e: Vec<String> = epics.iter().map(Key::to_string).collect();
    let rows = sqlx::query(
        "SELECT 'story' AS kind, s.key, s.epic_key, s.done_at,
                s.promised_on AS p_on, s.promised_by AS p_by, s.promised_at AS p_at, s.promised_to AS p_to,
                e.promised_on AS e_on, e.promised_by AS e_by, e.promised_at AS e_at, e.promised_to AS e_to
           FROM stories s LEFT JOIN epics e ON e.key = s.epic_key AND e.promised_on IS NOT NULL
          WHERE s.key = ANY($1)
         UNION ALL
         SELECT 'epic', key, NULL::text, NULL::timestamptz,
                promised_on, promised_by, promised_at, promised_to,
                NULL::date, NULL::text, NULL::timestamptz, NULL::text
           FROM epics WHERE key = ANY($2)",
    )
    .bind(&s)
    .bind(&e)
    .fetch_all(&mut **tx)
    .await?;
    let mut facts = PromiseFacts::default();
    for r in &rows {
        let k = key(r.get::<&str, _>("key"));
        match r.get::<&str, _>("kind") {
            "story" => {
                facts.stories.insert(k, story_promise_row(r)?);
            }
            "epic" => {
                if let Some(p) = promise_cols(r, "p_")? {
                    facts.epics.insert(k, p);
                }
            }
            other => return Err(bad("promise fact kind", other).into()),
        }
    }
    Ok(facts)
}

async fn promised_between_in(
    tx: Tx<'_, '_>,
    after: Option<NaiveDate>,
    upto: NaiveDate,
) -> Result<Vec<PromisedStoryRow>, RepoError> {
    // Two branches, each a range on one partial index (`stories_promised_idx`, `epics_promised_idx`): the stories whose
    // own date is in range, and the stories under an epic whose date is in range. `UNION` drops the story both find
    // (the rows are identical). An `OR` across the join would scan every story.
    let rows = sqlx::query(
        "SELECT * FROM (
           SELECT s.key, s.title, s.delivery, s.resolution, s.epic_key, s.done_at,
                  s.promised_on AS p_on, s.promised_by AS p_by, s.promised_at AS p_at, s.promised_to AS p_to,
                  e.promised_on AS e_on, e.promised_by AS e_by, e.promised_at AS e_at, e.promised_to AS e_to
             FROM stories s LEFT JOIN epics e ON e.key = s.epic_key AND e.promised_on IS NOT NULL
            WHERE s.promised_on > COALESCE($1::date, '-infinity'::date) AND s.promised_on <= $2
           UNION
           SELECT s.key, s.title, s.delivery, s.resolution, s.epic_key, s.done_at,
                  s.promised_on, s.promised_by, s.promised_at, s.promised_to,
                  e.promised_on, e.promised_by, e.promised_at, e.promised_to
             FROM epics e JOIN stories s ON s.epic_key = e.key
            WHERE e.promised_on > COALESCE($1::date, '-infinity'::date) AND e.promised_on <= $2
         ) u ORDER BY length(u.key), u.key",
    )
    .bind(after)
    .bind(upto)
    .fetch_all(&mut **tx)
    .await?;
    rows.iter()
        .map(|r| {
            Ok(PromisedStoryRow {
                key: key(r.get::<&str, _>("key")),
                title: r.get("title"),
                delivery: delivery_of(r.get("delivery"))?,
                resolution: resolution_of(r.get::<Option<String>, _>("resolution").as_deref())?,
                row: story_promise_row(r)?,
            })
        })
        .collect()
}
