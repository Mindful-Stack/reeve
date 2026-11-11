use super::dto::urgent_fields;
use super::epics::epic_dto_of;
use super::import::{Claim, with_ref};
use super::*;
use crate::domain::{items::*, readiness::compute_readiness, urgency::Urgency};
use reeve_protocol::{
    DeliveryState, DoneFilter, GoalContextDto, GoalRefDto, ItemType, Key, Kind, Readiness,
    StoryContextDto, StoryDto, StorySort, StoryWithReadiness,
};
use sqlx::{Postgres, Transaction};

impl Handlers {
    pub async fn create_story(
        &self,
        c: &Ctx,
        idem: &str,
        title: &str,
        kind: Kind,
        epic: Option<Key>,
        description: &str,
        criteria: Vec<String>,
        labels: Vec<String>,
        external_ref: Option<String>,
        owner: Option<String>,
    ) -> Result<Story, HandlerError> {
        self.create_story_reporting(
            c,
            idem,
            title,
            kind,
            epic,
            description,
            criteria,
            labels,
            external_ref,
            owner,
            DeliveryState::Todo,
        )
        .await
        .map(|(x, _)| x)
    }

    /// `create_story`, and whether this call inserted it (false when its idempotency key already had one). A `delivery`
    /// other than todo grandfathers a migrated story (a plan's): it is inserted in that state and logged
    /// `imported` with its reference, exactly as the importer logs it; the gate is never consulted.
    pub(crate) async fn create_story_reporting(
        &self,
        c: &Ctx,
        idem: &str,
        title: &str,
        kind: Kind,
        epic: Option<Key>,
        description: &str,
        criteria: Vec<String>,
        labels: Vec<String>,
        external_ref: Option<String>,
        owner: Option<String>,
        delivery: DeliveryState,
    ) -> Result<(Story, bool), HandlerError> {
        if delivery != DeliveryState::Todo
            && (external_ref.is_none() || delivery == DeliveryState::Done)
        {
            return Err(HandlerError::Validation(
                "only a migrated story (with an external_ref) is created in a state, and never done".into(),
            ));
        }
        if let Some(k) = self
            .ports
            .keys
            .find_by_idempotency(ItemType::Story, idem)
            .await?
        {
            return Ok((self.ports.stories.get_story(&k).await?, false));
        }
        if let Some(ek) = &epic {
            self.ports.epics.get_epic(ek).await?;
        }
        let mut tx = self.pool.begin().await?;
        let r = match self
            .claim_ref(&mut tx, ItemType::Story, idem, external_ref)
            .await?
        {
            Claim::Free(r) => r,
            Claim::Retried(k) => {
                tx.rollback().await?;
                return Ok((self.ports.stories.get_story(&k).await?, false));
            }
        };
        let key = self
            .ports
            .keys
            .next_key(&mut tx, &self.settings.key_prefix)
            .await?;
        let s = Story::new(
            key,
            title,
            kind,
            epic,
            description,
            criteria,
            owner.map(|e| reeve_protocol::normalise_email(&e)),
            labels,
        )?
        .with_external_ref(r)
        .grandfathered(delivery);
        if !self
            .ports
            .stories
            .insert_story(&mut tx, &s, idem, c.now)
            .await?
        {
            tx.rollback().await?;
            let k = self
                .ports
                .keys
                .find_by_idempotency(ItemType::Story, idem)
                .await?
                .ok_or(HandlerError::NotFound)?;
            return Ok((self.ports.stories.get_story(&k).await?, false));
        }
        let (action, payload) = match s.external_ref() {
            Some(r) if delivery != DeliveryState::Todo => {
                ("imported", serde_json::json!({"external_ref": r}))
            }
            r => (
                "created",
                with_ref(serde_json::json!({"title": s.title(), "kind": s.kind()}), r),
            ),
        };
        self.activity
            .record(
                &mut tx,
                self.rec(c, "story", &s.key().to_string(), action, payload),
            )
            .await?;
        tx.commit().await?;
        Ok((s, true))
    }

