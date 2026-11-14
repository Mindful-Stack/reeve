//! `get_initiative`: `reeve initiative show <KEY>` as a tool. Read-only.
use super::{
    Tool, ToolError,
    bound::{MAX_OWN_TEXT, cut},
};
use crate::{
    actions::{self, Acting},
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::GetInitiativeInput;
use serde_json::{Value, json};

pub(super) fn tool(key_prefix: &str) -> Tool {
    let mut input_schema = super::input_schema::<GetInitiativeInput>();
    input_schema["properties"]["key"]["description"] = json!(format!(
        "The initiative's key, for example {key_prefix}-12."
    ));
    input_schema["properties"]["key"]["examples"] = json!([format!("{key_prefix}-12")]);
    Tool {
        name: "get_initiative",
        title: "Read an initiative",
        description: format!(
            "Read one initiative by key (for example {key_prefix}-12): its title, brief, horizon, target month, owner, progress (the share of its stories completed, of those not dropped: 0 to 1, or null when nothing counts yet), whether its discovery is complete, whether it is urgent, and its version. Use it when the person asks what an initiative is for or how far along it is. Its epics are not included: list them with list_epics and `initiative`. Read-only. {}",
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
        let input: GetInitiativeInput = super::arguments(args)?;
        let mut initiative = actions::get_initiative(state, who, &input.key)
            .await
            .map_err(|e| super::not_found(e, "initiative", &input.key, &state.config.key_prefix))?;
        // The initiative is the item asked for: its brief is its own text.
        cut(&mut initiative.brief, MAX_OWN_TEXT);
        super::sent(initiative)
    })
}
