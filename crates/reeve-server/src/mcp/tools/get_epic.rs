//! `get_epic`: `reeve epic show <KEY>` as a tool. Read-only.
use super::{
    Tool, ToolError,
    bound::{MAX_OWN_TEXT, cut},
};
use crate::{
    actions::{self, Acting},
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::GetEpicInput;
use serde_json::{Value, json};

pub(super) fn tool(key_prefix: &str) -> Tool {
    let mut input_schema = super::input_schema::<GetEpicInput>();
    input_schema["properties"]["key"]["description"] =
        json!(format!("The epic's key, for example {key_prefix}-20."));
    input_schema["properties"]["key"]["examples"] = json!([format!("{key_prefix}-20")]);
    Tool {
        name: "get_epic",
        title: "Read an epic",
        description: format!(
            "Read one epic by key (for example {key_prefix}-20): its title, brief, the initiative it belongs to, whether its discovery is complete (stories under an epic still in discovery are not ready), its promise if it has one, whether it is urgent, and its version. Use it when the person asks what an epic is about or why its stories are held. Its stories are not included: list them with list_stories and `epic`. {} Read-only. {}",
            super::CUT_TEXT,
            super::PEOPLES_TEXT
        ),
        input_schema,
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
        let input: GetEpicInput = super::arguments(args)?;
        let mut epic = actions::get_epic(state, who, &input.key)
            .await
            .map_err(|e| super::not_found(e, "epic", &input.key, &state.config.key_prefix))?;
        // The epic is the item asked for: its brief is its own text.
        cut(&mut epic.brief, MAX_OWN_TEXT);
        super::sent(epic)
    })
}
