use super::{Acting, allowed_owner, named_target_month};
use crate::{error::AppError, state::AppState};
use reeve_protocol::{CreateInitiative, EditItem, InitiativeDto, ItemType};

/// `reeve initiative ls`: every initiative with its progress. Shared by `GET /api/v1/initiatives` and the MCP tool
/// `list_initiatives`. The route takes no input.
pub(crate) async fn list_initiatives(
    state: &AppState,
    who: &Acting<'_>,
) -> Result<Vec<InitiativeDto>, AppError> {
    Ok(state.handlers.list_initiatives(who.ctx.now).await?)
}

/// `reeve initiative show <KEY>`: one initiative. Shared by `GET /api/v1/initiatives/{key}` and the MCP tool
/// `get_initiative`. `key` is what the route takes: its path parameter, an initiative's key or external reference.
pub(crate) async fn get_initiative(
    state: &AppState,
    who: &Acting<'_>,
    key: &str,
) -> Result<InitiativeDto, AppError> {
    let key = state
        .handlers
        .resolve_ref(key, ItemType::Initiative)
        .await?;
    Ok(state.handlers.initiative_dto(&key, who.ctx.now).await?)
}

/// `reeve initiative create`. Shared by `POST /api/v1/initiatives` and the MCP tool `create_initiative`. `b` is
/// what the route takes: its body. The owner must be an allowed user and the target month a month. Answers the
/// initiative and whether this call created it.
pub(crate) async fn create_initiative(
    state: &AppState,
    who: &Acting<'_>,
    b: CreateInitiative,
) -> Result<(InitiativeDto, bool), AppError> {
    let idem = who.write_key(&b.idempotency_key)?;
    let owner = allowed_owner(who.policy, b.owner)?;
    let target_month = named_target_month(b.target_month)?;
    let (initiative, created) = state
        .handlers
        .create_initiative_reporting(
            &who.ctx,
            &idem,
            &b.title,
            b.brief.as_deref().unwrap_or(""),
            &b.horizon,
            target_month,
            owner,
            b.external_ref,
            &b.attachments,
        )
        .await?;
    Ok((
        state
            .handlers
            .initiative_dto(initiative.key(), who.ctx.now)
            .await?,
        created,
    ))
}

/// `reeve initiative edit <KEY>`. Shared by `PATCH /api/v1/initiatives/{key}` and the MCP tool
/// `edit_initiative`. `key` and `b` are what the route takes. An owner the edit names must be an allowed user and a
/// target month it names must be a month; an edit that leaves either alone or clears it is not judged, so a stored
/// owner who is no longer allowed and a stored month in another shape never stand in the way of another change.
/// An edit never reads or writes the discovery mark.
pub(crate) async fn edit_initiative(
    state: &AppState,
    who: &Acting<'_>,
    key: &str,
    b: EditItem,
) -> Result<InitiativeDto, AppError> {
    let key = state
        .handlers
        .resolve_ref(key, ItemType::Initiative)
        .await?;
    // `Some(Some(_))` is a value the request names; `Some(None)` clears and `None` leaves alone, and neither is
    // judged.
    let owner = match b.owner {
        Some(Some(o)) => Some(allowed_owner(who.policy, Some(o))?),
        other => other,
    };
    let target_month = match b.target_month {
        Some(Some(m)) => Some(named_target_month(Some(m))?),
        other => other,
    };
    state
        .handlers
        .edit_initiative_attaching(
            &who.ctx,
            &key,
            b.version,
            b.title,
            b.brief,
            b.horizon,
            target_month,
            owner,
            b.external_ref,
            &b.attachments,
        )
        .await?;
    Ok(state.handlers.initiative_dto(&key, who.ctx.now).await?)
}
