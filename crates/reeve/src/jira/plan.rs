//! The import plan: what a run would create, in dependency order, and why anything is left out. Pure; a rehearsal
//! prints it, `--apply` creates it. Epics, stories and comments are planned with their parent's Jira key;
//! their wire requests exist only through `request(<the parent's created key>)`, so a request can never be sent with
//! a placeholder parent.

use super::adf::{split_criteria, to_markdown};
use super::client::{Comment, Issue, User};
use super::mapping::Mapping;
use chrono::NaiveDate;
use chrono::{DateTime, Utc};
use reeve_protocol::{
    CriterionDto, DeliveryState, ImportComment, ImportEpic, ImportInitiative, ImportStory, Key,
    Kind, Resolution,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

pub type JiraKey = String;

pub const MAX_TITLE_CHARS: usize = 200;
pub const LEGACY_LABEL: &str = "legacy-import";
/// Epics are recognised by this issue type name; a site that renamed or localised it must be told (docs).
const EPIC_TYPE: &str = "Epic";

#[derive(Debug, Default)]
pub struct ImportPlan {
    pub initiatives: Vec<(JiraKey, ImportInitiative)>,
    pub epics: Vec<PlannedEpic>,
    pub stories: Vec<PlannedStory>,
    pub comments: Vec<PlannedComment>,
    pub warnings: Vec<Warning>,
    /// Every issue-link type name on ideas and epics, with counts (for the rehearsal).
    pub link_types_seen: BTreeMap<String, usize>,
    pub grandfathered_stories: Vec<JiraKey>,
    pub marked_epics: Vec<JiraKey>,
    pub marked_initiatives: Vec<JiraKey>,
}

/// An epic as planned. Its request is built at apply time, once its initiative exists.
#[derive(Debug, Clone)]
pub struct PlannedEpic {
    pub key: JiraKey,
    /// The parent idea; `None` for both kinds of orphan.
    pub idea: Option<JiraKey>,
    pub title: String,
    pub brief: Option<String>,
    pub discovery_complete: bool,
}

impl PlannedEpic {
    /// `initiative` is the created key of [`Self::idea`].
    pub fn request(&self, initiative: Option<Key>) -> ImportEpic {
        ImportEpic {
            external_ref: self.key.clone(),
            title: self.title.clone(),
            initiative,
            brief: self.brief.clone(),
            discovery_complete: self.discovery_complete,
        }
    }
}

/// A story, bug or chore as planned. Its request is built at apply time, once its epic exists.
#[derive(Debug, Clone)]
pub struct PlannedStory {
    pub key: JiraKey,
    /// The parent epic, when it is in the plan.
    pub epic: Option<JiraKey>,
    pub title: String,
    pub kind: Kind,
    pub description: Option<String>,
    pub criteria: Vec<CriterionDto>,
    pub owner: Option<String>,
    pub labels: Vec<String>,
    pub delivery: DeliveryState,
    pub resolution: Option<Resolution>,
}

impl PlannedStory {
    /// `epic` is the created key of [`Self::epic`].
    pub fn request(&self, epic: Option<Key>) -> ImportStory {
        ImportStory {
            external_ref: self.key.clone(),
            title: self.title.clone(),
            kind: self.kind,
            epic,
            description: self.description.clone(),
            criteria: self.criteria.clone(),
            owner: self.owner.clone(),
            labels: self.labels.clone(),
            delivery: self.delivery,
            resolution: self.resolution,
        }
    }
}

/// A comment as planned. Its request is built at apply time, once its story exists.
#[derive(Debug, Clone)]
pub struct PlannedComment {
    /// The Jira key of the story the comment belongs to.
    pub story: JiraKey,
    pub external_ref: String,
    pub author: String,
    pub body: String,
    pub at: DateTime<Utc>,
}

impl PlannedComment {
    /// `story` is the created key of [`Self::story`].
    pub fn request(&self, story: Key) -> ImportComment {
        ImportComment {
            external_ref: self.external_ref.clone(),
            story,
            author: self.author.clone(),
            body: self.body.clone(),
            at: self.at,
        }
    }
}

/// What a dry run prints as totals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub initiatives: usize,
    pub epics: usize,
    pub stories: usize,
    pub comments: usize,
    pub warnings: usize,
}

impl ImportPlan {
    pub fn counts(&self) -> Counts {
        Counts {
            initiatives: self.initiatives.len(),
            epics: self.epics.len(),
            stories: self.stories.len(),
            comments: self.comments.len(),
            warnings: self.warnings.len(),
        }
    }
}

