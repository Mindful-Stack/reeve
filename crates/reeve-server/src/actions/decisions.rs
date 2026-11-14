use super::Acting;
use crate::{error::AppError, state::AppState};
use reeve_planning::{DecisionScope, HandlerError, Handlers, RequiredDecision};
use reeve_protocol::{ItemType, Key, KeyRef, RequiredDecisionDto};
use serde::Deserialize;

/// The query of `GET /api/v1/decisions`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct DecisionQuery {
    /// A story, epic or initiative key or external reference. A reference that matches nothing yields an empty list, not 404.
    pub(crate) on: Option<KeyRef>,
    #[serde(default)]
    pub(crate) open_with_record: bool,
    /// Without `on`, also list closed decisions (a list scoped by `on` always includes them).
    #[serde(default)]
    pub(crate) include_closed: bool,
    /// Every decision under this initiative: on it, its epics and their stories, closed ones included. Not
    /// with `on`.
    pub(crate) initiative: Option<KeyRef>,
}

/// A decision target is untyped: `Handlers::resolve_any` checks native keys of every item type before any external
/// reference.
pub(crate) async fn resolve_on(h: &Handlers, raw: &KeyRef) -> Result<Key, HandlerError> {
    h.resolve_any(raw.as_str()).await.map(|(_, k)| k)
}

/// The one place a decision becomes a response: its record link gains `browse_url` from the repository's `https://`
/// URL in configuration (planning knows names only). Every route that returns a decision goes through it.
pub(crate) fn decision_dto(s: &AppState, d: &RequiredDecision) -> RequiredDecisionDto {
    let mut dto = RequiredDecisionDto::from(d);
    super::browse(s, &mut dto);
    dto
}

/// The scope `on`/`initiative`/`include_closed` names. `None`: a reference that matches nothing, answered as an
/// empty list, not 404. Both given is a validation error. The list and the queue resolve through this one function,
/// so an unknown key behaves the same for both.
pub(crate) async fn decision_scope(
    h: &Handlers,
    on: Option<&KeyRef>,
    initiative: Option<&KeyRef>,
    include_closed: bool,
) -> Result<Option<DecisionScope>, AppError> {
    if let Some(r) = initiative {
        if on.is_some() {
            return Err(HandlerError::Validation("give on or initiative, not both".into()).into());
        }
        return match h.resolve_ref(r.as_str(), ItemType::Initiative).await {
            Ok(k) => Ok(Some(DecisionScope::Initiative(k))),
            Err(HandlerError::NotFound) => Ok(None),
            Err(e) => Err(e.into()),
        };
    }
    match on {
        Some(r) => match resolve_on(h, r).await {
            Ok(k) => Ok(Some(DecisionScope::On(k))),
            Err(HandlerError::NotFound) => Ok(None),
            Err(e) => Err(e.into()),
        },
        None => Ok(Some(DecisionScope::All { include_closed })),
    }
}

/// `reeve decision ls`: required decisions, on one item, under one initiative, or all open ones. Shared by
/// `GET /api/v1/decisions` and the MCP tool `list_decisions`. `q` is what the route takes: its query.
pub(crate) async fn list_decisions(
    state: &AppState,
    _who: &Acting<'_>,
    q: &DecisionQuery,
) -> Result<Vec<RequiredDecisionDto>, AppError> {
    let rows = match decision_scope(
        &state.handlers,
        q.on.as_ref(),
        q.initiative.as_ref(),
        q.include_closed,
    )
    .await?
    {
        None => Vec::new(),
        Some(DecisionScope::Initiative(k)) => {
            match state.handlers.decisions_under_initiative(&k).await {
                Ok(rows) => rows,
                Err(HandlerError::NotFound) => Vec::new(),
                Err(e) => return Err(e.into()),
            }
        }
        Some(DecisionScope::On(k)) => {
            state
                .handlers
                .list_decisions(Some(&k), q.open_with_record, q.include_closed)
                .await?
        }
        Some(DecisionScope::All { include_closed }) => {
            state
                .handlers
                .list_decisions(None, q.open_with_record, include_closed)
                .await?
        }
    };
    Ok(rows.iter().map(|d| decision_dto(state, d)).collect())
}
