//! `list_epics`: `reeve epic ls` as a tool. Read-only.
use super::{Tool, ToolError, bound::summaries};
use crate::{
    actions::{self, Acting, EpicQuery},
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{ListEpicsInput, ListEpicsResult};
use serde_json::Value;

pub(super) fn tool() -> Tool {
    Tool {
        name: "list_epics",
        title: "List epics",
        description: format!(
            "List epics: all of them, or those of one initiative. Use it to see how an initiative is broken down, or to find the epic a piece of work belongs under. Answers `epics`: up to 200 summaries, oldest first, each with key, title, initiative, version, discovery_complete and promised_on; and `omitted`: the number of epics left out to keep the answer small (0 when all are listed; when it is not, narrow with `initiative` or use search_items). A summary has no brief: read one epic with get_epic, and its stories with list_stories and `epic`. Read-only. {}",
            super::PEOPLES_TEXT
        ),
        input_schema: super::input_schema::<ListEpicsInput>(),
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
        let input: ListEpicsInput = super::arguments(args)?;
        let query = EpicQuery {
            initiative: super::key_ref("initiative", input.initiative, &state.config.key_prefix)?,
        };
        let epics = actions::list_epics(state, who, &query)
            .await
            .map_err(super::told)?;
        let (epics, omitted) = summaries(epics);
        super::sent(ListEpicsResult { epics, omitted })
    })
}
