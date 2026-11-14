//! The tool registry: the one list `tools/list` and `tools/call` read (kb/ADR-0029 R10, R12). Empty until the first
//! tool lands (GRA-659).

/// `tools/list`'s `tools` array.
pub(crate) fn list() -> Vec<serde_json::Value> {
    Vec::new()
}
