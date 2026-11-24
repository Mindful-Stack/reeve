//! `remove_item_link`: `reeve link rm <KEY> <name> <OTHER>` as a tool. Changes the planner. It removes an item
//! link, never a blocker: the two are different tables, and this tool reaches only the first.
use super::{Annotations, Tool, ToolError};
use crate::{
    actions::{self, Acting},
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{ItemLinkDirection, ItemLinkReceipt, RemoveItemLinkInput};
use serde_json::{Value, json};

pub(super) fn tool(key_prefix: &str) -> Tool {
    let mut input_schema = super::input_schema::<RemoveItemLinkInput>();
    input_schema["properties"]["key"]["description"] = json!(format!(
        "The item that follows, duplicates or relates, for example {key_prefix}-596. For an item link an item shows with `direction: in`, this is the other item."
    ));
    Tool {
        name: "remove_item_link",
        title: "Remove an item link",
        description: format!(
            "Remove one item link, named as it was added: `key` is the item that follows, duplicates or relates to `to`. An item link that an item shows with `direction: in` (for example \"is followed by\") is removed by sending the other item as `key`; for `relates_to` either order names the same item link. An item link is informational: it never holds a story back, so removing one lets nothing start. {} Removing an item link that is not there changes nothing. Answers a receipt: `key`, `type` and `to` as you sent them, each key as the planner resolved it, and `changed`.",
            super::WRITE_TEXT
        ),
        input_schema,
        // It removes an item link (destructive); removing one that is not there changes nothing (idempotent);
        // nothing outside this planner.
        annotations: Annotations {
            read_only: false,
            destructive: true,
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
        let input: RemoveItemLinkInput = super::arguments(args)?;
        let prefix = &state.config.key_prefix;
        // The same check and the same message as `add_item_link`, before the action; the action takes the route's
        // three path segments.
        let to = super::link_end(&input.to, prefix)?;
        let written =
            actions::remove_item_link(state, who, &input.key, input.link_type.wire(), to.as_str())
                .await
                .map_err(|e| super::link_failed(e, &input.key, &input.to, prefix))?;
        let ((_, from), (_, to)) = (written.from, written.to);
        let note = (!written.changed).then(|| {
            format!(
                "Nothing was removed: there is no item link \"{from} {} {to}\". An item link is removed as it was added: for one that an item shows with `direction: in`, send the other item as `key`.",
                input.link_type.label(ItemLinkDirection::Out)
            )
        });
        super::sent(ItemLinkReceipt {
            key: from,
            link_type: input.link_type,
            to,
            changed: written.changed,
            note,
        })
    })
}
