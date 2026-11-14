//! `get_story`: `reeve story context <KEY>` as a tool. Read-only.
use super::{
    Tool, ToolError,
    bound::{
        MAX_OTHER_TEXT, MAX_OWN_TEXT, MAX_RECENT, cut, cut_closing_reason, cut_goal, most_recent,
    },
};
use crate::{
    actions::{self, Acting},
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
        let input: StoryContextInput = super::arguments(args)?;
        let context = actions::story_context(state, who, &input.key)
            .await
            .map_err(|e| super::not_found(e, "story", &input.key, &state.config.key_prefix))?;
        super::sent(bounded(context))
    })
}

/// The context with at most the `MAX_RECENT` most recent comments and a count of the rest, and with long text cut:
/// the story's own description at `MAX_OWN_TEXT`, every other text at `MAX_OTHER_TEXT`. The planner returns
/// comments oldest first (`ORDER BY at, id`), so the oldest are the ones dropped and the rest keep their order.
/// A blocker's criteria, each goal's texts and each required decision's closing reason are cut like every other
/// text that comes along.
/// The context is taken apart field by field, with no `..`: a field added to the API's type does not compile here
/// until someone decides how to bound it.
fn bounded(context: StoryContextDto) -> StoryContextResult {
    let StoryContextDto {
        mut story,
        mut epic,
        mut initiative,
        mut required_decisions,
        mut blockers,
        mut comments,
        readiness,
        mut goals,
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
        for criterion in &mut blocker.criteria {
            cut(&mut criterion.text, MAX_OTHER_TEXT);
        }
    }
    for comment in &mut comments {
        cut(&mut comment.body, MAX_OTHER_TEXT);
    }
    required_decisions.iter_mut().for_each(cut_closing_reason);
    for goal in &mut goals {
        cut_goal(&mut goal.goal);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// In memory, so that it needs no sprint, goal link or closed decision in a database: a context whose blocker,
    /// goal and required decision each carry one text over the limit.
    #[test]
    fn every_text_that_comes_along_with_a_story_is_cut() {
        let long = "å".repeat(MAX_OTHER_TEXT + 2);
        let cut_long = format!("{}[cut: 2 more characters]", "å".repeat(MAX_OTHER_TEXT));
        let story = |key: &str| {
            json!({
                "key": key, "title": "A story", "kind": "story", "epic": null, "description": "",
                "criteria": [{"text": "short", "done": false}, {"text": long, "done": false}],
                "owner": null, "labels": [], "rank": 0, "blocked_by": [], "delivery": "todo",
                "resolution": null, "external_ref": null, "version": 1, "prs": []
            })
        };
        let context: StoryContextDto = serde_json::from_value(json!({
            "story": story("TST-1"),
            "epic": null,
            "initiative": null,
            "required_decisions": [{
                "id": 1, "on": "TST-1", "on_type": "story", "title": "Which provider",
                "decider": "dan@example.com", "due": null, "record": null, "pr_url": null,
                "check": {"kind": "unchecked"}, "checked_at": null,
                "closed": {"at": "2026-11-01T10:00:00Z", "by": "dan@example.com", "reason": long},
                "created_at": "2026-11-01T09:00:00Z", "version": 2
            }],
            "blockers": [story("TST-2")],
            "comments": [],
            "readiness": {"ready": true, "reasons": []},
            "goals": [{
                "goal": {
                    "id": 1, "sprint_id": 1, "outcome": "Customers can book", "check": "one did",
                    "owner": "dan@example.com", "notes": long, "confidence": null,
                    "confidence_note": null, "confidence_at": null, "suggested_confidence": null,
                    "suggested_note": null, "suggested_at": null, "suggested_by": null,
                    "verdict": null, "verdict_note": null, "version": 1, "links": []
                },
                "starts_on": "2026-11-01", "ends_on": "2026-11-14", "via": "story"
            }],
            "attachments": []
        }))
        .unwrap();
        let sent = bounded(context).context;
        assert_eq!(sent.blockers[0].criteria[0].text, "short");
        assert_eq!(sent.blockers[0].criteria[1].text, cut_long);
        assert_eq!(sent.goals[0].goal.outcome, "Customers can book");
        assert_eq!(sent.goals[0].goal.notes, cut_long);
        assert_eq!(
            sent.required_decisions[0]
                .closed
                .as_ref()
                .map(|closed| closed.reason.as_str()),
            Some(cut_long.as_str())
        );
    }
}
