pub mod attachment;
pub mod decision;
pub mod findings;
pub mod items;
pub mod progress;
// The sprint list (`previous_end`, `in_sprint`, `SprintPromise`) has no caller until its handler lands; `expect`
// turns into an error once every item is used, so it cannot outlive it.
#[expect(
    dead_code,
    reason = "the sprint promise list, whose handler lands in a later task of the promised-by dates plan"
)]
pub mod promise;
pub mod pull_request;
pub mod queue;
pub mod readiness;
pub mod sprint;
pub mod urgency;
