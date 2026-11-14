//! `list_decisions`: `reeve decision ls` as a tool. Read-only.
use super::{
    Annotations, Tool, ToolError,
    bound::{MAX_WHOLE_LIST, capped, cut_closing_reason, fit},
};
use crate::{
    actions::{self, Acting, DecisionQuery},
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{
    ListDecisionsInput, ListDecisionsResult, RequiredDecisionDto, normalise_email,
};
use serde_json::Value;

pub(super) fn tool() -> Tool {
    Tool {
        name: "list_decisions",
        title: "List required decisions",
        description: format!(
            "List required decisions: choices that must be made and recorded before the work they are on is ready. Use it for what is waiting on a decision, what the person has to decide (mine), the decisions on one story, epic or initiative (on), or every decision anywhere under an initiative (initiative). Give `on` or `initiative`, not both. With neither, only open decisions are listed; with either, closed ones are listed too. Answers `decisions`: up to 50, each whole (except that a long closing reason is cut), with its id, the item it is on, title, decider, due date, reversibility, the linked record and the result of its last check, how and why it was closed if it was, and its version; and `omitted`: the number of decisions left out to keep the answer small (0 when all are listed; when it is not, narrow with `on`, `initiative` or `mine`). Read-only. {}",
            super::PEOPLES_TEXT
        ),
        input_schema: super::input_schema::<ListDecisionsInput>(),
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
        let input: ListDecisionsInput = super::arguments(args)?;
        let prefix = &state.config.key_prefix;
        // The adapter: the two keys are the route's; its two switches stay off, as `reeve decision ls` leaves
        // them. `on` together with `initiative` is refused by the action.
        let query = DecisionQuery {
            on: super::key_ref("on", input.on, prefix)?,
            open_with_record: false,
            include_closed: false,
            initiative: super::key_ref("initiative", input.initiative, prefix)?,
        };
        let mut decisions = actions::list_decisions(state, who, &query)
            .await
            .map_err(super::told)?;
        if input.mine {
            // What `reeve decision ls --mine` does on the client: keep the decisions the caller decides.
            let me = normalise_email(&who.ctx.actor);
            decisions.retain(|d| normalise_email(&d.decider) == me);
        }
        serde_json::to_value(bounded(decisions)).map_err(|e| ToolError::Internal(e.into()))
    })
}

/// The first `MAX_WHOLE_LIST` decisions, whole except for a long closing reason, fitted to the answer budget, and
/// how many were left out by the cap or the budget.
fn bounded(mut decisions: Vec<RequiredDecisionDto>) -> ListDecisionsResult {
    let omitted = capped(&mut decisions, MAX_WHOLE_LIST);
    decisions.iter_mut().for_each(cut_closing_reason);
    let omitted = fit(&mut decisions, omitted);
    ListDecisionsResult { decisions, omitted }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision(id: i64, closed_because: Option<&str>) -> RequiredDecisionDto {
        let closed = closed_because
            .map(|reason| serde_json::json!({"at": "2026-11-01T10:00:00Z", "by": "dan@example.com", "reason": reason}));
        serde_json::from_value(serde_json::json!({
            "id": id, "on": "TST-1", "on_type": "epic", "title": format!("Decision {id}"),
            "decider": "dan@example.com", "due": null, "record": null, "pr_url": null,
            "check": {"kind": "unchecked"}, "checked_at": null, "closed": closed,
            "created_at": "2026-11-01T09:00:00Z", "version": 1
        }))
        .unwrap()
    }

    #[test]
    fn a_list_one_over_the_cap_keeps_the_first_fifty_and_says_one_was_left_out() {
        let at_the_cap = bounded((1..=50).map(|id| decision(id, None)).collect());
        assert_eq!((at_the_cap.decisions.len(), at_the_cap.omitted), (50, 0));
        let one_over = bounded((1..=51).map(|id| decision(id, None)).collect());
        assert_eq!((one_over.decisions.len(), one_over.omitted), (50, 1));
        assert_eq!(
            (one_over.decisions[0].id, one_over.decisions[49].id),
            (1, 50)
        );
    }

    #[test]
    fn a_listed_decision_s_long_closing_reason_is_cut() {
        let long = "å".repeat(2_001);
        let sent = bounded(vec![decision(1, Some(&long)), decision(2, None)]);
        assert_eq!(
            sent.decisions[0]
                .closed
                .as_ref()
                .map(|c| c.reason.chars().count()),
            Some(2_000 + "[cut: 1 more characters]".len())
        );
        assert!(sent.decisions[1].closed.is_none());
    }
}
