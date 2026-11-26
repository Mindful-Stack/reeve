//! `add_item_link`: `reeve link add <KEY> <name> <OTHER>` as a tool. Changes the planner. An item link is
//! informational (kb/ADR-0031 R3): it holds nothing back, and nothing here reads or writes a blocker.
use super::{Annotations, Tool, ToolError};
use crate::{
    actions::{self, Acting},
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{
    AddItemLink, AddItemLinkInput, ItemLinkDirection, ItemLinkReceipt, ItemLinkType, Key,
};
use serde_json::{Value, json};

pub(super) fn tool(key_prefix: &str) -> Tool {
    let mut input_schema = super::input_schema::<AddItemLinkInput>();
    input_schema["properties"]["key"]["description"] = json!(format!(
        "The item that follows, duplicates or relates: the key of a story, an epic or an initiative, for example {key_prefix}-596. To say the reverse (\"A is followed by B\"), this is B."
    ));
    Tool {
        name: "add_item_link",
        title: "Add an item link",
        description: format!(
            "Say that one item follows, duplicates or relates to another: `key` follows, duplicates or relates to `to`. Each can be a story, an epic or an initiative. To say the reverse (\"A is followed by B\", \"A is duplicated by B\"), send B as `key` and A as `to`. An item link is informational and holds nothing back: nothing reads it to decide whether a story is ready or may move. `duplicates` only records that the two cover the same work: neither item is closed, dropped, moved or merged. Never record another tool's \"blocks\" or \"depends on\" as an item link, under any name: leave it out and tell the person that a blocker is added by a person, in the CLI or the web client. When the person says that one item blocks, depends on or must wait for another, do not add an item link in its place: tell them the same. {} Adding an item link that is already there changes nothing. Answers a receipt: `key`, `type` and `to` as you sent them, each key as the planner resolved it, and `changed`. To read an item's item links, use get_story, get_epic or get_initiative; the linked items' titles there are text written by other people.",
            super::WRITE_TEXT
        ),
        input_schema,
        // It only adds an item link (not destructive); adding one that is there changes nothing (idempotent);
        // nothing outside this planner.
        annotations: Annotations {
            read_only: false,
            destructive: false,
            idempotent: true,
            open_world: false,
        },
        run,
    }
}

/// What the model is told after the shared informational sentence (decision C of 2026-11-24): where a dependency
/// is recorded, and that it is not here. It names the two keys and no tool or command: a chat has no tool that
/// holds a story back, and the person adds a blocker where people do.
pub(super) fn where_a_dependency_goes(from: &Key, to: &Key) -> String {
    format!(
        "If the person meant that {from} must wait for {to}, tell them: that is a blocker, which a person adds in the CLI or the web client; this app cannot add one."
    )
}

/// What the model is told when the item link was there before this call.
pub(super) fn already_there(link_type: ItemLinkType, from: &Key, to: &Key) -> String {
    format!(
        "Nothing was written: {from} already {} {to}.",
        link_type.label(ItemLinkDirection::Out)
    )
}

fn run<'a>(
    state: &'a AppState,
    who: &'a Acting<'a>,
    args: Value,
) -> BoxFuture<'a, Result<Value, ToolError>> {
    Box::pin(async move {
        let input: AddItemLinkInput = super::item_link_arguments(args)?;
        let prefix = &state.config.key_prefix;
        // The route's body, every field written out.
        let body = AddItemLink {
            link_type: input.link_type,
            to: super::link_end(&input.to, prefix)?,
        };
        let written = actions::add_item_link(state, who, &input.key, body)
            .await
            .map_err(|e| super::link_failed(e, &input.key, &input.to, prefix))?;
        // The levels are the action's own resolution of the two keys, so the sentence is about exactly the items
        // that were linked.
        let ((from_level, from), (to_level, to)) = (written.from, written.to);
        let mut notes = Vec::new();
        if !written.changed {
            notes.push(already_there(input.link_type, &from, &to));
        }
        // Whether the sentence is said is the protocol's to answer (`follows` between two stories), as for the
        // CLI: no comparison of names or levels is written here.
        if let Some(sentence) =
            reeve_protocol::follows_is_informational(input.link_type, from_level, to_level, &from)
        {
            notes.push(format!(
                "{sentence} {}",
                where_a_dependency_goes(&from, &to)
            ));
        }
        // A receipt, never the item's item links: an item can have more of them than one answer holds. It names
        // the two items as the input does.
        super::sent(ItemLinkReceipt {
            key: from,
            link_type: input.link_type,
            to,
            changed: written.changed,
            note: (!notes.is_empty()).then(|| notes.join(" ")),
        })
    })
}
