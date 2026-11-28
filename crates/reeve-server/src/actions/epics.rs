use super::templates::{Start, starting_brief};
use super::{Acting, Edited, FilterRef, resolve_edit, resolve_filter, resolve_opt};
use crate::{error::AppError, state::AppState};
use reeve_planning::{Epic, EpicMove};
use reeve_protocol::{BriefTemplateLevel, CreateEpic, EditItem, EpicDto, ItemType, KeyRef};
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
/// one, and nothing was written). The brief is the caller's, or starts from a brief template when the body asks
/// for one (`templates::starting_brief`); nothing about the template is stored.
pub(crate) async fn create_epic(
    state: &AppState,
    who: &Acting<'_>,
    b: CreateEpic,
) -> Result<(EpicDto, bool), AppError> {
    // The key's space is the caller's to decide, never the body's.
    let idem = who.write_key(&b.idempotency_key)?;
    let initiative =
        resolve_opt(&state.handlers, b.initiative.as_ref(), ItemType::Initiative).await?;
    // The brief the caller sent, or the template it asked for (`template`, `default_template`). A caller that asks
    // for neither, an MCP tool among them, gets exactly what it sent. A retry whose named template is gone since
    // is answered the item its key made (`Start::Made`), as the handler answers every other retry.
    let (key, created) = match starting_brief(
        state,
        BriefTemplateLevel::Epic,
        &idem,
        &b.attachments,
        b.brief.as_deref(),
        b.template.as_deref(),
        b.default_template,
    )
    .await?
    {
        Start::Made(key) => (key, false),
        Start::Brief(brief) => {
            let (epic, created) = state
                .handlers
                .create_epic_reporting(
                    &who.ctx,
                    &idem,
                    &b.title,
                    initiative,
                    &brief,
                    b.external_ref,
                    &b.attachments,
                )
                .await?;
            (epic.key().clone(), created)
        }
    };
    Ok((state.handlers.epic_dto(&key, who.ctx.now).await?, created))
}

/// `reeve epic edit <KEY>`. Shared by `PATCH /api/v1/epics/{key}` and the MCP tool `edit_epic`. `key` and `b` are
/// what the route takes: its path parameter and its body. An edit never reads or writes the discovery mark: the
/// handler carries the stored mark over (`POST …/discovery` is the only way to it). Answers the epic as this edit
/// wrote it (the DTO the route sends, the rest read after the commit, and the item the receipt is built from), and
/// the move this edit wrote (`None`: it did not change the initiative), which `edit_epic`'s receipt says (decision
/// A). The written epic and the move are the handler's: the move is the one its `edited` entry names, never one
/// worked out here from reads around the write, which another edit could fall between.
pub(crate) async fn edit_epic(
    state: &AppState,
    who: &Acting<'_>,
    key: &str,
    b: EditItem,
) -> Result<(Edited<EpicDto, Epic>, Option<EpicMove>), AppError> {
    let key = state.handlers.resolve_ref(key, ItemType::Epic).await?;
    let initiative = resolve_edit(&state.handlers, b.initiative, ItemType::Initiative).await?;
    let (written, moved) = state
        .handlers
        .edit_epic_reporting(
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
    let answer = state
        .handlers
        .epic_dto_as_written(&written, who.ctx.now)
        .await?;
    Ok((Edited { answer, written }, moved))
}
