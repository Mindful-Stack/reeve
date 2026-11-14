use super::Acting;
use crate::{error::AppError, state::AppState};
use reeve_planning::{sprint_dto, sprint_view_dto};
use reeve_protocol::{SprintViewDto, SprintsDto};

/// The active sprint with its numbers, the draft, and the closed sprints newest first, from one snapshot: what
/// `reeve sprint ls` prints and what `reeve sprint show` without an id reads to pick the current sprint. Shared by
/// `GET /api/v1/sprints` and the MCP tool `get_sprint`. The route takes no input.
pub(crate) async fn list_sprints(
    state: &AppState,
    who: &Acting<'_>,
) -> Result<SprintsDto, AppError> {
    let x = state.handlers.sprints(who.ctx.now).await?;
    let line = state.handlers.settings.upkeep_line;
    Ok(SprintsDto {
        today: Some(x.today),
        active: x.active.as_ref().map(|a| {
            sprint_view_dto(
                a,
                &x.goals,
                &x.links,
                x.active_summary.as_ref(),
                // The active sprint's promises are read only by `GET /sprints/{id}` until a follow-up of GRA-608
                // renders them here.
                &[],
                None,
                line,
            )
        }),
        draft: x.draft.as_ref().map(|d| sprint_dto(d, &x.goals, &x.links)),
        closed: x
            .closed
            .iter()
            .map(|c| sprint_dto(c, &x.goals, &x.links))
            .collect(),
    })
}

/// `reeve sprint show <id>`: one sprint with its numbers, derived on read over its window, and its promises.
/// Shared by `GET /api/v1/sprints/{id}` and the MCP tool `get_sprint`. `id` is what the route takes: its path
/// parameter.
pub(crate) async fn get_sprint(
    state: &AppState,
    who: &Acting<'_>,
    id: i64,
) -> Result<SprintViewDto, AppError> {
    let v = state.handlers.sprint_view(id, who.ctx.now).await?;
    Ok(sprint_view_dto(
        &v.sprint,
        &v.goals,
        &v.links,
        v.summary.as_ref(),
        &v.promises,
        v.promises_after,
        state.handlers.settings.upkeep_line,
    ))
}
