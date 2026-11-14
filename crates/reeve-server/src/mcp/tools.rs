//! The tool registry: the one list `tools/list` and `tools/call` both read. Each tool corresponds to a `reeve` CLI
//! command and runs the action its API route runs (kb/ADR-0029 R1, kb/ADR-0005 R5); a tool outside the set of
//! kb/ADR-0029 R10 needs an observation on that record first.
mod get_story;

use crate::{actions::Acting, error::AppError, state::AppState};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

/// Why a tool call did not produce its result.
pub(crate) enum ToolError {
    /// The arguments do not fit the tool's input type: the deserializer's message. `call_tool` says it to the
    /// model together with the tool's input schema.
    Arguments(String),
    /// Said to the model as it is, as a tool result with `isError: true`.
    Told(String),
    /// Logged and answered 500 with no detail.
    Internal(anyhow::Error),
}

/// A tool's body: deserialise the arguments into the tool's input type, turn them into what the route's action
/// takes, run the action, serialise the result.
type Run =
    for<'a> fn(&'a AppState, &'a Acting<'a>, Value) -> BoxFuture<'a, Result<Value, ToolError>>;

/// The four hints the specification's `ToolAnnotations` defines. Every tool states all four, and all four are in
/// the snapshot.
pub(crate) struct Annotations {
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
    pub open_world: bool,
}

pub(crate) struct Tool {
    pub name: &'static str,
    /// The name a person sees.
    pub title: &'static str,
    pub description: String,
    /// Generated from the tool's `reeve-protocol` input type (kb/ADR-0029 R12).
    pub input_schema: Value,
    pub annotations: Annotations,
    pub run: Run,
}

/// Every tool, in a fixed order. `key_prefix` is the instance's, for the examples in descriptions.
pub(crate) fn registry(key_prefix: &str) -> Vec<Tool> {
    vec![get_story::tool(key_prefix)]
}

/// `tools/list`'s `tools` array. The field names are the same in the 2026-07-28 and the 2025-11-25 schema.
pub(crate) fn list(key_prefix: &str) -> Vec<Value> {
    registry(key_prefix)
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "title": t.title,
                "description": t.description,
                "inputSchema": t.input_schema,
                "annotations": {
                    "readOnlyHint": t.annotations.read_only,
                    "destructiveHint": t.annotations.destructive,
                    "idempotentHint": t.annotations.idempotent,
                    "openWorldHint": t.annotations.open_world
                }
            })
        })
        .collect()
}

/// The JSON Schema utoipa generates for a tool's input type.
fn input_schema<T: utoipa::PartialSchema>() -> Value {
    serde_json::to_value(T::schema()).expect("a schema serialises")
}

/// A call's arguments as the tool's input type, or the deserializer's message.
fn arguments<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, ToolError> {
    serde_json::from_value(args).map_err(|e| ToolError::Arguments(e.to_string()))
}

/// What the planner said no with, as the model reads it. Anything else is an internal error. (A tool whose "not
/// found" has a better wording, as `get_story`'s has, says it itself before calling this.)
fn told(e: AppError) -> ToolError {
    match e {
        AppError::Validation(message) => ToolError::Told(message),
        AppError::Refused(reasons) => ToolError::Told(format!(
            "Refused: {}",
            serde_json::to_string(&reasons).unwrap_or_default()
        )),
        AppError::Internal(e) => ToolError::Internal(e),
        other => ToolError::Internal(anyhow::anyhow!("an action failed with {other:?}")),
    }
}
