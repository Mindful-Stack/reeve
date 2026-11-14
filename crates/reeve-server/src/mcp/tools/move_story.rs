//! `move_story`: `reeve story mv <KEY>` as a tool. Changes the planner. It can never override the readiness gate
//! (kb/ADR-0029 R10): its input has no override reason, and it always passes none.
use super::{Annotations, Tool, ToolError, bound};
use crate::{
    actions::{self, Acting},
    error::AppError,
    state::AppState,
};
use futures_util::future::BoxFuture;
use reeve_protocol::{MoveStory, MoveStoryInput, Reason, StoryReceipt};
use serde_json::{Value, json};

pub(super) fn tool(key_prefix: &str) -> Tool {
    let mut input_schema = super::input_schema::<MoveStoryInput>();
    input_schema["properties"]["key"]["description"] =
        json!(format!("The story's key, for example {key_prefix}-596."));
    Tool {
        name: "move_story",
        title: "Move a story",
        description: format!(
            "Move one story to another delivery state: todo, in_progress, in_review, testing or done. {} Starting work (from todo or done into in_progress, in_review or testing) needs the story to be ready; if it is not, the move is refused, you are told why, and this tool cannot override that: a person can, in the CLI or the web client. Tell the person the reasons; do not look for another way to move it; fixing what the reasons name, when the person asks, is fine. Moving a story with no owner into work makes the person you act for its owner. Done needs a `resolution`. Send the story's `version` as you last read it. Answers a receipt: the story's key, title, new delivery state, resolution and version.",
            super::WRITE_TEXT
        ),
        input_schema,
        // It changes the story's state and may set its owner (destructive: not only an addition). The same
        // arguments again carry a stale version, or name the state the story is in, and change nothing
        // (idempotent). Nothing outside this planner.
        annotations: Annotations {
            read_only: false,
            destructive: true,
            idempotent: true,
            open_world: false,
        },
        run,
    }
}

/// What the model is told when the planner refused a move: its reasons as `tools::listed` lists them, whether a
/// person could override (only when every reason is a readiness fact), and that nothing changed. The key is
/// repeated with `bound::echo` and the reasons are bounded by `listed`, so the whole is within `bound::MAX_OWN_TEXT`
/// and the closing sentences always come last.
fn refused(key: &str, reasons: &[Reason]) -> String {
    let listed = super::listed(reasons);
    let not_ready = !reasons.is_empty() && reasons.iter().all(Reason::overridable);
    let (why, who_can) = if not_ready {
        (
            ": it is not ready",
            " A person can override this in the CLI or the web client; this tool cannot.",
        )
    } else {
        ("", "")
    };
    format!(
        "{} was not moved{why}. Reasons, as get_story lists them: {listed}.{who_can} Nothing changed.",
        bound::echo(key)
    )
}

