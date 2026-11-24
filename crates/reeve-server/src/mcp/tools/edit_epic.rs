//! `edit_epic`: `reeve epic edit <KEY>` as a tool. Changes the planner. It cannot reach the discovery mark: the
//! input has no such field, and the edit the action runs carries the stored mark over.
use super::{Annotations, Tool, ToolError, bound};
use crate::{
    actions::{self, Acting},
    error::AppError,
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{EditEpicInput, EditItem, EpicReceipt, Key};
use serde_json::{Value, json};

/// The receipt's note when the edit moved the epic (decision A): where it was moved from and to, and what its
/// stories now wait on. Built from the action's answer: `from` is the initiative it left, `to` the one it is in now
/// (None: none). None when neither is set, which a move cannot be.
fn moved_note(from: Option<&Key>, to: Option<&Key>) -> Option<String> {
    match (from, to) {
        (Some(from), Some(to)) => Some(format!(
            "Moved from {from} to {to}: its stories now wait on {to}'s discovery mark and required decisions."
        )),
        (None, Some(to)) => Some(format!(
            "Moved to {to}: its stories now wait on {to}'s discovery mark and required decisions."
        )),
        (Some(from), None) => Some(format!(
            "Taken out of {from}: its stories no longer wait on {from}'s discovery mark and required decisions."
        )),
        (None, None) => None,
    }
}

pub(super) fn tool(key_prefix: &str) -> Tool {
    let mut input_schema = super::input_schema::<EditEpicInput>();
    input_schema["properties"]["key"]["description"] =
        json!(format!("The epic's key, for example {key_prefix}-20."));
    Tool {
        name: "edit_epic",
        title: "Edit an epic",
        description: format!(
            "Change one epic's title, brief or initiative. Only the fields you send are changed; what a field held before is replaced. {} Read the epic first (get_epic) and send its `version`: if someone changed the epic since, the edit is refused and you are told the current version. get_epic shortens a long brief and ends it with `[cut: N more characters]`. Such a brief cannot be edited here: never send it back; leave it out and tell the person to edit it in the CLI or the web client. Moving an epic to another initiative, or out of one, changes which discovery mark and required decisions its stories wait on: do it only when the person asked for that move, and when you report the edit, say that the epic moved and that its stories now wait on other discovery and decisions. This tool cannot mark an epic's discovery complete or reopen it: a person does that, in the CLI or the web client. Answers a receipt: the epic's key, title, initiative and new version, and a `note` when the epic moved.",
            super::WRITE_TEXT
        ),
        input_schema,
        // It replaces what a field held (destructive). The same arguments again carry a version that is no
        // longer current and are refused, so a repeat changes nothing more (idempotent). Nothing outside this
        // planner.
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
        let input: EditEpicInput = super::arguments(args)?;
        let prefix = &state.config.key_prefix;
        let told = |message: &str| Err(ToolError::Told(message.to_string()));
        if input.detach_initiative && input.initiative.is_some() {
            return told("detach_initiative and initiative cannot be sent together.");
        }
        if input.brief.as_deref().is_some_and(super::looks_cut) {
            return told(
                "brief ends with the marker of a text get_epic cut. The whole text cannot be read here, so it cannot be replaced here: tell the person to edit it in the CLI or the web client.",
            );
        }
        if input.title.is_none()
            && input.brief.is_none()
            && input.initiative.is_none()
            && !input.detach_initiative
        {
            return told("Nothing to change: send at least one field besides key and version.");
        }
        let named = input.initiative.clone();
        let initiative = super::key_ref("initiative", input.initiative, prefix)?;
        // `..EditItem::default()` on purpose, as in `edit_story`: in an edit `None` means "leave it alone", so
        // what a chat may not send stays untouched by the default (the external reference, attachments, and
        // every field that is a story's or an initiative's).
        let body = EditItem {
            version: input.version,
            title: input.title,
            brief: input.brief,
            initiative: if input.detach_initiative {
                Some(None)
            } else {
                initiative.map(Some)
            },
            ..EditItem::default()
        };
        let (epic, left) = actions::edit_epic(state, who, &input.key, body)
            .await
            .map_err(|e| match (e, &named) {
                // With an initiative named, a missing item may be either.
                (AppError::NotFound, Some(initiative)) => ToolError::Told(format!(
                    "No epic with key {:?}, or no initiative with key {:?}. Keys look like {prefix}-123.",
                    bound::echo(&input.key),
                    bound::echo(initiative)
                )),
                (other, _) => {
                    super::item_write_failed(other, "epic", "get_epic", &input.key, prefix)
                }
            })?;
        let note = left.and_then(|from| moved_note(from.as_ref(), epic.initiative.as_ref()));
        let mut receipt = EpicReceipt::from(epic);
        receipt.note = note;
        super::sent(receipt)
    })
}
