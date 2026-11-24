use super::{Acting, FilterRef, resolve_edit, resolve_filter, resolve_opt};
use crate::{error::AppError, state::AppState};
use reeve_protocol::{CreateEpic, EditItem, EpicDto, ItemType, Key, KeyRef};
use serde::Deserialize;

/// The query of `GET /api/v1/epics`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct EpicQuery {
    /// An initiative key or external reference. A reference that matches nothing yields an empty list, not 404.
    pub(crate) initiative: Option<KeyRef>,
}

/// `reeve epic ls`: every epic, of one initiative when given. Shared by `GET /api/v1/epics` and the MCP tool
/// `list_epics`. `q` is what the route takes: its query.
pub(crate) async fn list_epics(
    state: &AppState,
    who: &Acting<'_>,
    q: &EpicQuery,
) -> Result<Vec<EpicDto>, AppError> {
    let initiative =
        match resolve_filter(&state.handlers, q.initiative.as_ref(), ItemType::Initiative).await? {
            FilterRef::Nothing => return Ok(Vec::new()),
            FilterRef::Is(k) => Some(k),
            FilterRef::Unset => None,
        };
    Ok(state
        .handlers
        .epic_dtos(initiative.as_ref(), who.ctx.now)
        .await?)
}

/// `reeve epic show <KEY>`: one epic. Shared by `GET /api/v1/epics/{key}` and the MCP tool `get_epic`. `key` is
/// what the route takes: its path parameter, an epic's key or external reference.
pub(crate) async fn get_epic(
    state: &AppState,
    who: &Acting<'_>,
    key: &str,
) -> Result<EpicDto, AppError> {
    let key = state.handlers.resolve_ref(key, ItemType::Epic).await?;
    Ok(state.handlers.epic_dto(&key, who.ctx.now).await?)
}

/// `reeve epic create`. Shared by `POST /api/v1/epics` and the MCP tool `create_epic`. `b` is what the route
/// takes: its body. Answers the epic and whether this call created it (false: the idempotency key already had
/// one, and nothing was written).
pub(crate) async fn create_epic(
    state: &AppState,
    who: &Acting<'_>,
    b: CreateEpic,
) -> Result<(EpicDto, bool), AppError> {
    // The key's space is the caller's to decide, never the body's.
    let idem = who.write_key(&b.idempotency_key)?;
    let initiative =
        resolve_opt(&state.handlers, b.initiative.as_ref(), ItemType::Initiative).await?;
    let (epic, created) = state
        .handlers
        .create_epic_reporting(
            &who.ctx,
            &idem,
            &b.title,
            initiative,
            b.brief.as_deref().unwrap_or(""),
            b.external_ref,
            &b.attachments,
        )
        .await?;
    Ok((
        state.handlers.epic_dto(epic.key(), who.ctx.now).await?,
        created,
    ))
}

/// `reeve epic edit <KEY>`. Shared by `PATCH /api/v1/epics/{key}` and the MCP tool `edit_epic`. `key` and `b` are
/// what the route takes: its path parameter and its body. An edit never reads or writes the discovery mark: the
/// handler carries the stored mark over (`POST …/discovery` is the only way to it). Answers the epic as edited and,
/// when the edit moved it to another initiative or out of one, the initiative it left (`Some(None)`: it was in
/// none), which `edit_epic`'s receipt says (decision A). The epic is read before the edit: the versioned update
/// refuses the edit if the row changed since the caller's version, so an edit that succeeds replaced the row that
/// read saw.
pub(crate) async fn edit_epic(
    state: &AppState,
    who: &Acting<'_>,
    key: &str,
    b: EditItem,
) -> Result<(EpicDto, Option<Option<Key>>), AppError> {
    let key = state.handlers.resolve_ref(key, ItemType::Epic).await?;
    let initiative = resolve_edit(&state.handlers, b.initiative, ItemType::Initiative).await?;
    let before = state.handlers.epic_dto(&key, who.ctx.now).await?;
    let epic = state
        .handlers
        .edit_epic_attaching(
            &who.ctx,
            &key,
            b.version,
            b.title,
            b.brief,
            initiative,
            b.external_ref,
            &b.attachments,
        )
        .await?;
    let after = state.handlers.epic_dto(epic.key(), who.ctx.now).await?;
    let left = (after.initiative != before.initiative).then(|| before.initiative.clone());
    Ok((after, left))
}
