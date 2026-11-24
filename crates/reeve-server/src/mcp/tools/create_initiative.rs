//! `create_initiative`: `reeve initiative create` as a tool. Changes the planner. It cannot mark discovery.
use super::{Annotations, Tool, ToolError};
use crate::{
    actions::{self, Acting},
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{CreateInitiative, CreateInitiativeInput, InitiativeReceipt};
use serde_json::{Value, json};

pub(super) fn tool(horizons: &[String]) -> Tool {
    let mut input_schema = super::input_schema::<CreateInitiativeInput>();
    input_schema["properties"]["horizon"]["description"] = json!(format!(
        "The planning horizon, exactly one of: {}.",
        horizons.join(", ")
    ));
    Tool {
        name: "create_initiative",
        title: "Create an initiative",
        description: format!(
            "Create an initiative: the top level of the plan, which epics belong to. {} Make one `idempotency_key` for each initiative you mean to create, ending in a few random characters, and send it again only to retry that same create: a repeat writes nothing and answers the initiative the key already made, with `created: false`. Search first (search_items) when the person may be describing an initiative that already exists. A new initiative's discovery is not complete, so stories under it are not ready yet; this tool cannot mark it complete: a person does that, in the CLI or the web client. Answers a receipt: the initiative's key, title and version, and `created`. To read the initiative, use get_initiative.",
            super::WRITE_TEXT
        ),
        input_schema,
        // It only adds an initiative (not destructive); the same arguments, key included, make nothing more
        // (idempotent); it reaches nothing outside this planner.
        annotations: Annotations {
            read_only: false,
            destructive: false,
            idempotent: true,
            open_world: false,
        },
        run,
    }
}

fn run<'a>(
    state: &'a AppState,
    who: &'a Acting<'a>,
    args: Value,
) -> BoxFuture<'a, Result<Value, ToolError>> {
    Box::pin(async move {
        let input: CreateInitiativeInput = super::arguments(args)?;
        // Every field of the route's body is written out, with no `..` (kb/ADR-0029 R12). The tool withholds the
        // external reference and attachments. A create looks nothing up, so nothing here can be "not found".
        let body = CreateInitiative {
            idempotency_key: input.idempotency_key,
            title: input.title,
            brief: input.brief,
            horizon: input.horizon,
            target_month: input.target_month,
            owner: input.owner,
            external_ref: None,
            attachments: Vec::new(),
        };
        let (initiative, created) = actions::create_initiative(state, who, body)
            .await
            .map_err(super::told)?;
        let mut receipt = InitiativeReceipt::from(initiative);
        receipt.created = Some(created);
        if !created {
            receipt.note = Some(format!(
                "Nothing was written: this idempotency key already made {} ({:?}). If you did not mean to retry that create, send it again with a new key.",
                receipt.key, receipt.title
            ));
        }
        super::sent(receipt)
    })
}