/// Display gives the report line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    IdeaSkipped {
        idea: JiraKey,
        why: SkipWhy,
    },
    /// The epic has no link of `idea_link_type`.
    OrphanNoIdeaLink {
        epic: JiraKey,
    },
    /// Linked, but the idea is not in the plan.
    OrphanIdeaSkipped {
        epic: JiraKey,
        idea: JiraKey,
        why: SkipWhy,
    },
    /// Unmapped status or type.
    IssueSkipped {
        issue: JiraKey,
        why: SkipWhy,
    },
    /// A comment on a skipped issue; one per comment, so the report counts them. Only comments on stories, bugs
    /// and tasks are fetched (see [`comment_sources`]), so comments on ideas and epics are never seen.
    CommentSkipped {
        issue: JiraKey,
        comment_id: String,
    },
    /// The story's `parent` is not an epic in the plan (another project, say); it is imported without an epic.
    ParentNotInPlan {
        story: JiraKey,
        parent: JiraKey,
    },
    /// Done with a null or unmapped Jira resolution.
    DefaultResolution {
        issue: JiraKey,
        jira: Option<String>,
    },
    /// The email is hidden and the account is not in `owner_map`.
    UnresolvedOwner {
        issue: JiraKey,
        account_id: String,
    },
    TitleTruncated {
        issue: JiraKey,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipWhy {
    UnmappedBucket(String),
    UnreadableField { field: String, value: String },
    UnmappedStatus(String),
    UnmappedType(String),
}

impl fmt::Display for SkipWhy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkipWhy::UnmappedBucket(b) => write!(f, "unmapped bucket {b:?}"),
            SkipWhy::UnreadableField { field, value } => {
                write!(f, "unreadable field {field}: {value}")
            }
            SkipWhy::UnmappedStatus(s) => write!(f, "unmapped status {s:?}"),
            SkipWhy::UnmappedType(t) => write!(f, "unmapped type {t:?}"),
        }
    }
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Warning::IdeaSkipped { idea, why } => write!(f, "skipped: idea {idea} ({why})"),
            Warning::OrphanNoIdeaLink { epic } => write!(f, "{epic}: orphan: no idea link"),
            Warning::OrphanIdeaSkipped { epic, idea, why } => {
                write!(f, "{epic}: orphan: idea {idea} skipped ({why})")
            }
            Warning::IssueSkipped { issue, why } => write!(f, "skipped: issue {issue} ({why})"),
            Warning::CommentSkipped { issue, comment_id } => write!(
                f,
                "jira-comment:{comment_id}: skipped: story {issue} skipped"
            ),
            Warning::ParentNotInPlan { story, parent } => write!(
                f,
                "{story}: parent {parent} is not an epic in the plan; imported without an epic"
            ),
            Warning::DefaultResolution { issue, jira: None } => {
                write!(
                    f,
                    "{issue}: done without a Jira resolution; the default applies"
                )
            }
            Warning::DefaultResolution {
                issue,
                jira: Some(r),
            } => write!(
                f,
                "{issue}: done with unmapped Jira resolution {r:?}; the default applies"
            ),
            Warning::UnresolvedOwner { issue, account_id } => write!(
                f,
                "{issue}: owner left empty: Jira hides the email of {account_id} and owner_map does not name it"
            ),
            Warning::TitleTruncated { issue } => {
                write!(
                    f,
                    "{issue}: title truncated to {MAX_TITLE_CHARS} characters"
                )
            }
        }
    }
}

/// The issues whose comments are fetched: every delivery issue but the epics (a skipped story's comments are fetched
/// only to be counted as skipped). Ideas and epics never carry imported comments, so they are not read.
pub fn comment_sources(issues: &[Issue]) -> impl Iterator<Item = &Issue> {
    issues
        .iter()
        .filter(|i| i.fields.type_name() != Some(EPIC_TYPE))
}

