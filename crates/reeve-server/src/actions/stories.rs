use super::{Acting, FilterRef, resolve_edit, resolve_filter, resolve_opt};
use crate::{error::AppError, state::AppState};
use reeve_planning::{GoalFilter, StoryFilter};
use reeve_protocol::{
    CreateStory, DeliveryState, DoneFilter, EditItem, ItemType, KeyRef, Kind, MoveStory,
    PromisedFilter, StoryContextDto, StoryDto, StorySort, StoryWithReadiness, normalise_email,
};
use serde::Deserialize;

/// The query of `GET /api/v1/stories`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct StoryQuery {
    /// An epic key or external reference. A reference that matches nothing yields an empty list, not 404.
    pub(crate) epic: Option<KeyRef>,
    pub(crate) owner: Option<String>,
    pub(crate) delivery: Option<DeliveryState>,
    /// Items of any kind with no epic that are not done (kb/ADR-0027 R4).
    #[serde(default)]
    pub(crate) untriaged: bool,
    pub(crate) label: Option<String>,
    #[serde(default)]
    pub(crate) ready: bool,
    /// `recent`: done stories only when they entered done within `[board] done_window_days`; `none`: no done
    /// stories; default `all`.
    #[serde(default)]
    pub(crate) done: DoneFilter,
    /// Stories serving the active sprint's goals: a goal id, `any` (serving some goal) or `none` (upkeep). Combines
    /// with every other filter.
    pub(crate) goal: Option<String>,
    /// Text, 1 to 200 characters after trimming: a case-insensitive substring of the title, description or external
    /// reference; a key-shaped text (`12`, `#12`, `abc-12`) also matches that key exactly. The length bounds count
    /// characters of the trimmed text, so padding around 200 characters is accepted.
    #[param(min_length = 1, max_length = 200)]
    pub(crate) q: Option<String>,
    pub(crate) kind: Option<Kind>,
    /// An initiative key or external reference: stories whose epic belongs to it. A reference that matches nothing,
    /// or an initiative with no epics, yields an empty list, not 404.
    pub(crate) initiative: Option<KeyRef>,
    /// Only stories shown as urgent (non-empty `urgent_sources`).
    #[serde(default)]
    pub(crate) urgent: bool,
    /// Only stories whose promise (kb/ADR-0025 R5) is:
    /// - `this_sprint`: overdue, or open and due this sprint;
    /// - `next_sprint`: open and due next sprint;
    /// - `overdue`: overdue;
    /// - `any`: open or overdue.
    ///
    /// Kept, late and dropped promises never match. Combines with every other filter.
    pub(crate) promised: Option<PromisedFilter>,
    /// `rank` (default: delivery state, rank, key number) or `relevance` (needs `q`): the story whose key or external
    /// reference equals `q` first, then title matches, then the rest, each in `rank` order.
    #[serde(default)]
    pub(crate) sort: StorySort,
    /// One page of the list: at most this many stories, 1 to 100, starting at `offset`, cut after every filter and in
    /// the list's order. Without it the whole list is answered. A page of exactly `limit` stories means there may be
    /// more.
    #[param(minimum = 1, maximum = 100)]
    pub(crate) limit: Option<u32>,
    /// Stories to skip before the page, 0 or more; refused without `limit`. An offset at or past the end answers an
    /// empty list.
    #[param(format = Int64, minimum = 0, maximum = 4294967295u32)]
    pub(crate) offset: Option<u32>,
}

/// `goal=<id>|any|none`.
fn goal_filter(raw: Option<&str>) -> Result<Option<GoalFilter>, AppError> {
    Ok(match raw {
        None => None,
        Some("any") => Some(GoalFilter::Any),
        Some("none") => Some(GoalFilter::None),
        Some(id) => Some(GoalFilter::Id(id.parse().map_err(|_| {
            reeve_planning::HandlerError::Validation("goal must be any, none or a goal id".into())
        })?)),
    })
}

