//! `create_epic`: `reeve epic create` as a tool. Changes the planner. It cannot mark discovery: a new epic's
//! discovery is not complete, and the input has nothing that says otherwise.
use super::{Annotations, Tool, ToolError, bound};
use crate::{
    actions::{self, Acting},
    error::AppError,
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{CreateEpic, CreateEpicInput, EpicReceipt};
use serde_json::{Value, json};

pub(super) fn tool(key_prefix: &str) -> Tool {
    let mut input_schema = super::input_schema::<CreateEpicInput>();
    input_schema["properties"]["initiative"]["description"] = json!(format!(
        "The initiative the epic belongs to: the initiative's key, for example {key_prefix}-12. Left out, the epic belongs to none."
    ));
    Tool {
        name: "create_epic",
        title: "Create an epic",
        description: format!(
            "Create an epic: a body of work that stories are placed in. {} Make one `idempotency_key` for each epic you mean to create, ending in a few random characters, and send it again only to retry that same create: a repeat writes nothing and answers the epic the key already made, with `created: false`. Search first (search_items) when the person may be describing an epic that already exists. A new epic's discovery is not complete, so stories placed in it are not ready yet; this tool cannot mark it complete: a person does that, in the CLI or the web client. Answers a receipt: the epic's key, title, initiative and version, and `created`. To read the epic, use get_epic.",
            super::WRITE_TEXT
        ),
        input_schema,
        // It only adds an epic (not destructive); the same arguments, key included, make nothing more
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
        let input: CreateEpicInput = super::arguments(args)?;
        let prefix = &state.config.key_prefix;
        let named = input.initiative.clone();
        // Every field of the route's body is written out, with no `..`: a field added to `CreateEpic` does not
        // compile here until someone decides whether a chat may send it (kb/ADR-0029 R12). The tool withholds
        // the external reference and attachments.
        let body = CreateEpic {
            idempotency_key: input.idempotency_key,
            title: input.title,
            initiative: super::key_ref("initiative", input.initiative, prefix)?,
            brief: input.brief,
            external_ref: None,
            attachments: Vec::new(),
        };
        let (epic, created) =
            actions::create_epic(state, who, body)
                .await
                .map_err(|e| match (e, &named) {
                    // The initiative is the only thing a create looks up, when one is named.
                    (AppError::NotFound, Some(initiative)) => ToolError::Told(format!(
                        "No initiative with key {:?}. Keys look like {prefix}-123.",
                        bound::echo(initiative)
                    )),
                    (other, _) => super::told(other),
                })?;
        // A receipt, never the epic: a brief can be over the largest answer, and the last guard must not refuse
        // an answer after its write.
        let mut receipt = EpicReceipt::from(epic);
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
