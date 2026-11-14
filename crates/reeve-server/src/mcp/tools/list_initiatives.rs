//! `list_initiatives`: `reeve initiative ls` as a tool. Read-only.
use super::{
    Annotations, Tool, ToolError,
    bound::{MAX_LIST, capped, fit},
};
use crate::{
    actions::{self, Acting},
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{
    InitiativeDto, InitiativeSummary, ListInitiativesInput, ListInitiativesResult,
};
use serde_json::Value;

pub(super) fn tool() -> Tool {
    Tool {
        name: "list_initiatives",
        title: "List initiatives",
        description: format!(
            "List every initiative: the top level of the plan, above epics and stories. Use it to answer what the team is working towards, or to find an initiative's key. It takes no arguments. Answers `initiatives`: up to 200 summaries, oldest first, each with key, title, horizon, target_month, owner, version, discovery_complete and progress (the share of its stories completed, of those not dropped: 0 to 1, or null when nothing counts yet); and `omitted`: the number of initiatives left out to keep the answer small (0 when all are listed; when it is not, find the others with search_items). A summary has no brief: read one initiative with get_initiative, and its epics with list_epics and `initiative`. Read-only. {}",
            super::PEOPLES_TEXT
        ),
        input_schema: super::input_schema::<ListInitiativesInput>(),
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
        // No arguments to adapt; deserializing still refuses any that are sent.
        let _: ListInitiativesInput = super::arguments(args)?;
        let initiatives = actions::list_initiatives(state, who)
            .await
            .map_err(super::told)?;
        serde_json::to_value(bounded(initiatives)).map_err(|e| ToolError::Internal(e.into()))
    })
}

/// The first `MAX_LIST` initiatives as summaries, fitted to the answer budget, and how many were left out by
/// either.
fn bounded(mut initiatives: Vec<InitiativeDto>) -> ListInitiativesResult {
    let omitted = capped(&mut initiatives, MAX_LIST);
    let mut initiatives: Vec<InitiativeSummary> = initiatives
        .into_iter()
        .map(InitiativeSummary::from)
        .collect();
    let omitted = fit(&mut initiatives, omitted);
    ListInitiativesResult {
        initiatives,
        omitted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn initiative(n: u64) -> InitiativeDto {
        serde_json::from_value(serde_json::json!({
            "key": format!("TST-{n}"), "title": format!("Initiative {n}"), "brief": "", "horizon": "Now",
            "target_month": null, "owner": null, "discovery_complete": null, "progress": null,
            "external_ref": null, "version": 1
        }))
        .unwrap()
    }

    #[test]
    fn a_list_one_over_the_cap_keeps_the_first_two_hundred_and_says_one_was_left_out() {
        let at_the_cap = bounded((1..=MAX_LIST as u64).map(initiative).collect());
        assert_eq!(
            (at_the_cap.initiatives.len(), at_the_cap.omitted),
            (MAX_LIST, 0)
        );
        let one_over = bounded((1..=MAX_LIST as u64 + 1).map(initiative).collect());
        assert_eq!((one_over.initiatives.len(), one_over.omitted), (200, 1));
        assert_eq!(one_over.initiatives[0].key.to_string(), "TST-1");
        assert_eq!(one_over.initiatives[199].key.to_string(), "TST-200");
    }
}