/// `limit` and `offset` as the slice to cut from the list, `(skip, take)`; `None` is the whole list. `limit` is 1 to
/// 100 and `offset` is valid only with it.
fn page(limit: Option<u32>, offset: Option<u32>) -> Result<Option<(usize, usize)>, AppError> {
    match (limit, offset) {
        (None, None) => Ok(None),
        (None, Some(_)) => Err(AppError::Validation("offset needs limit".into())),
        (Some(l), _) if !(1..=100).contains(&l) => {
            Err(AppError::Validation("limit must be 1..=100".into()))
        }
        (Some(l), o) => Ok(Some((o.unwrap_or(0) as usize, l as usize))),
    }
}

/// `reeve story ls`: the story list, filtered, the whole of it or one page. Shared by `GET /api/v1/stories` and the
/// MCP tool `list_stories`. `q` is what the route takes: its query.
pub(crate) async fn list_stories(
    state: &AppState,
    who: &Acting<'_>,
    q: &StoryQuery,
) -> Result<Vec<StoryWithReadiness>, AppError> {
    let page = page(q.limit, q.offset)?;
    let goal = goal_filter(q.goal.as_deref())?;
    // A reference that names nothing lists nothing, through the handler, so validation still runs (kb/ADR-0021).
    let mut epics = None;
    let epic = match resolve_filter(&state.handlers, q.epic.as_ref(), ItemType::Epic).await? {
        FilterRef::Nothing => {
            epics = Some(Vec::new());
            None
        }
        FilterRef::Is(k) => Some(k),
        FilterRef::Unset => None,
    };
    let initiative =
        match resolve_filter(&state.handlers, q.initiative.as_ref(), ItemType::Initiative).await? {
            FilterRef::Nothing => {
                epics = Some(Vec::new());
                None
            }
            FilterRef::Is(k) => Some(k),
            FilterRef::Unset => None,
        };
    let f = StoryFilter {
        epic,
        owner: q.owner.clone(),
        delivery: q.delivery,
        untriaged: q.untriaged,
        label: q.label.clone(),
        goal,
        text: None,
        kind: q.kind,
        epics,
        urgent: q.urgent,
        promised: q.promised,
        sort: q.sort,
    };
    let stories = state
        .handlers
        .list_stories(
            &f,
            q.q.as_deref(),
            initiative.as_ref(),
            q.ready,
            q.done,
            who.ctx.now,
        )
        .await?;
    Ok(match page {
        Some((skip, take)) => stories.into_iter().skip(skip).take(take).collect(),
        None => stories,
    })
}

/// `reeve story context <KEY>`: everything an agent needs before starting a story. Shared by
/// `GET /api/v1/stories/{key}/context` and the MCP tool `get_story`. `key` is what the route takes: its path
/// parameter, a story's key or external reference.
pub(crate) async fn story_context(
    state: &AppState,
    who: &Acting<'_>,
    key: &str,
) -> Result<StoryContextDto, AppError> {
    let key = state.handlers.resolve_ref(key, ItemType::Story).await?;
    let mut context = state.handlers.story_context(&key, who.ctx.now).await?;
    for d in &mut context.required_decisions {
        super::browse(state, d);
    }
    Ok(context)
}

/// A story's owner must be an allowed user (allowed by the floor or a database entry): an owner nobody can sign
/// in as would hold work no one can pick up.
fn allowed_owner(
    policy: &reeve_identity::AccessPolicy,
    owner: Option<String>,
) -> Result<Option<String>, AppError> {
    match owner {
        Some(o) if !policy.is_allowed(&o) => Err(reeve_planning::HandlerError::Validation(
            format!("{} is not an allowed user", normalise_email(&o)),
        )
        .into()),
        other => Ok(other),
    }
}

