//! `list_initiatives`: `reeve initiative ls` as a tool. Read-only.
use super::{Tool, ToolError, bound::summaries};
use crate::{
    actions::{self, Acting},
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{ListInitiativesInput, ListInitiativesResult};
use serde_json::Value;

pub(super) fn tool() -> Tool {
    Tool {
        name: "list_initiatives",
        title: "List initiatives",
        description: format!(
            "List every initiative: the top level of the plan, above epics and stories. Use it to answer what the team is working towards, or to find an initiative's key. It takes no arguments. Answers `initiatives`: up to 200 summaries, oldest first, each with key, title, horizon, target_month, owner, version, discovery_complete and progress (the share of its stories completed, of those not dropped: 0 to 1, or null when nothing counts yet); and `omitted`: the number of initiatives left out to keep the answer small (0 when all are listed; when it is not, find the others with search_items). A summary has no brief: read one initiative with get_initiative, and its epics with list_epics and `initiative`. Read-only. {}",
            super::PEOPLES_TEXT
        ),
        input_schema: super::input_schema::<ListInitiativesInput>(),
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
        // No arguments to adapt; deserializing still refuses any that are sent.
        let _: ListInitiativesInput = super::arguments(args)?;
        let initiatives = actions::list_initiatives(state, who)
            .await
            .map_err(super::told)?;
        let (initiatives, omitted) = summaries(initiatives);
        super::sent(ListInitiativesResult {
            initiatives,
            omitted,
        })
    })
}
