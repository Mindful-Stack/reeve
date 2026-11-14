//! `get_story`: `reeve story context <KEY>` as a tool. Read-only.
use super::{
    Annotations, Tool, ToolError,
    bound::{MAX_OTHER_TEXT, MAX_OWN_TEXT, MAX_RECENT, cut, echo, most_recent},
};
use crate::{
    actions::{self, Acting},
    error::AppError,
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{StoryContextDto, StoryContextInput, StoryContextResult};
use serde_json::{Value, json};

pub(super) fn tool(key_prefix: &str) -> Tool {
    let mut input_schema = super::input_schema::<StoryContextInput>();
    input_schema["properties"]["key"]["description"] =
        json!(format!("The story's key, for example {key_prefix}-596."));
    input_schema["properties"]["key"]["examples"] = json!([format!("{key_prefix}-596")]);
    Tool {
        name: "get_story",
        title: "Read a story",
        description: format!(
            "Read one story by key (for example {key_prefix}-596): its description, acceptance criteria, readiness and why it is not ready, blockers, required decisions, sprint goals, epic, initiative and its most recent comments. Read-only. {}",
            super::PEOPLES_TEXT
        ),
        input_schema,
        // Reads only; nothing to destroy; the same call twice changes nothing; reaches nothing outside this planner.
        annotations: Annotations {
            read_only: true,
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
        let input: StoryContextInput = super::arguments(args)?;
        // The adapter: the tool's input becomes what the route's action takes, its path parameter.
        let context = actions::story_context(state, who, &input.key)
            .await
            .map_err(|e| match e {
                AppError::NotFound => ToolError::Told(format!(
                    "No story with key {}. Keys look like {}-123.",
                    echo(&input.key),
                    state.config.key_prefix
                )),
                other => super::told(other),
            })?;
        serde_json::to_value(bounded(context)).map_err(|e| ToolError::Internal(e.into()))
    })
}

/// The context with at most the `MAX_RECENT` most recent comments and a count of the rest, and with long text cut:
/// the story's own description at `MAX_OWN_TEXT`, every other text at `MAX_OTHER_TEXT`. The planner returns
/// comments oldest first (`ORDER BY at, id`), so the oldest are the ones dropped and the rest keep their order.
/// The context is taken apart field by field, with no `..`: a field added to the API's type does not compile here
/// until someone decides how to bound it.
fn bounded(context: StoryContextDto) -> StoryContextResult {
    let StoryContextDto {
        mut story,
        mut epic,
        mut initiative,
        required_decisions,
        mut blockers,
        mut comments,
        readiness,
        goals,
        attachments,
    } = context;
    let comments_omitted = most_recent(&mut comments, MAX_RECENT);
    cut(&mut story.description, MAX_OWN_TEXT);
    for criterion in &mut story.criteria {
        cut(&mut criterion.text, MAX_OTHER_TEXT);
    }
    if let Some(epic) = &mut epic {
        cut(&mut epic.brief, MAX_OTHER_TEXT);
    }
    if let Some(initiative) = &mut initiative {
        cut(&mut initiative.brief, MAX_OTHER_TEXT);
    }
    for blocker in &mut blockers {
        cut(&mut blocker.description, MAX_OTHER_TEXT);
    }
    for comment in &mut comments {
        cut(&mut comment.body, MAX_OTHER_TEXT);
    }
    StoryContextResult {
        context: StoryContextDto {
            story,
            epic,
            initiative,
            required_decisions,
            blockers,
            comments,
            readiness,
            goals,
            attachments,
        },
        comments_omitted,
    }
}
