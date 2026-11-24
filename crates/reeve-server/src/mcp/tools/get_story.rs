//! `get_story`: `reeve story context <KEY>` as a tool. Read-only.
use super::{
    Tool, ToolError,
    bound::{
        MAX_OTHER_TEXT, MAX_OWN_TEXT, MAX_RECENT, bound_links, cut, cut_goal, emptied, most_recent,
        sent_bytes,
    },
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
            "Read one story by key (for example {key_prefix}-596): its description, acceptance criteria, readiness and why it is not ready, blockers, required decisions, sprint goals, epic, initiative and its most recent comments. `comments_omitted` is the number of older comments left out. Its item links to other items are in `story.links`, each read from this story's side with its wording in `label`: they are informational and never hold the story back, unlike blockers; at most 50 are sent, fewer when the story itself is large, and `links_omitted` is the number left out. {} Read-only. {}",
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
    // The story's own item links, last. First the answer as it is sent when none of them is (its baseline): the
    // list empty and every link counted as left out. Then the rows are fitted into what the baseline leaves of
    // one answer. The blockers, the epic and the initiative come along without theirs.
    let mut links = story.links.take();
    let (no_links, all_links) = emptied(&links);
    story.links = no_links;
    let mut result = StoryContextResult {
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
        links_omitted: all_links,
    };
    result.links_omitted = bound_links(&mut links, sent_bytes(&result));
    result.context.story.links = links;
    result
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

    /// The answer `bounded` must give when no item link fits: the list empty and every link counted as left out.
    /// Written out here, not taken from `bound`, so the tests below do not measure the code with itself.
    fn with_no_link_sent(context: &StoryContextDto) -> StoryContextResult {
        let mut context = context.clone();
        let links_omitted = context.story.links.as_ref().map_or(0, Vec::len);
        context.story.links = Some(vec![]);
        StoryContextResult {
            context,
            comments_omitted: 0,
            links_omitted,
        }
    }

    /// A story with three short item links of one size whose answer with none of them sent (`with_no_link_sent`)
    /// weighs exactly `baseline` bytes as `call_tool` measures it. One long label fills it: what fills a story is
    /// not the bound's business, and the label stands in for the criteria, blockers, decisions, goals and
    /// attachments whose number is not capped.
    fn story_weighing(baseline: usize) -> StoryContextDto {
        let links: Vec<Value> = (2..=4)
            .map(|n| {
                json!({"type": "relates_to", "direction": "out", "label": "relates to",
                       "item": {"key": format!("TST-{n}"), "level": "story", "title": "S"}})
            })
            .collect();
        let story = |fill: usize| -> StoryContextDto {
            serde_json::from_value(json!({
                "story": {
                    "key": "TST-1", "title": "A story", "kind": "story", "epic": null,
                    "description": "", "criteria": [], "owner": null, "labels": ["a".repeat(fill)],
                    "rank": 0, "blocked_by": [], "delivery": "todo", "resolution": null, "external_ref": null,
                    "version": 1, "prs": [], "goals": [], "links": links
                },
                "epic": null, "initiative": null, "required_decisions": [], "blockers": [], "comments": [],
                "readiness": {"ready": true, "reasons": []}, "goals": [], "attachments": []
            }))
            .unwrap()
        };
        let unfilled = sent_bytes(&with_no_link_sent(&story(0)));
        let context = story(baseline - unfilled);
        assert_eq!(
            sent_bytes(&with_no_link_sent(&context)),
            baseline,
            "the control: the story weighs what was asked for"
        );
        context
    }

    /// The guarantee of Task 9 at its edge. A story whose baseline is three bytes under the dispatch limit, and one
    /// whose baseline is exactly the limit: neither has room for a link, so the answer is the baseline itself, the
    /// list empty and all three links counted, and the dispatch sends it.
    #[test]
    fn a_story_at_or_just_under_the_limit_is_sent_with_no_item_link_and_all_counted() {
        let limit = crate::mcp::tools::MAX_SENT;
        for under in [3, 0] {
            let context = story_weighing(limit - under);
            let want = with_no_link_sent(&context);
            let sent = bounded(context);
            assert_eq!(
                sent.context.story.links,
                Some(vec![]),
                "{under} under: no link is sent"
            );
            assert_eq!(
                sent.links_omitted, 3,
                "{under} under: all three are counted"
            );
            assert_eq!(
                sent, want,
                "{under} under: and nothing else in the answer changed"
            );
            let bytes = sent_bytes(&sent);
            assert_eq!(
                bytes,
                limit - under,
                "{under} under: the answer weighs what its baseline did"
            );
            assert_eq!(
                crate::mcp::tools::too_large(bytes),
                None,
                "{under} under: the dispatch sends it"
            );
        }
    }

    /// The exact boundary of a row. With exactly one link's bytes between the baseline and the limit, the first
    /// link is sent and the answer is exactly the limit; with one byte less, none is. A story whose baseline is
    /// over the limit is outside the guarantee: its answer is the baseline and the last guard refuses it, as it
    /// refuses an oversized story today.
    #[test]
    fn an_item_link_is_sent_exactly_when_the_baseline_leaves_its_bytes() {
        let limit = crate::mcp::tools::MAX_SENT;
        let row = {
            let context = story_weighing(limit);
            serde_json::to_string(&context.story.links.as_ref().unwrap()[0])
                .unwrap()
                .len()
        };

        let sent = bounded(story_weighing(limit - row));
        let links = sent.context.story.links.as_ref().unwrap();
        assert_eq!(links.len(), 1, "exactly one link's bytes were left");
        assert_eq!(links[0].item.key.to_string(), "TST-2", "the first one");
        assert_eq!(sent.links_omitted, 2);
        let bytes = sent_bytes(&sent);
        assert_eq!(bytes, limit, "the answer is exactly the limit");
        assert_eq!(crate::mcp::tools::too_large(bytes), None, "and is sent");

        let sent = bounded(story_weighing(limit - row + 1));
        assert_eq!(
            sent.context.story.links,
            Some(vec![]),
            "one byte short of a link"
        );
        assert_eq!(sent.links_omitted, 3);
        assert_eq!(sent_bytes(&sent), limit - row + 1);

        let over = story_weighing(limit + 1);
        let want = with_no_link_sent(&over);
        let sent = bounded(over);
        assert_eq!(
            sent, want,
            "over the limit: the list is empty and every link counted"
        );
        assert!(
            crate::mcp::tools::too_large(sent_bytes(&sent)).is_some(),
            "and the last guard refuses it: the bound promises nothing for a baseline over the limit"
        );
    }

    /// The property of Task 9 for `get_story`: the story's item links never push the answer over the limit of one
    /// answer. A description of the maximum length it is sent at, in four-byte characters (40 000 bytes), and 200
    /// item links whose titles JSON escapes sixfold.
    #[test]
    fn a_story_at_its_longest_with_200_item_links_fits_one_answer() {
        let escaped = "\u{1}".repeat(200);
        let links: Vec<Value> = (2..=201)
            .map(|n| {
                json!({"type": "relates_to", "direction": "out", "label": "relates to",
                       "item": {"key": format!("TST-{n}"), "level": "story", "title": escaped}})
            })
            .collect();
        let context: StoryContextDto = serde_json::from_value(json!({
            "story": {
                "key": "TST-1", "title": "A story", "kind": "story", "epic": null,
                "description": "𝄞".repeat(MAX_OWN_TEXT), "criteria": [], "owner": null, "labels": [],
                "rank": 0, "blocked_by": [], "delivery": "todo", "resolution": null, "external_ref": null,
                "version": 1, "prs": [], "goals": [], "links": links
            },
            "epic": null, "initiative": null, "required_decisions": [], "blockers": [], "comments": [],
            "readiness": {"ready": true, "reasons": []}, "goals": [], "attachments": []
        }))
        .unwrap();
        let limit = crate::mcp::tools::MAX_SENT;
        let sent = bounded(context);
        let story = &sent.context.story;
        assert_eq!(
            story.description.len(),
            4 * MAX_OWN_TEXT,
            "the control: the description is whole, 40 000 bytes"
        );
        let kept = story.links.as_ref().unwrap();
        assert!(!kept.is_empty() && kept.len() < 50, "{} sent", kept.len());
        assert_eq!(
            kept.len() + sent.links_omitted,
            200,
            "every link is either sent or counted"
        );
        assert_eq!(kept[0].item.key.to_string(), "TST-2", "the first are kept");
        assert_eq!(kept[0].item.title, escaped, "a kept link is whole");
        // As `call_tool` measures it.
        let bytes = serde_json::to_value(&sent).unwrap().to_string().len();
        assert!(bytes <= limit, "{bytes} bytes");
        assert!(
            bytes > limit - 2_000,
            "and the room is used, to within one link: {bytes} bytes"
        );
    }
}
