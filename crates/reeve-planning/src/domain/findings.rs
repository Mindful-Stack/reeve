//! Advisory findings on an initiative or an epic (`reeve_protocol::Finding`): a pure function of facts read together,
//! computed on every read and never stored, so a later trigger (an event, a schedule) can call it as it is.
use super::decision::RequiredDecision;
use reeve_protocol::Finding;

/// The facts one item's findings rest on.
#[derive(Debug, Clone)]
pub struct FindingsInput<'a> {
    pub brief: &'a str,
    pub discovery_complete: bool,
    /// Required decisions on the item itself.
    pub own_decisions: &'a [RequiredDecision],
    /// Whether any required decision, open or closed, is on an item above it (an epic's initiative).
    pub ancestor_has_decision: bool,
    /// Stories under the item (an initiative: under its epics).
    pub stories: u32,
}

pub fn compute_findings(i: &FindingsInput) -> Vec<Finding> {
    let mut out = vec![];
    if i.brief.trim().is_empty() {
        out.push(Finding::EmptyBrief);
    }
    let cites = citations(i.brief);
    if !cites.is_empty() && i.own_decisions.is_empty() && !i.ancestor_has_decision {
        out.push(Finding::BriefCitesWithoutDecision { cites });
    }
    if i.discovery_complete {
        for d in i.own_decisions {
            if d.is_open() && !d.is_satisfied() {
                out.push(Finding::DiscoveryCompleteWithOpenDecision {
                    decision_id: d.id(),
                    title: d.title().to_string(),
                });
            }
        }
        if i.stories == 0 {
            out.push(Finding::DiscoveryCompleteWithoutStories);
        }
    }
    out
}

/// What a brief rests on that is not a checkable record: Jira, Confluence (by name or by an Atlassian link), or a
/// "decided". Whole words only, so "undecided" or "jiralike" is not a citation.
fn citations(brief: &str) -> Vec<String> {
    let lower = brief.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).collect();
    let has = |w: &str| words.contains(&w);
    let mut out = vec![];
    if has("jira") || lower.contains("atlassian.net/browse/") {
        out.push("Jira".to_string());
    }
    if has("confluence") || lower.contains("atlassian.net/wiki") {
        out.push("Confluence".to_string());
    }
    if has("decided") {
        out.push("decided".to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use reeve_protocol::{CheckResult, Key};

    fn decision(id: i64, check: CheckResult, closed: bool) -> RequiredDecision {
        RequiredDecision::from_persisted(
            id,
            "ABC-1".parse::<Key>().unwrap(),
            reeve_protocol::ItemType::Initiative,
            format!("Pick {id}"),
            "dan@example.com".into(),
            None,
            None,
            None,
            None,
            check,
            None,
            closed.then(|| super::super::decision::Closed {
                at: Utc::now(),
                by: "dan@example.com".into(),
                reason: "moot".into(),
                kind: None,
                via: None,
            }),
            Utc::now(),
            1,
        )
    }

    fn input<'a>(brief: &'a str, own: &'a [RequiredDecision]) -> FindingsInput<'a> {
        FindingsInput {
            brief,
            discovery_complete: false,
            own_decisions: own,
            ancestor_has_decision: false,
            stories: 3,
        }
    }

    #[test]
    fn a_blank_brief_is_found_and_a_written_one_is_not() {
        assert_eq!(compute_findings(&input("  \n", &[])), [Finding::EmptyBrief]);
        assert_eq!(compute_findings(&input("Why we do this.", &[])), []);
    }

    #[test]
    fn a_brief_citing_jira_confluence_or_decided_wants_a_required_decision() {
        let brief = "Migrated from Jira JPD-30; see https://acme.atlassian.net/wiki/x. We decided on NorthCloud.";
        assert_eq!(
            compute_findings(&input(brief, &[])),
            [Finding::BriefCitesWithoutDecision {
                cites: vec!["Jira".into(), "Confluence".into(), "decided".into()]
            }]
        );
        // any required decision on the item, even a closed one, or above it answers it
        let own = [decision(1, CheckResult::Unchecked, true)];
        assert_eq!(compute_findings(&input(brief, &own)), []);
        let above = FindingsInput {
            ancestor_has_decision: true,
            ..input(brief, &[])
        };
        assert_eq!(compute_findings(&above), []);
        // a word that merely contains the cue is not a citation
        assert_eq!(
            compute_findings(&input("Undecided; see the jiralike tool.", &[])),
            []
        );
    }

    #[test]
    fn complete_discovery_with_an_open_unsatisfied_decision_or_no_stories_is_found() {
        let own = [
            decision(1, CheckResult::Unchecked, false),
            decision(
                2,
                CheckResult::Accepted {
                    commit: "abc".into(),
                    reversibility: None,
                },
                false,
            ),
            decision(3, CheckResult::Unchecked, true),
        ];
        let done = FindingsInput {
            discovery_complete: true,
            stories: 0,
            ..input("Why.", &own)
        };
        assert_eq!(
            compute_findings(&done),
            [
                Finding::DiscoveryCompleteWithOpenDecision {
                    decision_id: 1,
                    title: "Pick 1".into()
                },
                Finding::DiscoveryCompleteWithoutStories,
            ]
        );
        // while discovery is open neither applies
        let open = FindingsInput {
            discovery_complete: false,
            ..done
        };
        assert_eq!(compute_findings(&open), []);
    }
}
