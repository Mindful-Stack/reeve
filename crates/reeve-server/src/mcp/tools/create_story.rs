//! `create_story`: `reeve story create` as a tool. Changes the planner.
use super::{Annotations, Tool, ToolError, bound};
use crate::{
    actions::{self, Acting},
    error::AppError,
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{CreateStory, CreateStoryInput, Kind, StoryReceipt};
use serde_json::{Value, json};

pub(super) fn tool(key_prefix: &str) -> Tool {
    let mut input_schema = super::input_schema::<CreateStoryInput>();
    input_schema["properties"]["epic"]["description"] = json!(format!(
        "The epic to place it in: the epic's key, for example {key_prefix}-12. Left out, the item is untriaged until someone places it."
    ));
    Tool {
        name: "create_story",
        title: "Create a story",
        description: format!(
            "Create a story, a bug or a chore. {} Make one `idempotency_key` for each story you mean to create, ending in a few random characters, and send it again only to retry that same create: a repeat writes nothing and answers the story the key already made, with `created: false`. Search first (search_items) when the person may be describing work that already exists. Answers a receipt: the story's key, title, delivery state and version, and `created`. To read the story, use get_story.",
            super::WRITE_TEXT
        ),
        input_schema,
        // It only adds a story (not destructive); the same arguments, key included, make nothing more
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
        let input: CreateStoryInput = super::arguments(args)?;
        let prefix = &state.config.key_prefix;
        let named_epic = input.epic.clone();
        // Every field of the route's body is written out, with no `..`: a field added to `CreateStory` does not
        // compile here until someone decides whether a chat may send it (kb/ADR-0029 R12). The tool withholds the
        // external reference and attachments.
        let body = CreateStory {
            idempotency_key: input.idempotency_key,
            title: input.title,
            kind: input.kind.unwrap_or(Kind::Story),
            epic: super::key_ref("epic", input.epic, prefix)?,
            description: input.description,
            criteria: input.criteria,
            labels: input.labels,
            external_ref: None,
            owner: input.owner,
            attachments: Vec::new(),
        };
        let (story, created) =
            actions::create_story(state, who, body)
                .await
                .map_err(|e| match (e, &named_epic) {
                    // The epic is the only thing a create looks up, when one is named.
                    (AppError::NotFound, Some(epic)) => ToolError::Told(format!(
                        "No epic with key {:?}. Keys look like {prefix}-123.",
                        bound::echo(epic)
                    )),
                    (other, _) => super::told(other),
                })?;
        // A receipt, never the story: nothing in it but the title is a person's text, and the planner limits a
        // title to 200 characters, so the last guard cannot refuse this answer after its write.
        let mut receipt = StoryReceipt::from(story);
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