/// `reeve story create`. Shared by `POST /api/v1/stories` and the MCP tool `create_story`. `b` is what the route
/// takes: its body. Answers the story and whether this call created it (false: the idempotency key already had
/// one, and nothing was written).
pub(crate) async fn create_story(
    state: &AppState,
    who: &Acting<'_>,
    b: CreateStory,
) -> Result<(StoryDto, bool), AppError> {
    let idem = who.write_key(&b.idempotency_key)?;
    let epic = resolve_opt(&state.handlers, b.epic.as_ref(), ItemType::Epic).await?;
    let owner = allowed_owner(who.policy, b.owner)?;
    let (story, created) = state
        .handlers
        .create_story_attaching(
            &who.ctx,
            &idem,
            &b.title,
            b.kind,
            epic,
            b.description.as_deref().unwrap_or(""),
            b.criteria,
            b.labels,
            b.external_ref,
            owner,
            &b.attachments,
        )
        .await?;
    Ok((
        state.handlers.story_dto(story.key(), who.ctx.now).await?,
        created,
    ))
}

/// `reeve story show <KEY>`: the story as the API shows it. Shared by `GET /api/v1/stories/{key}` and the MCP tool
/// `edit_story`, which reads the story to merge its criteria and labels into. (The tool `get_story` is
/// `story_context`, not this.) `key` is what the route takes: its path parameter, a story's key or external
/// reference.
pub(crate) async fn read_story(
    state: &AppState,
    who: &Acting<'_>,
    key: &str,
) -> Result<StoryDto, AppError> {
    let key = state.handlers.resolve_ref(key, ItemType::Story).await?;
    Ok(state.handlers.story_dto(&key, who.ctx.now).await?)
}

/// `reeve story edit <KEY>`. Shared by `PATCH /api/v1/stories/{key}` and the MCP tool `edit_story`. `key` and `b`
/// are what the route takes: its path parameter and its body.
pub(crate) async fn edit_story(
    state: &AppState,
    who: &Acting<'_>,
    key: &str,
    b: EditItem,
) -> Result<StoryDto, AppError> {
    let key = state.handlers.resolve_ref(key, ItemType::Story).await?;
    let criteria = b.criteria.map(|v| {
        v.into_iter()
            .map(|c| reeve_planning::Criterion {
                text: c.text,
                done: c.done,
            })
            .collect()
    });
    let epic = resolve_edit(&state.handlers, b.epic, ItemType::Epic).await?;
    let owner = match b.owner {
        Some(Some(o)) => Some(allowed_owner(who.policy, Some(o))?),
        other => other,
    };
    let story = state
        .handlers
        .edit_story_attaching(
            &who.ctx,
            &key,
            b.version,
            b.title,
            b.description,
            b.kind,
            epic,
            criteria,
            owner,
            b.labels,
            b.rank,
            b.external_ref,
            &b.attachments,
        )
        .await?;
    Ok(state.handlers.story_dto(story.key(), who.ctx.now).await?)
}

/// `reeve story mv <KEY>`. Shared by `POST /api/v1/stories/{key}/move` and the MCP tool `move_story`. `key` and `b`
/// are what the route takes: its path parameter and its body. The route passes the override reason an interactive
/// caller sent; the tool's adapter always passes none, and a reason from a caller that came through MCP is
/// refused here before anything is read (kb/ADR-0029 R10).
pub(crate) async fn move_story(
    state: &AppState,
    who: &Acting<'_>,
    key: &str,
    b: MoveStory,
) -> Result<StoryDto, AppError> {
    who.no_override_over_mcp(b.override_reason.as_deref())?;
    let key = state.handlers.resolve_ref(key, ItemType::Story).await?;
    let story = state
        .handlers
        .move_story(
            &who.ctx,
            &key,
            b.version,
            b.to,
            b.resolution,
            b.override_reason,
        )
        .await?;
    Ok(state.handlers.story_dto(story.key(), who.ctx.now).await?)
}
