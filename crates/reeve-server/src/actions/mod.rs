//! Actions: everything between the caller check and the response, shared by an API route and its MCP tool, so the
//! two run the same code and the same validation (kb/ADR-0029 R1). Every action has one signature:
//!
//! ```text
//! async fn <name>(state: &AppState, who: &Acting<'_>, <the input its route takes>) -> Result<<Dto>, AppError>
//! ```
//!
//! The input is what the route takes (its path, its query struct or its body), not the tool's type: the route
//! passes what it parsed, and a tool is an adapter that turns its own narrower input into the route's and calls the
//! same action. A check that sits in a route function moves into the action when that route gets a tool.
mod search;
mod story_context;

use crate::state::AppState;
use chrono::{DateTime, Utc};
use reeve_identity::AccessPolicy;
use reeve_protocol::{RequiredDecisionDto, Role, Via};
pub(crate) use search::{SearchQuery, search};
pub(crate) use story_context::story_context;

/// What an action may need to know about the caller: planning's context (actor, channel, the request's one
/// timestamp), the caller's role, and the access policy the request was authenticated with. Role checks and owner
/// validation against the policy need the last two; they cannot be recovered from an input.
///
/// `role` and `policy` are private to `actions`, so an `Acting` is built in one place, `Acting::new`.
pub(crate) struct Acting<'a> {
    pub(crate) ctx: reeve_planning::Ctx,
    #[allow(dead_code)] // read by the first action that checks a role (GRA-661)
    role: Role,
    #[allow(dead_code)] // read by the first action that validates an owner (GRA-661)
    policy: &'a AccessPolicy,
}

impl<'a> Acting<'a> {
    /// The one constructor: `Caller::acting` and `McpCaller::acting` both call it, so the two cannot differ.
    pub(crate) fn new(
        email: &str,
        via: Via,
        role: Role,
        now: DateTime<Utc>,
        policy: &'a AccessPolicy,
    ) -> Self {
        Self {
            ctx: reeve_planning::Ctx {
                actor: email.to_string(),
                via,
                now,
            },
            role,
            policy,
        }
    }
}

/// Sets `browse_url` on an already-built decision DTO: `<repo url>/blob/HEAD/<path>`, nothing for a repository
/// without an `https://` URL.
pub(crate) fn browse(s: &AppState, dto: &mut RequiredDecisionDto) {
    if let Some(r) = dto.record.as_mut() {
        r.browse_url = s.repo_urls.get(&r.repo).map(|base| {
            let path = r
                .path
                .trim_start_matches('/')
                .split('/')
                .map(|seg| urlencoding::encode(seg).into_owned())
                .collect::<Vec<_>>()
                .join("/");
            format!("{base}/blob/HEAD/{path}")
        });
    }
}
