//! `list_stories`: `reeve story ls` as a tool. Read-only.
use super::{Tool, ToolError, bound::fit};
use crate::{
    actions::{self, Acting, StoryQuery},
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{
    DoneFilter, ListStoriesInput, ListStoriesResult, StorySort, StorySummary, StoryWithReadiness,
};
use serde_json::Value;

/// The page when `limit` is left out, and the largest a chat may ask for. The route allows 100; a tool's answer
/// is sent twice, so its page is smaller.
const DEFAULT_LIMIT: u32 = 25;
const MAX_LIMIT: u32 = 50;

pub(super) fn tool() -> Tool {
    Tool {
        name: "list_stories",
        title: "List stories",
        description: format!(
            "List stories (stories, bugs and chores), filtered, one page at a time, in backlog order: by delivery state, then rank. Use it for questions such as: what is on my plate (mine and open), what can be picked up next (ready), what is in an epic or under an initiative, what is urgent, what is promised and by when (promised), what nobody has placed yet (untriaged), what serves the sprint's goals (goal), or what a named person is working on (owner). Filters combine: every one given must hold. Answers `stories`: one page of summaries, each with key, title, kind, epic, owner, delivery, resolution, labels, version, blocked_by, criteria_done and criteria_total, ready and the reasons when it is not, urgent, and promised_on; and `next_offset`: a number when there may be more, because the page is full or stories were left out to keep the answer small (call again with the same filters and that `offset`), null on the last page; a story too large to list is skipped. A summary has no description or comments: read one story with get_story. Read-only. {}",
            super::PEOPLES_TEXT
        ),
        input_schema: super::input_schema::<ListStoriesInput>(),
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
        let input: ListStoriesInput = super::arguments(args)?;
        let prefix = &state.config.key_prefix;
        let limit = input.limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(ToolError::Told(format!("limit must be 1 to {MAX_LIMIT}.")));
        }
        let offset = input.offset.unwrap_or(0);
        // `mine` is the CLI's `--mine`: the CLI asks the server who is calling and sends that as `owner`.
        let owner = match (input.mine, input.owner) {
            (true, Some(_)) => {
                return Err(ToolError::Told("Give mine or owner, not both.".into()));
            }
            (true, None) => Some(who.ctx.actor.clone()),
            (false, owner) => owner,
        };
        // The adapter: every field of the route's query is named, so a field added to it does not compile here
        // until someone decides what a tool sends for it.
        let query = StoryQuery {
            epic: super::key_ref("epic", input.epic, prefix)?,
            owner,
            delivery: input.delivery,
            untriaged: input.untriaged,
            label: input.label,
            ready: input.ready,
            // `open` is the CLI's `--open`.
            done: if input.open {
                DoneFilter::None
            } else {
                DoneFilter::All
            },
            goal: input.goal,
            q: input.text,
            kind: input.kind,
            initiative: super::key_ref("initiative", input.initiative, prefix)?,
            urgent: input.urgent,
            promised: input.promised,
            // No `sort` argument: a tool's list is in backlog order.
            sort: StorySort::Rank,
            limit: Some(limit),
            offset: Some(offset),
        };
        let stories = actions::list_stories(state, who, &query)
            .await
            .map_err(super::told)?;
        super::sent(paged(stories, offset, limit))
    })
}

/// The route's page as summaries, fitted to the answer budget, and where the next page starts: after the stories
/// sent, when the route's page was full (it may have more after it: the route's rule) or the budget left stories
/// out. A page shortened by the budget is thus followed by the first story it left out. A story too large to send
/// even alone is skipped: the next page starts after it, so it cannot stop the paging.
fn paged(rows: Vec<StoryWithReadiness>, offset: u32, limit: u32) -> ListStoriesResult {
    let full = rows.len() == limit as usize;
    let mut stories: Vec<StorySummary> = rows.into_iter().map(StorySummary::from).collect();
    let dropped = fit(&mut stories, 0);
    let next_offset = if full || dropped > 0 {
        offset.checked_add((stories.len() as u32).max(1))
    } else {
        None
    };
    ListStoriesResult {
        stories,
        next_offset,
    }
}

#[cfg(test)]
mod tests {
    use super::super::bound::MAX_ANSWER;
    use super::*;
    use serde_json::json;

    /// A row whose one label has `label` characters: in memory a label has no limit, so a row can be any size.
    fn row(n: u64, label: usize) -> StoryWithReadiness {
        serde_json::from_value(json!({
            "story": {
                "key": format!("TST-{n}"), "title": "A story", "kind": "story", "epic": null,
                "description": "", "criteria": [], "owner": null, "labels": ["l".repeat(label)],
                "rank": 0, "blocked_by": [], "delivery": "todo", "resolution": null,
                "external_ref": null, "version": 1, "prs": []
            },
            "readiness": {"ready": true, "reasons": []}
        }))
        .unwrap()
    }

    #[test]
    fn the_next_page_starts_after_the_stories_sent() {
        // A full page, nothing dropped: there may be more, after `limit` stories.
        let full = paged((1..=3).map(|n| row(n, 1)).collect(), 10, 3);
        assert_eq!((full.stories.len(), full.next_offset), (3, Some(13)));
        // A short page is the last one.
        let short = paged((1..=2).map(|n| row(n, 1)).collect(), 10, 3);
        assert_eq!((short.stories.len(), short.next_offset), (2, None));
        // Fifty rows of about 2 300 bytes do not fit the budget: the first k are sent, and the next page starts
        // with the first one left out.
        let wide = paged((1..=50).map(|n| row(n, 2_000)).collect(), 100, 50);
        let k = wide.stories.len();
        assert!(0 < k && k < 50, "{k}");
        assert_eq!(wide.stories[0].key.to_string(), "TST-1");
        assert!(serde_json::to_string(&wide.stories).unwrap().len() <= MAX_ANSWER);
        assert_eq!(wide.next_offset, Some(100 + k as u32));
        // A first story too large to send alone is skipped, on a page that was not even full.
        let huge = paged(vec![row(1, MAX_ANSWER), row(2, 1)], 100, 25);
        assert_eq!((huge.stories.len(), huge.next_offset), (0, Some(101)));
    }
}
