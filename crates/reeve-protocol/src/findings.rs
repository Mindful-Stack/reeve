use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Something worth a look on an initiative or an epic. Advisory: computed on read, never stored, and never consulted by
/// a move (kb/ADR-0003 keeps one gate, readiness). Shown by the CLI, the web and quoted by the planning skills.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "finding", rename_all = "snake_case")]
pub enum Finding {
    /// The brief is empty.
    EmptyBrief,
    /// The brief cites Jira or Confluence, or says something was decided, and no required decision is on the item or
    /// above it: a cited page or a "decided" is not a record anyone can check once the source is gone.
    BriefCitesWithoutDecision { cites: Vec<String> },
    /// Discovery is marked complete while a required decision on the item is open and not satisfied.
    DiscoveryCompleteWithOpenDecision { decision_id: i64, title: String },
    /// Discovery is marked complete and nothing has been broken down under the item yet.
    DiscoveryCompleteWithoutStories,
}
