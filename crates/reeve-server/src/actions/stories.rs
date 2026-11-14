use super::{Acting, FilterRef, resolve_filter};
use crate::{error::AppError, state::AppState};
use reeve_planning::{GoalFilter, StoryFilter};
use reeve_protocol::{
    DeliveryState, DoneFilter, ItemType, KeyRef, Kind, PromisedFilter, StorySort,
    StoryWithReadiness,
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