pub fn build(
    ideas: &[Issue],
    issues: &[Issue],
    comments: &HashMap<String, Vec<Comment>>,
    m: &Mapping,
) -> ImportPlan {
    let mut plan = ImportPlan::default();

    // Ideas → initiatives; a skipped idea is remembered so a linked epic can say why it is an orphan.
    let mut skipped_ideas: HashMap<&str, SkipWhy> = HashMap::new();
    for idea in ideas {
        count_links(&mut plan.link_types_seen, idea);
        match initiative(idea, m, &mut plan.warnings) {
            Ok(i) => plan.initiatives.push((idea.key.clone(), i)),
            Err(why) => {
                skipped_ideas.insert(idea.key.as_str(), why.clone());
                plan.warnings.push(Warning::IdeaSkipped {
                    idea: idea.key.clone(),
                    why,
                });
            }
        }
    }
    let planned_ideas: HashSet<&str> = plan.initiatives.iter().map(|(k, _)| k.as_str()).collect();

    // Epics are never skipped.
    let (epics, others): (Vec<&Issue>, Vec<&Issue>) = issues
        .iter()
        .partition(|i| i.fields.type_name() == Some(EPIC_TYPE));
    for epic in &epics {
        count_links(&mut plan.link_types_seen, epic);
        let linked: Vec<&str> = if m.idea_link_type.trim().is_empty() {
            vec![]
        } else {
            epic.fields
                .issuelinks
                .iter()
                .filter(|l| l.link_type.name.trim() == m.idea_link_type.trim())
                .filter_map(|l| l.other())
                .collect()
        };
        let parent = linked.iter().find(|k| planned_ideas.contains(**k));
        if parent.is_none() {
            let skipped = linked
                .iter()
                .find_map(|k| skipped_ideas.get(k).map(|why| (k, why)));
            plan.warnings.push(match skipped {
                Some((idea, why)) => Warning::OrphanIdeaSkipped {
                    epic: epic.key.clone(),
                    idea: idea.to_string(),
                    why: why.clone(),
                },
                None => Warning::OrphanNoIdeaLink {
                    epic: epic.key.clone(),
                },
            });
        }
        plan.epics.push(PlannedEpic {
            key: epic.key.clone(),
            idea: parent.map(|k| k.to_string()),
            title: title(epic, &mut plan.warnings),
            brief: non_empty(description(epic, m)),
            discovery_complete: false,
        });
    }
    let epic_keys: HashSet<&str> = epics.iter().map(|e| e.key.as_str()).collect();

    // Stories, bugs and tasks.
    let mut skipped_issues: HashSet<&str> = HashSet::new();
    for issue in &others {
        let f = &issue.fields;
        let type_name = f.type_name().unwrap_or("");
        let status = f.status_name().unwrap_or("");
        let (kind, delivery) = match (m.kind(type_name), m.delivery(status)) {
            (Some(k), Some(d)) => (k, d),
            (None, _) => {
                skip_issue(
                    &mut plan,
                    &mut skipped_issues,
                    issue,
                    SkipWhy::UnmappedType(type_name.into()),
                );
                continue;
            }
            (_, None) => {
                skip_issue(
                    &mut plan,
                    &mut skipped_issues,
                    issue,
                    SkipWhy::UnmappedStatus(status.into()),
                );
                continue;
            }
        };
        let resolution = (delivery == DeliveryState::Done).then(|| {
            let jira = f.resolution.as_ref().map(|r| r.name.clone());
            jira.as_deref()
                .and_then(|r| m.resolution(r))
                .unwrap_or_else(|| {
                    plan.warnings.push(Warning::DefaultResolution {
                        issue: issue.key.clone(),
                        jira: jira.clone(),
                    });
                    m.default_resolution()
                })
        });
        let grandfathered = matches!(
            delivery,
            DeliveryState::InProgress | DeliveryState::InReview | DeliveryState::Testing
        );
        let mut labels = f.labels.clone();
        if grandfathered && !labels.iter().any(|l| l == LEGACY_LABEL) {
            labels.push(LEGACY_LABEL.into());
        }
        let (body, criteria) = split_criteria(
            f.description.as_ref().unwrap_or(&Value::Null),
            &m.acceptance_heading,
            &m.browse_url(&issue.key),
        );
        let parent = f.parent.as_ref().map(|p| p.key.as_str());
        let epic = parent.filter(|k| epic_keys.contains(k)).map(String::from);
        if let Some(p) = parent
            && epic.is_none()
        {
            plan.warnings.push(Warning::ParentNotInPlan {
                story: issue.key.clone(),
                parent: p.to_string(),
            });
        }
        let story = PlannedStory {
            key: issue.key.clone(),
            epic: epic.clone(),
            title: title(issue, &mut plan.warnings),
            kind,
            description: non_empty(body),
            criteria,
            owner: owner(issue, m, &mut plan.warnings),
            labels,
            delivery,
            resolution,
        };
        if grandfathered {
            plan.grandfathered_stories.push(issue.key.clone());
            if let Some(e) = &epic
                && !plan.marked_epics.contains(e)
            {
                plan.marked_epics.push(e.clone());
            }
        }
        plan.stories.push(story);
    }

    // A grandfathered story's epic, and that epic's initiative, are marked discovery complete.
    for e in &mut plan.epics {
        if plan.marked_epics.contains(&e.key) {
            e.discovery_complete = true;
            if let Some(i) = &e.idea
                && !plan.marked_initiatives.contains(i)
            {
                plan.marked_initiatives.push(i.clone());
            }
        }
    }
    for (key, i) in &mut plan.initiatives {
        i.discovery_complete = plan.marked_initiatives.contains(key);
    }

    // Comments, in story order; those on a skipped issue are reported one by one.
    for issue in &others {
        let Some(cs) = comments.get(&issue.key) else {
            continue;
        };
        if skipped_issues.contains(issue.key.as_str()) {
            for c in cs {
                plan.warnings.push(Warning::CommentSkipped {
                    issue: issue.key.clone(),
                    comment_id: c.id.clone(),
                });
            }
            continue;
        }
        let url = m.browse_url(&issue.key);
        for c in cs {
            let body = to_markdown(c.body.as_ref().unwrap_or(&Value::Null), &url);
            let body = if body.trim().is_empty() {
                "_(empty comment)_".to_string()
            } else {
                body
            };
            plan.comments.push(PlannedComment {
                story: issue.key.clone(),
                external_ref: format!("jira-comment:{}", c.id),
                author: author(c.author.as_ref()),
                body,
                at: c.created,
            });
        }
    }
    plan
}

