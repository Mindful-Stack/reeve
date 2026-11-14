//! `list_epics`: `reeve epic ls` as a tool. Read-only.
use super::{
    Annotations, Tool, ToolError,
    bound::{MAX_LIST, capped, fit},
};
use crate::{
    actions::{self, Acting, EpicQuery},
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{EpicDto, EpicSummary, ListEpicsInput, ListEpicsResult};
use serde_json::Value;

pub(super) fn tool() -> Tool {
    Tool {
        name: "list_epics",
        title: "List epics",
        description: format!(
            "List epics: all of them, or those of one initiative. Use it to see how an initiative is broken down, or to find the epic a piece of work belongs under. Answers `epics`: up to 200 summaries, oldest first, each with key, title, initiative, version, discovery_complete and promised_on; and `omitted`: the number of epics left out to keep the answer small (0 when all are listed; when it is not, narrow with `initiative` or use search_items). A summary has no brief: read one epic with get_epic, and its stories with list_stories and `epic`. Read-only. {}",
            super::PEOPLES_TEXT
        ),
        input_schema: super::input_schema::<ListEpicsInput>(),
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
        let input: ListEpicsInput = super::arguments(args)?;
        // The adapter: the tool's `initiative` is the route's query parameter.
        let query = EpicQuery {
            initiative: super::key_ref("initiative", input.initiative, &state.config.key_prefix)?,
        };
        let epics = actions::list_epics(state, who, &query)
            .await
            .map_err(super::told)?;
        serde_json::to_value(bounded(epics)).map_err(|e| ToolError::Internal(e.into()))
    })
}

/// The first `MAX_LIST` epics as summaries, fitted to the answer budget, and how many were left out by either.
fn bounded(mut epics: Vec<EpicDto>) -> ListEpicsResult {
    let omitted = capped(&mut epics, MAX_LIST);
    let mut epics: Vec<EpicSummary> = epics.into_iter().map(EpicSummary::from).collect();
    let omitted = fit(&mut epics, omitted);
    ListEpicsResult { epics, omitted }
}

#[cfg(test)]
mod tests {
    use super::super::bound::MAX_ANSWER;
    use super::*;

    fn epic(n: u64) -> EpicDto {
        serde_json::from_value(serde_json::json!({
            "key": format!("TST-{n}"), "title": format!("Epic {n}"), "initiative": null, "brief": "",
            "discovery_complete": null, "external_ref": null, "version": 1
        }))
        .unwrap()
    }

    /// The cap of 200 and the budget, tested where it is cheap: rows in memory, not 201 creates through HTTP.
    #[test]
    fn a_list_over_the_cap_or_the_budget_keeps_the_first_rows_and_counts_the_rest() {
        let at_the_cap = bounded((1..=MAX_LIST as u64).map(epic).collect());
        assert_eq!((at_the_cap.epics.len(), at_the_cap.omitted), (MAX_LIST, 0));
        let one_over = bounded((1..=MAX_LIST as u64 + 1).map(epic).collect());
        assert_eq!((one_over.epics.len(), one_over.omitted), (200, 1));
        assert_eq!(one_over.epics[0].key.to_string(), "TST-1");
        assert_eq!(one_over.epics[199].key.to_string(), "TST-200");

        // Rows too large for the budget (titles no planner would accept) are left out after the cap, and counted
        // with the capped one.
        let wide = bounded(
            (1..=MAX_LIST as u64 + 1)
                .map(|n| {
                    let mut epic = epic(n);
                    epic.title = "t".repeat(400);
                    epic
                })
                .collect(),
        );
        assert!(wide.omitted > 1, "{}", wide.omitted);
        assert_eq!(wide.epics.len() + wide.omitted, MAX_LIST + 1);
        assert_eq!(wide.epics[0].key.to_string(), "TST-1");
        assert!(serde_json::to_string(&wide.epics).unwrap().len() <= MAX_ANSWER);
    }
}
