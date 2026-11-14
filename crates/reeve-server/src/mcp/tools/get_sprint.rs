//! `get_sprint`: `reeve sprint show [id]` as a tool. Read-only.
use super::{Tool, ToolError, bound::cut_goal};
use crate::{
    actions::{self, Acting},
    error::AppError,
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{GetSprintInput, SprintViewDto};
use serde_json::Value;

pub(super) fn tool() -> Tool {
    Tool {
        name: "get_sprint",
        title: "Read a sprint",
        description: format!(
            "Read one sprint: its state and dates, its goals (the outcome, how the team will know, the owner, the confidence recorded and the verdict once closed, and the items linked to each), the counts of work completed, in flight, in review and still to do per goal, the stories completed in its last seven days (the demo list), and the promises that fall due in it. Without `id` it reads the current sprint: the active one, or the draft being planned when none is active. Use it for how the sprint is going, what its goals are, or what is promised in it; list the stories serving its goals with list_stories and `goal`. For a closed sprint the counts are as of now, not as of its close. Read-only. {}",
            super::PEOPLES_TEXT
        ),
        input_schema: super::input_schema::<GetSprintInput>(),
        annotations: super::READ_ONLY,
        run,
    }
}

fn run<'a>(
    state: &'a AppState,
    who: &'a Acting<'a>,
    args: Value,
) -> BoxFuture<'a, Result<Value, ToolError>> {
    Box::pin(async move {
        let input: GetSprintInput = super::arguments(args)?;
        // The adapter does what `reeve sprint show` does without an id: read the list, take the active sprint,
        // else the draft, and read that one again by id, because the list carries no promises.
        let id = match input.id {
            Some(id) => id,
            None => {
                let sprints = actions::list_sprints(state, who)
                    .await
                    .map_err(super::told)?;
                match (sprints.active, sprints.draft) {
                    (Some(active), _) => active.sprint.id,
                    (None, Some(draft)) => draft.id,
                    (None, None) => {
                        return Err(ToolError::Told("There is no sprint yet.".into()));
                    }
                }
            }
        };
        let view = actions::get_sprint(state, who, id)
            .await
            .map_err(|e| match e {
                AppError::NotFound => ToolError::Told(format!("No sprint with id {id}.")),
                other => super::told(other),
            })?;
        super::sent(bounded(view))
    })
}

/// The view with each goal's texts cut: the planner limits none of them. The number of goals, promises and demo
/// stories is not capped here; the last guard in `call_tool` holds for a sprint that carries too many.
fn bounded(mut view: SprintViewDto) -> SprintViewDto {
    view.sprint.goals.iter_mut().for_each(cut_goal);
    view
}
