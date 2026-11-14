//! `search_items`: `reeve search <text>` as a tool. Read-only.
use super::{Tool, ToolError, bound::fit};
use crate::{
    actions::{self, Acting, SearchQuery},
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{SearchItemsInput, SearchItemsResult};
use serde_json::Value;

pub(super) fn tool(key_prefix: &str) -> Tool {
    Tool {
        name: "search_items",
        title: "Search the planner",
        description: format!(
            "Find initiatives, epics and stories by text: a word or phrase from a title, brief, description or external reference (any case), or a key such as {key_prefix}-596. Use it when the person names work by what it is about and you do not have its key; to list stories by owner, state, epic or sprint goal, use list_stories instead. Answers `hits`: up to 50, best match first, each with its key, its level (initiative, epic or story), a story's kind, its title, which fields matched, and `exact` when the text is that item's key or its whole external reference; and `omitted`: the number of hits left out to keep the answer small (0 when all are listed; when it is not, search for something narrower). Read a hit with get_story, get_epic or get_initiative. Read-only. {}",
            super::PEOPLES_TEXT
        ),
        input_schema: super::input_schema::<SearchItemsInput>(),
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
        let input: SearchItemsInput = super::arguments(args)?;
        let mut hits = actions::search(state, who, &SearchQuery { q: input.text })
            .await
            .map_err(super::told)?;
        // The handler stops at 50 hits; the budget is the only bound left to apply.
        let omitted = fit(&mut hits, 0);
        super::sent(SearchItemsResult { hits, omitted })
    })
}