    pub async fn edit_story(
        &self,
        c: &Ctx,
        key: &Key,
        version: i64,
        title: Option<String>,
        description: Option<String>,
        kind: Option<Kind>,
        epic: Option<Option<Key>>,
        criteria: Option<Vec<Criterion>>,
        owner: Option<Option<String>>,
        labels: Option<Vec<String>>,
        rank: Option<i32>,
        external_ref: Option<String>,
    ) -> Result<Story, HandlerError> {
        let cur = self.ports.stories.get_story(key).await?;
        let title = title.unwrap_or_else(|| cur.title().into());
        check_title(&title)?;
        let title = title.trim().to_string();
        let epic = epic.unwrap_or(cur.epic().cloned());
        if let Some(ek) = &epic {
            self.ports.epics.get_epic(ek).await?;
        }
        let mut criteria = criteria.unwrap_or_else(|| cur.criteria().to_vec());
        for x in criteria.iter_mut() {
            x.text = x.text.trim().to_string();
        }
        if criteria.iter().any(|x| x.text.is_empty()) {
            return Err(HandlerError::Validation(
                "a criterion must not be blank".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        let (r, set) = self
            .set_ref(&mut tx, key, cur.external_ref(), external_ref)
            .await?;
        let next = Story::from_persisted(
            cur.key().clone(),
            title,
            kind.unwrap_or(cur.kind()),
            epic,
            description.unwrap_or_else(|| cur.description().into()),
            criteria,
            owner
                .map(|o| o.map(|e| reeve_protocol::normalise_email(&e)))
                .unwrap_or(cur.owner().map(String::from)),
            labels.unwrap_or_else(|| cur.labels().to_vec()),
            cur.rank(),
            cur.delivery(),
            cur.resolution(),
            r,
            cur.version(),
        );
        self.ports
            .stories
            .update_story(&mut tx, &next, rank, version, c.now)
            .await?;
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "story",
                    &key.to_string(),
                    "edited",
                    with_ref(serde_json::json!({}), set.as_deref()),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(self.ports.stories.get_story(key).await?)
    }

    pub async fn get_story(&self, key: &Key) -> Result<Story, HandlerError> {
        Ok(self.ports.stories.get_story(key).await?)
    }

    /// The story imported from `external_ref` (a Jira delivery key such as `SCRUM-11`), if any. A plain lookup of the
    /// reference alone; the webhook's legacy branch names go through `find_story_by_branch_token`, which checks native
    /// keys first.
    pub async fn find_story_by_external_ref(
        &self,
        external_ref: &str,
    ) -> Result<Option<Key>, HandlerError> {
        Ok(self
            .ports
            .keys
            .find_by_external_ref(ItemType::Story, external_ref)
            .await?)
    }

    /// The story a key-shaped branch token of a prefix other than the instance one names (`OLD-12`, `SCRUM-11`): the
    /// native story that has it as its key first (an item issued before a `key_prefix` change keeps its key), then,
    /// only when `legacy` (the webhook's `legacy_prefixes` allowlist), the story imported from it as an `external_ref`.
    /// Epics and initiatives are never matched.
    pub async fn find_story_by_branch_token(
        &self,
        token: &str,
        legacy: bool,
    ) -> Result<Option<Key>, HandlerError> {
        // Only called for tokens of another prefix (resolve_branch returns instance keys itself); an instance-prefix
        // token must still be checked, never returned unchecked.
        if let Ok(k) = token.parse::<Key>()
            && k.prefix() == self.settings.key_prefix
        {
            return Ok(self
                .ports
                .keys
                .key_exists(ItemType::Story, &k)
                .await?
                .then_some(k));
        }
        match self
            .resolve(token, Scope::One(ItemType::Story), legacy)
            .await
        {
            Ok((_, k)) => Ok(Some(k)),
            Err(HandlerError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The story list for `f`, read in one `REPEATABLE READ READ ONLY` snapshot: stories, readiness facts, blockers,
    /// pull requests and urgency, in a number of statements that does not depend on the number of stories.
    /// Readiness is computed for every story whatever its delivery state, but `ready_only` means pickable work: ready
    /// stories that are still in todo.
    ///
    /// `text` is the caller's raw text: it is trimmed and must be 1..=200 characters, and the key it names
    /// (`key_named_by`) is derived here, so `f.text` is ignored and replaced. `StorySort::Relevance` needs text. Both
    /// are validated before anything is read, so a filter that would list nothing is still refused when invalid.
    /// `initiative` narrows `f.epics` to that initiative's epics; `f.urgent` keeps the rows whose computed urgency has
    /// a source (kb/ADR-0021 R4).
    pub async fn list_stories(
        &self,
        f: &StoryFilter,
        text: Option<&str>,
        initiative: Option<&Key>,
        ready_only: bool,
        done: DoneFilter,
        now: DateTime<Utc>,
    ) -> Result<Vec<StoryWithReadiness>, HandlerError> {
        let mut f = f.clone();
        f.text = match text.map(str::trim) {
            None => None,
            Some(t) if t.is_empty() || t.chars().count() > 200 => {
                return Err(HandlerError::Validation(
                    "story text must be 1..=200 characters".into(),
                ));
            }
            Some(t) => Some(StoryText {
                key: self.key_named_by(t),
                text: t.to_string(),
            }),
        };
        if f.sort == StorySort::Relevance && f.text.is_none() {
            return Err(HandlerError::Validation(
                "sort=relevance needs text (q)".into(),
            ));
        }
        let done_window = match done {
            DoneFilter::All => DoneWindow::All,
            // An absurd window reaches before the oldest representable date: list everything rather than panic.
            DoneFilter::Recent => chrono::Duration::try_days(self.settings.done_window_days.into())
                .and_then(|window| now.checked_sub_signed(window))
                .map_or(DoneWindow::All, DoneWindow::Since),
            DoneFilter::None => DoneWindow::None,
        };
        f.owner = f.owner.map(|o| reeve_protocol::normalise_email(&o));
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = async {
            if let Some(i) = initiative {
                let under = self.ports.epics.epic_keys_in(&mut tx, i).await?;
                f.epics = Some(match f.epics.take() {
                    Some(given) => given.into_iter().filter(|k| under.contains(k)).collect(),
                    None => under,
                });
            }
            let urg = self.urgency_in(&mut tx, now).await?;
            self.list_stories_in(&mut tx, &f, ready_only, done_window, &urg)
                .await
        }
        .await;
        end_read(tx, out).await
    }

    /// The story list for `f` on the caller's transaction, with `urg` as every row's urgency. `f.goal` and `f.urgent`
    /// filter after the read; `f.urgent` keeps the rows `urg` shows urgent, so a default `Urgency` (a caller that
    /// computed none) yields nothing under `f.urgent` rather than an unfiltered list.
    pub(super) async fn list_stories_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        f: &StoryFilter,
        ready_only: bool,
        done: DoneWindow,
        urg: &Urgency,
    ) -> Result<Vec<StoryWithReadiness>, HandlerError> {
        let stories = self.ports.stories.list_stories(tx, f, done).await?;
        let keys: Vec<Key> = stories.iter().map(|s| s.key().clone()).collect();
        let mut inputs = self.ports.reads.readiness_inputs(tx, &keys).await?;
        for i in inputs.values_mut() {
            self.mark_configured(i);
        }
        let mut kept = Vec::new();
        let mut readiness = Vec::new();
        for s in stories {
            // Cannot be missing: the inputs are read in the snapshot that listed the story.
            let input = inputs.remove(s.key()).ok_or(HandlerError::NotFound)?;
            let r = compute_readiness(&input);
            if !ready_only || (r.ready && s.delivery() == DeliveryState::Todo) {
                kept.push(s);
                readiness.push(r);
            }
        }
        let mut out: Vec<StoryWithReadiness> = self
            .story_dtos_in(tx, &kept, urg)
            .await?
            .into_iter()
            .zip(readiness)
            .map(|(story, readiness)| StoryWithReadiness { story, readiness })
            .collect();
        // Goal membership is read with the DTOs (one statement), so it filters after them; it only drops rows and never
        // changes what a kept row shows (a started story still lists under its goal).
        if let Some(g) = f.goal {
            out.retain(|s| match g {
                GoalFilter::Id(id) => s.story.goals.iter().any(|x| x.id == id),
                GoalFilter::Any => !s.story.goals.is_empty(),
                GoalFilter::None => s.story.goals.is_empty(),
            });
        }
        // Urgency is computed once per snapshot (`urgency_in`) and shown on every row; the filter keeps exactly the
        // rows shown urgent rather than restating kb/ADR-0017 R3 in SQL.
        if f.urgent {
            out.retain(|s| !s.story.urgent_sources.is_empty());
        }
        Ok(out)
    }

    pub async fn readiness(&self, key: &Key) -> Result<Readiness, HandlerError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let r = self.readiness_in(&mut tx, key).await;
        end_read(tx, r).await
    }

    /// Readiness of one story on the caller's transaction; NotFound when it has no row.
    pub(crate) async fn readiness_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        key: &Key,
    ) -> Result<Readiness, HandlerError> {
        let mut m = self
            .ports
            .reads
            .readiness_inputs(tx, std::slice::from_ref(key))
            .await?;
        let mut input = m.remove(key).ok_or(HandlerError::NotFound)?;
        self.mark_configured(&mut input);
        Ok(compute_readiness(&input))
    }

    /// Readiness inputs come from the repo, which does not know the settings: each decision's record is marked by
    /// whether its repository is configured.
    pub(super) fn mark_configured(&self, i: &mut crate::domain::readiness::ReadinessInput) {
        let repos = &self.settings.repos;
        for d in i
            .own_decisions
            .iter_mut()
            .chain(i.ancestors.iter_mut().flat_map(|a| a.decisions.iter_mut()))
        {
            d.repo_configured = d.record_repo.as_ref().is_none_or(|r| repos.contains(r));
        }
    }

    /// Places a todo story before `before` (another todo story), or at the end of the todo column. One transaction at the
    /// caller's version: the whole column is locked and renumbered 0..n-1 in one statement, only the moved story's version
    /// is bumped (rank is ordering metadata, so a neighbour's open editor keeps its base), and one `reranked` entry with the
    /// story's old and new 0-based positions is written last. A placement that changes nothing writes nothing.
    pub async fn rank_story(
        &self,
        c: &Ctx,
        key: &Key,
        version: i64,
        before: Option<&Key>,
    ) -> Result<Story, HandlerError> {
        if before == Some(key) {
            return Err(HandlerError::Validation(
                "a story cannot be ranked before itself".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        let column = self.ports.stories.todo_column_for_update(&mut tx).await?;
        let cur = self
            .ports
            .stories
            .get_story_for_update(&mut tx, key)
            .await?;
        if cur.version() != version {
            tx.rollback().await?;
            return Err(HandlerError::Conflict {
                current: cur.version(),
            });
        }
        let Some(from) = column.iter().position(|k| k == key) else {
            tx.rollback().await?;
            return Err(HandlerError::Validation(
                "only todo stories are ranked".into(),
            ));
        };
        let mut order: Vec<Key> = column.iter().filter(|k| *k != key).cloned().collect();
        let at = match before {
            None => order.len(),
            Some(b) => match order.iter().position(|k| k == b) {
                Some(i) => i,
                None => {
                    tx.rollback().await?;
                    return Err(HandlerError::Validation(
                        "a story can only be ranked before another todo story".into(),
                    ));
                }
            },
        };
        order.insert(at, key.clone());
        if at == from {
            tx.rollback().await?;
            return Ok(cur);
        }
        let ranks: Vec<(Key, i32)> = order
            .into_iter()
            .enumerate()
            .map(|(i, k)| (k, i as i32))
            .collect();
        self.ports.stories.set_ranks(&mut tx, &ranks).await?;
        self.ports
            .stories
            .touch_story(&mut tx, key, version, c.now)
            .await?;
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "story",
                    &key.to_string(),
                    "reranked",
                    serde_json::json!({"from": from, "to": at}),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(self.ports.stories.get_story(key).await?)
    }

    pub async fn add_block(&self, c: &Ctx, story: &Key, blocker: &Key) -> Result<(), HandlerError> {
        self.add_block_reporting(c, story, blocker)
            .await
            .map(|_| ())
    }

    /// `add_block`, and whether this call added the edge (false when it was already there).
    pub(crate) async fn add_block_reporting(
        &self,
        c: &Ctx,
        story: &Key,
        blocker: &Key,
    ) -> Result<bool, HandlerError> {
        if story == blocker {
            return Err(HandlerError::Validation(
                "a story cannot block itself".into(),
            ));
        }
        self.ports.stories.get_story(story).await?;
        self.ports.stories.get_story(blocker).await?;
        let mut tx = self.pool.begin().await?;
        if !self
            .ports
            .stories
            .add_block(&mut tx, story, blocker)
            .await?
        {
            tx.rollback().await?;
            return Ok(false);
        }
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "story",
                    &story.to_string(),
                    "blocked_by",
                    serde_json::json!({"blocker": blocker}),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Removes "`blocker` blocks `story`". Both stories must exist (NotFound otherwise, as for `add_block`); an edge that does
    /// not exist is a no-op that writes nothing. Readiness is computed from facts on read (kb/ADR-0018 R1), so the next read
    /// drops the reason; no story version moves, as with `add_block`.
    pub async fn remove_block(
        &self,
        c: &Ctx,
        story: &Key,
        blocker: &Key,
    ) -> Result<(), HandlerError> {
        self.ports.stories.get_story(story).await?;
        self.ports.stories.get_story(blocker).await?;
        let mut tx = self.pool.begin().await?;
        if !self
            .ports
            .stories
            .remove_block(&mut tx, story, blocker)
            .await?
        {
            tx.rollback().await?;
            return Ok(());
        }
        self.activity
            .record(
                &mut tx,
                self.rec(
                    c,
                    "story",
                    &story.to_string(),
                    "unblocked",
                    serde_json::json!({"blocker": blocker}),
                ),
            )
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// One story's DTO in its own read-only snapshot (the single-story routes): the row, its assembly and its urgency
    /// all read there. NotFound when no story has the key.
    pub async fn story_dto(&self, key: &Key, now: DateTime<Utc>) -> Result<StoryDto, HandlerError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = async {
            let urg = self.urgency_in(&mut tx, now).await?;
            let stories = self
                .ports
                .reads
                .stories_by_keys(&mut tx, std::slice::from_ref(key))
                .await?;
            self.story_dtos_in(&mut tx, &stories, &urg).await
        }
        .await;
        end_read(tx, out).await?.pop().ok_or(HandlerError::NotFound)
    }

    /// The one place a `StoryDto` is built: blockers, pull requests and the active sprint's goals for all the stories in
    /// one statement each, on the caller's transaction, in the order of `stories`, with the urgency the caller read on
    /// that same transaction. Callers pass distinct stories: a repeated story gets its blockers and pull requests only on
    /// its first occurrence.
    pub async fn story_dtos_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        stories: &[Story],
        urg: &Urgency,
    ) -> Result<Vec<StoryDto>, HandlerError> {
        if stories.is_empty() {
            return Ok(Vec::new());
        }
        let keys: Vec<Key> = stories.iter().map(|s| s.key().clone()).collect();
        let mut blockers = self.ports.reads.blockers_of_many(tx, &keys).await?;
        let mut prs = self.ports.reads.pull_requests_for_many(tx, &keys).await?;
        let mut goals = self.ports.reads.active_goals_of_stories(tx, &keys).await?;
        Ok(stories
            .iter()
            .map(|s| {
                let mut dto = StoryDto::from(s);
                dto.blocked_by = blockers.remove(s.key()).unwrap_or_default();
                dto.prs = prs
                    .remove(s.key())
                    .unwrap_or_default()
                    .iter()
                    .map(Into::into)
                    .collect();
                dto.goals = goals
                    .remove(s.key())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(id, outcome, via)| GoalRefDto { id, outcome, via })
                    .collect();
                (dto.urgent_mark, dto.urgent_sources) = urgent_fields(urg, s.key());
                dto
            })
            .collect())
    }

    /// The story, its ancestors, open decisions, blockers, comments, readiness and the active sprint's goals it serves,
    /// all from one `REPEATABLE READ READ ONLY` snapshot, in a number of statements that does not depend on the number
    /// of blockers or goals.
    pub async fn story_context(
        &self,
        key: &Key,
        now: DateTime<Utc>,
    ) -> Result<StoryContextDto, HandlerError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = async {
            let urg = self.urgency_in(&mut tx, now).await?;
            self.story_context_in(&mut tx, key, &urg).await
        }
        .await;
        end_read(tx, out).await
    }

    pub(super) async fn story_context_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        key: &Key,
        urg: &Urgency,
    ) -> Result<StoryContextDto, HandlerError> {
        let s = self
            .ports
            .reads
            .stories_by_keys(tx, std::slice::from_ref(key))
            .await?
            .pop()
            .ok_or(HandlerError::NotFound)?;
        let epic = match s.epic() {
            Some(k) => Some(self.ports.epics.get_epic_in(tx, k).await?),
            None => None,
        };
        let initiative = match epic.as_ref().and_then(|e| e.initiative()) {
            Some(k) => Some(self.initiative_dto_in(tx, k, urg).await?),
            None => None,
        };
        let mut required_decisions = Vec::new();
        if let Some(i) = &initiative {
            required_decisions.extend(
                self.ports
                    .decisions
                    .list_decisions_on_in(tx, &i.key)
                    .await?,
            );
        }
        if let Some(e) = &epic {
            required_decisions.extend(
                self.ports
                    .decisions
                    .list_decisions_on_in(tx, e.key())
                    .await?,
            );
        }
        required_decisions.extend(self.ports.decisions.list_decisions_on_in(tx, key).await?);
        // A closed decision no longer gates anything; it stays out of the bundle so an agent never reads it as pending.
        required_decisions.retain(|d| d.closed().is_none());
        // Blockers in three statements whatever their number: the edges (in key-number order), the blocker rows (in
        // that order), and one assembly for the story and its blockers together.
        let blocker_keys = self
            .ports
            .reads
            .blockers_of_many(tx, std::slice::from_ref(key))
            .await?
            .remove(key)
            .unwrap_or_default();
        let mut stories = vec![s];
        stories.extend(self.ports.reads.stories_by_keys(tx, &blocker_keys).await?);
        let mut dtos = self.story_dtos_in(tx, &stories, urg).await?.into_iter();
        let story = dtos.next().ok_or(HandlerError::NotFound)?;
        let blockers = dtos.collect();
        let goals = self.goals_context_in(tx, &story.goals).await?;
        Ok(StoryContextDto {
            story,
            epic: epic.as_ref().map(|e| epic_dto_of(e, urg)),
            initiative,
            required_decisions: required_decisions
                .iter()
                .map(reeve_protocol::RequiredDecisionDto::from)
                .collect(),
            blockers,
            comments: self
                .ports
                .comments
                .list_comments_for_story_in(tx, key)
                .await?,
            readiness: self.readiness_in(tx, key).await?,
            goals,
        })
    }

    /// The goals a story serves, in full, with the active sprint's dates: three statements whatever their number (the
    /// goals, their links, the active sprint), none when the story serves no goal.
    async fn goals_context_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        refs: &[GoalRefDto],
    ) -> Result<Vec<GoalContextDto>, HandlerError> {
        if refs.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<i64> = refs.iter().map(|r| r.id).collect();
        let goals = self.ports.sprints.goals_by_ids_in(tx, &ids).await?;
        let links = self.ports.sprints.links_of_in(tx, &ids).await?;
        // The refs name the active sprint's goals, read in this snapshot, so it exists.
        let sprint = self
            .ports
            .sprints
            .sprint_in_state(tx, reeve_protocol::SprintState::Active)
            .await?
            .ok_or(HandlerError::NotFound)?;
        refs.iter()
            .map(|r| {
                let g = goals
                    .iter()
                    .find(|g| g.id == r.id)
                    .ok_or(HandlerError::NotFound)?;
                Ok(GoalContextDto {
                    goal: super::dto::goal_dto(g, &links),
                    starts_on: sprint.starts_on,
                    ends_on: sprint.ends_on,
                    via: r.via,
                })
            })
            .collect()
    }
}