fn skip_issue<'a>(
    plan: &mut ImportPlan,
    skipped: &mut HashSet<&'a str>,
    issue: &'a Issue,
    why: SkipWhy,
) {
    skipped.insert(issue.key.as_str());
    plan.warnings.push(Warning::IssueSkipped {
        issue: issue.key.clone(),
        why,
    });
}

fn initiative(
    idea: &Issue,
    m: &Mapping,
    warnings: &mut Vec<Warning>,
) -> Result<ImportInitiative, SkipWhy> {
    let f = &idea.fields;
    let horizon = match f.custom(&m.bucket_field) {
        None => {
            return Err(SkipWhy::UnreadableField {
                field: m.bucket_field.clone(),
                value: "null".into(),
            });
        }
        Some(v) => match v.get("value").and_then(Value::as_str) {
            None => {
                return Err(SkipWhy::UnreadableField {
                    field: m.bucket_field.clone(),
                    value: v.to_string(),
                });
            }
            Some(b) => m
                .horizon(b)
                .ok_or_else(|| SkipWhy::UnmappedBucket(b.into()))?
                .to_string(),
        },
    };
    let target_month = match f.custom(&m.target_date_field) {
        None => None,
        Some(v) => Some(target_month(v).ok_or_else(|| SkipWhy::UnreadableField {
            field: m.target_date_field.clone(),
            value: v.to_string(),
        })?),
    };
    // Warnings are pushed only once the idea is known to be planned.
    let mut local = Vec::new();
    let i = ImportInitiative {
        external_ref: idea.key.clone(),
        title: title(idea, &mut local),
        brief: non_empty(description(idea, m)),
        horizon,
        target_month,
        owner: owner(idea, m, &mut local),
        discovery_complete: false,
    };
    warnings.extend(local);
    Ok(i)
}

/// `"YYYY-MM-DD"` (or `"YYYY-MM"`), an object `{start, end}`, or a string holding that object as JSON; the month
/// comes from `end`, else `start`.
fn target_month(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => {
            let s = s.trim();
            if s.starts_with('{') {
                target_month(&serde_json::from_str(s).ok()?)
            } else {
                month(s)
            }
        }
        Value::Object(o) => {
            let pick = |k: &str| o.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
            month(pick("end").or_else(|| pick("start"))?)
        }
        _ => None,
    }
}

fn month(s: &str) -> Option<String> {
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(&format!("{s}-01"), "%Y-%m-%d"))
        .ok()?;
    Some(d.format("%Y-%m").to_string())
}

/// Initiative and epic briefs keep their full text: no criteria are lifted from them.
fn description(issue: &Issue, m: &Mapping) -> String {
    to_markdown(
        issue.fields.description.as_ref().unwrap_or(&Value::Null),
        &m.browse_url(&issue.key),
    )
}

fn title(issue: &Issue, warnings: &mut Vec<Warning>) -> String {
    let t = issue.fields.summary.trim();
    if t.chars().count() > MAX_TITLE_CHARS {
        warnings.push(Warning::TitleTruncated {
            issue: issue.key.clone(),
        });
        t.chars().take(MAX_TITLE_CHARS).collect()
    } else {
        t.to_string()
    }
}

