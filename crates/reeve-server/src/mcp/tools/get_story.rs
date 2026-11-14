//! `get_story`: `reeve story context <KEY>` as a tool. Read-only.
use super::{
    Tool, ToolError,
    bound::{MAX_OTHER_TEXT, MAX_OWN_TEXT, MAX_RECENT, cut, cut_goal, most_recent},
};
use crate::{
    actions::{self, Acting},
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{StoryContextDto, StoryContextInput, StoryContextResult, StoryDto};
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
            "Read one story by key (for example {key_prefix}-596): its description, acceptance criteria, readiness and why it is not ready, blockers, required decisions, sprint goals, epic, initiative and its most recent comments. `comments_omitted` is the number of older comments left out. {} Read-only. {}",
            super::CUT_TEXT,
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
/// A blocker's criteria, each goal's texts and the goal outcomes the story and each blocker refer to are cut like
/// every other text that comes along.
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
        mut goals,
        attachments,
    } = context;
    let comments_omitted = most_recent(&mut comments, MAX_RECENT);
    cut(&mut story.description, MAX_OWN_TEXT);
    for criterion in &mut story.criteria {
        cut(&mut criterion.text, MAX_OTHER_TEXT);
    }
    cut_goal_outcomes(&mut story);
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
        cut_goal_outcomes(blocker);
    }
    for comment in &mut comments {
        cut(&mut comment.body, MAX_OTHER_TEXT);
    }
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

/// A story carries the goals it serves as short references (`GoalRefDto`), each with the goal's outcome, which has
/// no length limit: cut like every other text that comes along. Only a whole story carries them, and only
/// `get_story` answers whole stories (the story and its blockers).
fn cut_goal_outcomes(story: &mut StoryDto) {
    for goal in &mut story.goals {
        cut(&mut goal.outcome, MAX_OTHER_TEXT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In memory, so that it needs no sprint or goal link in a database: a context whose blocker
    /// and goal each carry one text over the limit, as do the goal references on the story and the blocker.
    #[test]
    fn every_text_that_comes_along_with_a_story_is_cut() {
        let long = "å".repeat(MAX_OTHER_TEXT + 2);
        let cut_long = format!("{}[cut: 2 more characters]", "å".repeat(MAX_OTHER_TEXT));
        let story = |key: &str| {
            json!({
                "key": key, "title": "A story", "kind": "story", "epic": null, "description": "",
                "criteria": [{"text": "short", "done": false}, {"text": long, "done": false}],
                "owner": null, "labels": [], "rank": 0, "blocked_by": [], "delivery": "todo",
                "resolution": null, "external_ref": null, "version": 1, "prs": [],
                "goals": [
                    {"id": 1, "outcome": "Customers can book", "via": "story"},
                    {"id": 2, "outcome": long, "via": "epic"}
                ]
            })
        };
        let context: StoryContextDto = serde_json::from_value(json!({
            "story": story("TST-1"),
            "epic": null,
            "initiative": null,
            "required_decisions": [],
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
        // The same goals as short references on the story and on each blocker.
        for story in [&sent.story, &sent.blockers[0]] {
            assert_eq!(story.goals[0].outcome, "Customers can book");
            assert_eq!(story.goals[1].outcome, cut_long);
        }
        assert_eq!(sent.goals[0].goal.outcome, "Customers can book");
        assert_eq!(sent.goals[0].goal.notes, cut_long);
    }
}