fn run<'a>(
    state: &'a AppState,
    who: &'a Acting<'a>,
    args: Value,
) -> BoxFuture<'a, Result<Value, ToolError>> {
    Box::pin(async move {
        // `deny_unknown_fields`: a call that sends an override reason is refused here, before anything runs.
        let input: MoveStoryInput = super::arguments(args)?;
        // The route's body, every field written out. The override reason is always none: kb/ADR-0029 R10. (The
        // action refuses one from a caller that came through MCP, should an adapter ever pass it.)
        let body = MoveStory {
            version: input.version,
            to: input.to,
            resolution: input.resolution,
            override_reason: None,
        };
        let story = actions::move_story(state, who, &input.key, body)
            .await
            .map_err(|e| match e {
                AppError::Refused(reasons) => ToolError::Told(refused(&input.key, &reasons)),
                other => super::write_failed(other, &input.key, &state.config.key_prefix),
            })?;
        super::sent(StoryReceipt::from(story))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const OVERRIDE_AND_NOTHING: &str = ". A person can override this in the CLI or the web client; this tool cannot. Nothing changed.";

    fn no_record(title: String) -> Reason {
        Reason::DecisionNoRecord {
            decision_id: 7,
            title,
        }
    }

    /// A refused move as `call_tool` sends it, checked for what every refusal keeps: its closing sentences whole
    /// and last, its length within the bound with every marker, and each reason it shows either whole (it parses)
    /// or cut on its own, within the other-text bound with its marker. Returns the reasons shown.
    fn kept(key: &str, reasons: &[Reason]) -> Vec<String> {
        let said = refused(key, reasons);
        assert_eq!(
            super::super::bounded(said.clone()),
            said,
            "the last guard cut it"
        );
        assert!(
            said.chars().count() <= bound::MAX_OWN_TEXT,
            "{} characters",
            said.chars().count()
        );
        assert!(
            said.ends_with(OVERRIDE_AND_NOTHING),
            "{}",
            &said[said.len() - 300..]
        );
        let start = said.find("lists them: [").expect("a list") + "lists them: [".len();
        let end = said.len() - OVERRIDE_AND_NOTHING.len();
        let mut list = &said[start..end];
        list = match list.find("] … and ") {
            Some(more) => &list[..more],
            None => list.strip_suffix(']').expect("the list's end"),
        };
        let shown: Vec<String> = list.split(", {").map(str::to_string).collect();
        for (n, reason) in shown.iter().enumerate() {
            let reason = if n == 0 {
                reason.clone()
            } else {
                format!("{{{reason}")
            };
            let whole = serde_json::from_str::<Reason>(&reason).is_ok();
            let cut = reason.ends_with(" more characters]")
                && reason.chars().count() <= bound::MAX_OTHER_TEXT;
            assert!(whole || cut, "reason {n}: {}", &reason[reason.len() - 60..]);
        }
        shown
    }

    #[test]
    fn a_reason_with_an_unbroken_long_title_is_cut_on_its_own() {
        let shown = kept("TST-5", &[no_record("t".repeat(3_000))]);
        assert_eq!(shown.len(), 1);
        assert!(shown[0].ends_with(" more characters]"));
    }

    /// A key of 203 characters, one title of 3 000 characters and four of 1 942: each reason is cut on its own,
    /// the reasons that do not fit are counted, and the closing sentences survive. In ASCII and in multibyte text.
    #[test]
    fn a_long_key_and_long_titles_stay_within_the_bound_and_end_with_nothing_changed() {
        let key = format!("TST-{}", "1".repeat(199));
        for letter in ["t", "å", "世", "🙂"] {
            let mut reasons = vec![no_record(letter.repeat(3_000))];
            reasons.extend((0..4).map(|_| no_record(letter.repeat(1_942))));
            let shown = kept(&key, &reasons);
            assert!(shown[0].ends_with(" more characters]"), "{letter}");
            assert!(shown.len() < reasons.len(), "{letter}");
        }
    }

    /// Two thousand reasons: whole trailing reasons are left out and counted.
    #[test]
    fn reasons_that_do_not_fit_are_left_out_whole_and_counted() {
        let many: Vec<Reason> = (0..2_000)
            .map(|_| no_record("Which queue".repeat(5)))
            .collect();
        let shown = kept("TST-5", &many);
        let said = refused("TST-5", &many);
        assert!(
            said.contains(&format!("] … and {} more reasons", 2_000 - shown.len())),
            "{}",
            &said[said.len() - 300..]
        );
    }

    #[test]
    fn a_refusal_says_a_person_can_override_only_when_every_reason_is_a_readiness_fact() {
        let key = "TST-5".parse().unwrap();
        assert_eq!(
            refused(
                "TST-5",
                &[
                    Reason::Untriaged,
                    Reason::AncestorDiscoveryIncomplete { item: key }
                ]
            ),
            "TST-5 was not moved: it is not ready. Reasons, as get_story lists them: [{\"reason\":\"untriaged\"}, {\"reason\":\"ancestor_discovery_incomplete\",\"item\":\"TST-5\"}]. A person can override this in the CLI or the web client; this tool cannot. Nothing changed."
        );
        for rule in [
            Reason::ResolutionRequired,
            Reason::CompletedNeedsActiveOrigin,
        ] {
            let said = refused("TST-5", std::slice::from_ref(&rule));
            assert!(
                !said.contains("override") && said.ends_with("Nothing changed."),
                "{said}"
            );
        }
    }

    /// Thirty blockers with an owner each make a list of reasons longer than the other-text bound, and no reason
    /// has a title to put a space in. Bounded as `call_tool` sends it, every blocker's key is still there.
    #[test]
    fn a_refusal_keeps_every_reason_of_a_long_list_without_titles() {
        let blockers: Vec<Reason> = (100..130)
            .map(|n| Reason::BlockedBy {
                story: format!("TST-{n}").parse().unwrap(),
                delivery: reeve_protocol::DeliveryState::InProgress,
                owner: Some("someone.with.a.long.name@example.com".into()),
            })
            .collect();
        let sent = super::super::bounded(refused("TST-5", &blockers));
        assert!(
            sent.chars().count() > bound::MAX_OTHER_TEXT,
            "the case is a long refusal"
        );
        for n in 100..130 {
            assert!(sent.contains(&format!("\"TST-{n}\"")), "TST-{n}: {sent}");
        }
        assert!(sent.ends_with("Nothing changed."), "{sent}");
    }
}