/// The assignee's email, else `owner_map[accountId]`, else none (with a warning).
fn owner(issue: &Issue, m: &Mapping, warnings: &mut Vec<Warning>) -> Option<String> {
    let a = issue.fields.assignee.as_ref()?;
    if let Some(e) = a.email_address.as_deref().filter(|e| !e.trim().is_empty()) {
        return Some(e.to_string());
    }
    if let Some(e) = m.owner_map.get(&a.account_id) {
        return Some(e.clone());
    }
    warnings.push(Warning::UnresolvedOwner {
        issue: issue.key.clone(),
        account_id: a.account_id.clone(),
    });
    None
}

/// `"<displayName> (<email or accountId>)"`.
fn author(u: Option<&User>) -> String {
    let Some(u) = u else {
        return "Unknown (Jira)".into();
    };
    let id = u
        .email_address
        .as_deref()
        .filter(|e| !e.trim().is_empty())
        .unwrap_or(&u.account_id);
    let name = u
        .display_name
        .as_deref()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or(&u.account_id);
    format!("{name} ({id})")
}

fn non_empty(s: String) -> Option<String> {
    (!s.trim().is_empty()).then_some(s)
}

fn count_links(seen: &mut BTreeMap<String, usize>, issue: &Issue) {
    for l in &issue.fields.issuelinks {
        *seen.entry(l.link_type.name.clone()).or_default() += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::super::client::tests::fixture;
    use super::*;

    const LINK: &str = "Polaris work item link";

    fn mapping() -> Mapping {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../jira-import.example.toml"
        ))
        .unwrap();
        let mut m = Mapping::parse(&text).unwrap();
        m.idea_link_type = LINK.into();
        m.owner_map
            .insert("acc-carol".into(), "carol@example.com".into());
        m
    }

    fn pages(a: &str, b: &str) -> Vec<Issue> {
        [a, b]
            .iter()
            .flat_map(|p| {
                serde_json::from_value::<Vec<Issue>>(fixture(p)["issues"].clone()).unwrap()
            })
            .collect()
    }

    fn ideas() -> Vec<Issue> {
        pages("ideas_page1.json", "ideas_page2.json")
    }
    fn issues() -> Vec<Issue> {
        pages("issues_page1.json", "issues_page2.json")
    }
    fn comments() -> HashMap<String, Vec<Comment>> {
        ["SCRUM-11", "SCRUM-15"]
            .iter()
            .map(|k| {
                let page = fixture(&format!("comments_{k}_page1.json"));
                (
                    k.to_string(),
                    serde_json::from_value(page["comments"].clone()).unwrap(),
                )
            })
            .collect()
    }
    fn plan() -> ImportPlan {
        build(&ideas(), &issues(), &comments(), &mapping())
    }

    fn story<'a>(p: &'a ImportPlan, key: &str) -> Option<&'a PlannedStory> {
        p.stories.iter().find(|s| s.key == key)
    }
    fn epic<'a>(p: &'a ImportPlan, key: &str) -> &'a PlannedEpic {
        p.epics.iter().find(|e| e.key == key).unwrap()
    }
    fn initiative<'a>(p: &'a ImportPlan, key: &str) -> &'a ImportInitiative {
        &p.initiatives.iter().find(|(k, _)| k == key).unwrap().1
    }
    fn comment(id: &str, body: Value, author: Value) -> Comment {
        serde_json::from_value(serde_json::json!({
            "id": id, "author": author, "body": body, "created": "2026-09-03T08:00:00.000+0000"
        }))
        .unwrap()
    }

    /// Catches a plan that loses or invents items.
    #[test]
    fn counts() {
        let p = plan();
        let c = p.counts();
        assert_eq!(
            (c.initiatives, c.epics, c.stories, c.comments),
            (2, 3, 6, 1)
        );
        let keys: Vec<_> = p.stories.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "SCRUM-11", "SCRUM-12", "SCRUM-13", "SCRUM-14", "SCRUM-16", "SCRUM-17"
            ]
        );
    }

    /// Catches grandfathering that misses the in-flight story's ancestors, or marks the wrong ones.
    #[test]
    fn grandfathering_sets() {
        let p = plan();
        assert_eq!(p.grandfathered_stories, ["SCRUM-11"]);
        assert_eq!(p.marked_epics, ["SCRUM-10"]);
        assert_eq!(p.marked_initiatives, ["JPD-1"]);
        assert!(epic(&p, "SCRUM-10").discovery_complete);
        assert!(!epic(&p, "SCRUM-20").discovery_complete);
        assert!(initiative(&p, "JPD-1").discovery_complete);
        assert!(!initiative(&p, "JPD-2").discovery_complete);
        let s11 = story(&p, "SCRUM-11").unwrap();
        assert_eq!(s11.delivery, DeliveryState::InProgress);
        assert_eq!(s11.labels, ["frontend", LEGACY_LABEL]);
        assert_eq!(s11.owner.as_deref(), Some("dana@example.com"));
        assert!(
            !story(&p, "SCRUM-14")
                .unwrap()
                .labels
                .contains(&LEGACY_LABEL.to_string())
        );
    }

    /// Catches a resolution map that is not consulted.
    #[test]
    fn wont_do_is_done_and_dropped() {
        let p = plan();
        let s = story(&p, "SCRUM-12").unwrap();
        assert_eq!(s.delivery, DeliveryState::Done);
        assert_eq!(s.resolution, Some(Resolution::Dropped));
        assert!(
            s.owner.is_none(),
            "a null assignee is no owner and no warning"
        );
        assert!(
            !p.warnings.iter().any(
                |w| matches!(w, Warning::UnresolvedOwner { issue, .. } if issue == "SCRUM-12")
            )
        );
        assert!(
            !p.warnings.iter().any(
                |w| matches!(w, Warning::DefaultResolution { issue, .. } if issue == "SCRUM-12")
            )
        );
    }

    /// Catches a plan that conflates the two orphan kinds, and one that drops an epic whose idea was skipped.
    #[test]
    fn orphan_kinds() {
        let p = plan();
        assert_eq!(epic(&p, "SCRUM-10").idea.as_deref(), Some("JPD-1"));
        assert!(epic(&p, "SCRUM-20").idea.is_none());
        assert!(p.warnings.contains(&Warning::OrphanIdeaSkipped {
            epic: "SCRUM-20".into(),
            idea: "JPD-3".into(),
            why: SkipWhy::UnmappedBucket("Parking lot".into()),
        }));
        assert!(
            epic(&p, "SCRUM-30").idea.is_none(),
            "a Relates link to an idea is not the idea link"
        );
        assert!(p.warnings.contains(&Warning::OrphanNoIdeaLink {
            epic: "SCRUM-30".into()
        }));
        assert!(
            !p.warnings
                .iter()
                .any(|w| matches!(w, Warning::OrphanNoIdeaLink { epic } if epic == "SCRUM-20"))
        );
        assert!(p.warnings.contains(&Warning::IdeaSkipped {
            idea: "JPD-3".into(),
            why: SkipWhy::UnmappedBucket("Parking lot".into()),
        }));
        let lines: Vec<String> = p.warnings.iter().map(|w| w.to_string()).collect();
        assert!(lines.contains(&"SCRUM-30: orphan: no idea link".to_string()));
        assert!(lines.contains(
            &"SCRUM-20: orphan: idea JPD-3 skipped (unmapped bucket \"Parking lot\")".to_string()
        ));
        assert!(
            lines.contains(&"skipped: idea JPD-3 (unmapped bucket \"Parking lot\")".to_string())
        );
    }

    /// The request carries exactly the parent key the apply step passes in: no placeholder can reach the server.
    #[test]
    fn requests_take_the_created_parent_keys() {
        let p = plan();
        let created: Key = "ABC-7".parse().unwrap();
        let e = epic(&p, "SCRUM-10").request(Some(created.clone()));
        assert_eq!(e.initiative, Some(created.clone()));
        assert_eq!(e.external_ref, "SCRUM-10");
        assert!(e.discovery_complete);
        assert!(epic(&p, "SCRUM-30").request(None).initiative.is_none());
        let s = story(&p, "SCRUM-11")
            .unwrap()
            .request(Some(created.clone()));
        assert_eq!(s.epic, Some(created.clone()));
        assert_eq!(s.external_ref, "SCRUM-11");
        assert_eq!(s.labels, ["frontend", LEGACY_LABEL]);
        let c = p.comments[0].request(created.clone());
        assert_eq!(c.story, created);
        assert_eq!(c.external_ref, "jira-comment:900");
    }

    /// Catches a plan that skips the stories of an orphan epic.
    #[test]
    fn a_story_of_an_orphan_epic_is_planned_under_it() {
        let p = plan();
        assert_eq!(
            story(&p, "SCRUM-17").unwrap().epic.as_deref(),
            Some("SCRUM-20")
        );
        assert_eq!(
            story(&p, "SCRUM-13").unwrap().epic.as_deref(),
            Some("SCRUM-30")
        );
        assert!(
            !p.warnings
                .iter()
                .any(|w| matches!(w, Warning::ParentNotInPlan { .. }))
        );
    }

    /// Catches a story whose parent is outside the plan losing its epic without a word.
    #[test]
    fn a_parent_outside_the_plan_is_reported_and_the_story_kept() {
        let mut all = issues();
        let s = all.iter_mut().find(|i| i.key == "SCRUM-14").unwrap();
        s.fields.parent = Some(super::super::client::KeyOnly {
            key: "OTHER-1".into(),
        });
        let p = build(&ideas(), &all, &comments(), &mapping());
        assert!(story(&p, "SCRUM-14").unwrap().epic.is_none());
        let w = Warning::ParentNotInPlan {
            story: "SCRUM-14".into(),
            parent: "OTHER-1".into(),
        };
        assert!(p.warnings.contains(&w));
        assert_eq!(
            w.to_string(),
            "SCRUM-14: parent OTHER-1 is not an epic in the plan; imported without an epic"
        );
    }

    /// Catches comments planned for a story that will not exist.
    #[test]
    fn a_skipped_issue_drops_its_comments() {
        let p = plan();
        assert!(story(&p, "SCRUM-15").is_none());
        assert!(p.warnings.contains(&Warning::IssueSkipped {
            issue: "SCRUM-15".into(),
            why: SkipWhy::UnmappedStatus("Blocked".into()),
        }));
        assert!(p.warnings.contains(&Warning::CommentSkipped {
            issue: "SCRUM-15".into(),
            comment_id: "901".into(),
        }));
        assert!(
            !p.comments
                .iter()
                .any(|c| c.external_ref == "jira-comment:901")
        );
        let lines: Vec<String> = p.warnings.iter().map(|w| w.to_string()).collect();
        assert!(lines.contains(&"jira-comment:901: skipped: story SCRUM-15 skipped".to_string()));
    }

    /// Catches a fetch that reads comments of ideas and epics, which are never imported.
    #[test]
    fn comments_are_read_only_from_non_epic_issues() {
        let issues = issues();
        let keys: Vec<&str> = comment_sources(&issues).map(|i| i.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "SCRUM-11", "SCRUM-12", "SCRUM-13", "SCRUM-14", "SCRUM-15", "SCRUM-16", "SCRUM-17"
            ]
        );
    }

    #[test]
    fn a_comment_keeps_its_story_author_date_and_body() {
        let p = plan();
        let c = &p.comments[0];
        let story: &JiraKey = &c.story;
        assert_eq!(story, "SCRUM-11");
        assert_eq!(c.external_ref, "jira-comment:900");
        assert_eq!(c.author, "Dana Example (dana@example.com)");
        assert_eq!(c.body, "Started on the **form**.");
        assert_eq!(c.at.to_rfc3339(), "2026-09-01T10:15:30.123+00:00");
    }

    /// Catches a comment the server would refuse as blank, or one with no author.
    #[test]
    fn an_empty_body_and_a_null_author_still_make_a_comment() {
        let mut cs = comments();
        cs.insert(
            "SCRUM-12".into(),
            vec![comment(
                "920",
                serde_json::json!({"type": "doc", "content": []}),
                Value::Null,
            )],
        );
        let p = build(&ideas(), &issues(), &cs, &mapping());
        let c = p
            .comments
            .iter()
            .find(|c| c.external_ref == "jira-comment:920")
            .unwrap();
        assert_eq!(c.story, "SCRUM-12");
        assert_eq!(c.body, "_(empty comment)_");
        assert_eq!(c.author, "Unknown (Jira)");
    }

    /// Catches a missing `owner_map` lookup.
    #[test]
    fn a_done_issue_without_resolution_gets_the_default_and_its_mapped_owner() {
        let p = plan();
        let s = story(&p, "SCRUM-16").unwrap();
        assert_eq!(s.kind, Kind::Chore);
        assert_eq!(s.delivery, DeliveryState::Done);
        assert_eq!(s.resolution, Some(Resolution::Completed));
        assert_eq!(s.owner.as_deref(), Some("carol@example.com"));
        assert!(p.warnings.contains(&Warning::DefaultResolution {
            issue: "SCRUM-16".into(),
            jira: None,
        }));
    }

    #[test]
    fn an_unresolvable_owner_is_left_empty_with_a_warning() {
        let p = plan();
        assert!(story(&p, "SCRUM-17").unwrap().owner.is_none());
        assert!(p.warnings.contains(&Warning::UnresolvedOwner {
            issue: "SCRUM-17".into(),
            account_id: "acc-dave".into(),
        }));
    }

    #[test]
    fn a_long_title_is_truncated() {
        let p = plan();
        let s = story(&p, "SCRUM-13").unwrap();
        assert_eq!(s.kind, Kind::Bug);
        assert_eq!(s.title.chars().count(), MAX_TITLE_CHARS);
        assert!(p.warnings.contains(&Warning::TitleTruncated {
            issue: "SCRUM-13".into()
        }));
    }

    /// Catches lifted items duplicated in the body, and a brief losing its acceptance section.
    #[test]
    fn criteria_are_lifted_for_stories_only() {
        let p = plan();
        let s14 = story(&p, "SCRUM-14").unwrap();
        assert_eq!(
            s14.criteria,
            [
                CriterionDto {
                    text: "A link is sent".into(),
                    done: false
                },
                CriterionDto {
                    text: "The link expires after a day".into(),
                    done: false
                },
                CriterionDto {
                    text: "Design reviewed".into(),
                    done: true
                },
                CriterionDto {
                    text: "Copy reviewed".into(),
                    done: false
                },
            ]
        );
        let d = s14.description.as_deref().unwrap();
        assert!(!d.contains("A link is sent") && !d.contains("Design reviewed"));
        assert!(d.contains("- Uses the mail service"));
        for s in &p.stories {
            if s.key != "SCRUM-14" {
                assert!(s.criteria.is_empty(), "{}", s.key);
            }
        }
        let e10 = epic(&p, "SCRUM-10").brief.as_deref().unwrap();
        assert!(e10.contains("## Acceptance criteria\n\n- Sign-up works\n- Welcome email sent"));
        let jpd1 = initiative(&p, "JPD-1").brief.as_deref().unwrap();
        assert!(jpd1.contains("- A customer can sign up alone"));
        assert!(jpd1.contains("- [ ] Pricing agreed"));
        assert!(epic(&p, "SCRUM-30").brief.is_none());
    }

    #[test]
    fn target_months_come_from_end_else_the_date() {
        let p = plan();
        assert_eq!(
            initiative(&p, "JPD-1").target_month.as_deref(),
            Some("2026-12")
        );
        assert_eq!(initiative(&p, "JPD-1").horizon, "Now");
        assert_eq!(
            initiative(&p, "JPD-2").target_month.as_deref(),
            Some("2027-01")
        );
        assert_eq!(
            target_month(&serde_json::json!({"start": "2026-03-04"})).as_deref(),
            Some("2026-03")
        );
        assert_eq!(
            target_month(&serde_json::json!("2026-05")).as_deref(),
            Some("2026-05")
        );
        assert_eq!(target_month(&serde_json::json!("soon")), None);
    }

    /// Catches a strict parse that aborts the run.
    #[test]
    fn an_unreadable_field_skips_the_idea_and_the_plan_is_still_built() {
        let mut all = ideas();
        let mut copy = all.iter().find(|i| i.key == "JPD-2").unwrap().clone();
        copy.key = "JPD-4".into();
        copy.fields.extra.insert(
            "customfield_10043".into(),
            serde_json::json!(["2027-01-20"]),
        );
        all.push(copy);
        let mut odd_bucket = all[0].clone();
        odd_bucket.key = "JPD-5".into();
        odd_bucket.fields.extra.insert(
            "customfield_10042".into(),
            serde_json::json!([{"value": "Now"}]),
        );
        all.push(odd_bucket);
        let p = build(&all, &issues(), &comments(), &mapping());
        assert_eq!(p.initiatives.len(), 2);
        assert!(p.warnings.iter().any(|w| matches!(w,
            Warning::IdeaSkipped { idea, why: SkipWhy::UnreadableField { field, .. } }
                if idea == "JPD-4" && field == "customfield_10043")));
        assert!(p.warnings.iter().any(|w| matches!(w,
            Warning::IdeaSkipped { idea, why: SkipWhy::UnreadableField { field, .. } }
                if idea == "JPD-5" && field == "customfield_10042")));
    }

    #[test]
    fn link_types_seen_are_counted() {
        let p = plan();
        let expected: BTreeMap<String, usize> =
            [(LINK.to_string(), 4), ("Relates".to_string(), 2)].into();
        assert_eq!(p.link_types_seen, expected);
    }

    /// An empty `idea_link_type` (a rehearsal) orphans every epic as "no idea link" and still lists the link types.
    #[test]
    fn without_a_link_type_every_epic_is_an_orphan() {
        let mut m = mapping();
        m.idea_link_type = String::new();
        let p = build(&ideas(), &issues(), &comments(), &m);
        assert_eq!(p.epics.len(), 3);
        assert!(p.epics.iter().all(|e| e.idea.is_none()));
        let orphans = p
            .warnings
            .iter()
            .filter(|w| matches!(w, Warning::OrphanNoIdeaLink { .. }))
            .count();
        assert_eq!(orphans, 3);
        assert_eq!(p.link_types_seen.get(LINK), Some(&4));
        assert!(p.marked_initiatives.is_empty());
    }
}
