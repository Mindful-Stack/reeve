//! `add_comment`: `reeve comment add` as a tool. Changes the planner.
use super::{Annotations, Tool, ToolError};
use crate::{
    actions::{self, Acting},
    error::AppError,
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{AddComment, AddCommentInput, CommentReceipt};
use serde_json::{Value, json};

pub(super) fn tool(key_prefix: &str) -> Tool {
    let mut input_schema = super::input_schema::<AddCommentInput>();
    input_schema["properties"]["item"]["description"] = json!(format!(
        "The story, epic or initiative to comment on: its key, for example {key_prefix}-596. Send this or `decision`, not both."
    ));
    Tool {
        name: "add_comment",
        title: "Add a comment",
        description: format!(
            "Add a comment to a story, an epic, an initiative or a required decision, as the person you act for. Everyone on the team can read it. {} Make a new `idempotency_key` for every comment, ending in a few random characters, and send a key again only to retry that same comment: a repeat writes nothing and answers `created: false`. Answers a receipt: the comment's id, what it is on, and `created`. To read a story's comments, use get_story.",
            super::WRITE_TEXT
        ),
        input_schema,
        // It only adds a comment (not destructive); the same key makes nothing more (idempotent); nothing outside
        // this planner.
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
        let input: AddCommentInput = super::arguments(args)?;
        let prefix = &state.config.key_prefix;
        // The route's body, every field written out: attachments are withheld (kb/ADR-0029 R12).
        let body = AddComment {
            body: input.body,
            idempotency_key: Some(input.idempotency_key),
            attachments: Vec::new(),
        };
        let (item, decision, made) = match (input.item, input.decision) {
            (Some(item), None) => {
                let ((_, key), made) = actions::add_item_comment(state, who, &item, body)
                    .await
                    .map_err(|e| super::not_found(e, "item", &item, prefix))?;
                (Some(key), None, made)
            }
            (None, Some(decision)) => {
                let made = actions::add_decision_comment(state, who, decision, body)
                    .await
                    .map_err(|e| match e {
                        AppError::NotFound => {
                            ToolError::Told(format!("No required decision with id {decision}."))
                        }
                        other => super::told(other),
                    })?;
                (None, Some(decision), made)
            }
            _ => {
                return Err(ToolError::Told(
                    "Send exactly one of item and decision.".into(),
                ));
            }
        };
        // A receipt, never the thread: twenty comments can be over the largest answer, and the last guard must
        // not refuse an answer after its comment was written. A repeat names no target: a key names one comment
        // across the instance, so the comment it made may be on another item than this call named.
        super::sent(if made.created {
            CommentReceipt {
                comment_id: made.id,
                item,
                decision,
                created: true,
                note: None,
            }
        } else {
            CommentReceipt {
                comment_id: made.id,
                item: None,
                decision: None,
                created: false,
                note: Some(format!(
                    "Nothing was written: this idempotency key already made comment {}, which may be on another item. For another comment, use a new key.",
                    made.id
                )),
            }
        })
    })
}
