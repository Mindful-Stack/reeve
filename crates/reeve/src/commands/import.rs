//! `reeve import jira`: read Jira Cloud, build the import plan, print it (the rehearsal, the default) and, with
//! `--apply`, create it through `POST /api/v1/import/*`.
//!
//! Nothing is written unless every page was fetched: a fetch failure ends the run before the plan is built. The
//! creates run in dependency order (initiatives, epics, stories, comments); every request is built from the created
//! key of its planned parent, and a descendant of a failed create is never sent. The import is create-only on the
//! server, so a second `--apply` reports every item as existing.
use super::finish;
use crate::client::{Client, Outcome};
use crate::jira::client::{Comment, FetchError, Issue, JiraClient, search_fields};
use crate::jira::mapping::{Mapping, Purpose};
use crate::jira::plan::{self, ImportPlan, JiraKey, Warning};
use clap::{Args, Subcommand};
use reeve_protocol::{
    ErrorCode, ImportComment, ImportEpic, ImportInitiative, ImportStory, Imported,
    InstanceConfigDto, Key, Me, Role, TokenKind,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum Import {
    /// Import Jira Cloud ideas, epics, stories and their comments. Rehearses (writes nothing) unless --apply.
    Jira(JiraArgs),
}

#[derive(Args)]
pub struct JiraArgs {
    /// The mapping file (see jira-import.example.toml).
    #[arg(long)]
    mapping: PathBuf,
    /// The Jira site, overriding the mapping's `site`.
    #[arg(long)]
    site: Option<String>,
    /// The environment variable holding the Jira API token.
    #[arg(long, default_value = "JIRA_TOKEN")]
    token_from_env: String,
    /// Create the plan on the server; without it the run is a rehearsal and writes nothing.
    #[arg(long)]
    apply: bool,
}

pub const SWITCHED_OFF: &str =
    "import is switched off on this instance: set [import] enabled = true and restart";

pub async fn run(c: &Client, cmd: Import, json: bool) -> anyhow::Result<i32> {
    match cmd {
        Import::Jira(a) => jira(c, a, json).await,
    }
}

async fn jira(c: &Client, a: JiraArgs, json: bool) -> anyhow::Result<i32> {
    // A mapping or token the user must fix is a `validation` error, never `internal`.
    let invalid = |message: String| fail(json, ErrorCode::Validation, &message, 1, None);
    let mapping = match Mapping::load(&a.mapping) {
        Ok(m) => m.with_site_override(a.site.as_deref()),
        Err(e) => return Ok(invalid(e.to_string())),
    };

    // Preflight: the server would refuse anyone but an interactive admin; say so before reading Jira.
    let me = match c.get::<Me>("/me").await? {
        Outcome::Ok(me) => me,
        other => return Ok(finish(other, json, |_| String::new())),
    };
    if me.role != Role::Admin || me.token_kind != TokenKind::Interactive {
        return Ok(fail(
            json,
            ErrorCode::Forbidden,
            &format!(
                "import needs an admin with an interactive token; {} is {} with an {} token",
                me.email,
                wire(&me.role),
                wire(&me.token_kind)
            ),
            2,
            None,
        ));
    }
    let config = match c.get::<InstanceConfigDto>("/config").await? {
        Outcome::Ok(cfg) => cfg,
        other => return Ok(finish(other, json, |_| String::new())),
    };
    if a.apply && !config.import_enabled {
        return Ok(invalid(SWITCHED_OFF.into()));
    }
    let purpose = if a.apply {
        Purpose::Apply
    } else {
        Purpose::Rehearse
    };
    let api = match mapping
        .validate(&config.horizon_buckets, purpose)
        .and_then(|()| mapping.api_url())
    {
        Ok(u) => u,
        Err(e) => return Ok(invalid(e.to_string())),
    };
    let Some(token) = crate::config::env_nonempty(&a.token_from_env) else {
        return Ok(invalid(format!(
            "no Jira API token: set {} (or name another variable with --token-from-env)",
            a.token_from_env
        )));
    };
    let jira = JiraClient::new(api, mapping.email.clone(), token);

    let (ideas, issues, comments) = match fetch(&jira, &mapping).await {
        Ok(f) => f,
        Err(e) => {
            return Ok(fail(
                json,
                ErrorCode::Internal,
                &format!("{e}; nothing written: a partial fetch is never imported"),
                1,
                Some(serde_json::json!({"page": e.page, "status": e.status, "written": false})),
            ));
        }
    };
    let plan = plan::build(&ideas, &issues, &comments, &mapping);
    let mut report = Report::of(&plan);
    let code = if a.apply {
        let applied = apply(&plan, c).await;
        let code = applied.exit_code();
        report.written = applied.created.total() > 0;
        report.apply = Some(applied);
        code
    } else {
        0
    };
    // One shape for every report, partial failures included: the exit code and `value.apply.failed` carry them.
    if json {
        println!("{}", serde_json::json!({"outcome": "ok", "value": report}));
    } else {
        println!("{}", crate::render::import_report(&report));
    }
    Ok(code)
}

/// A refusal or failure with no report: stderr always, and with `--json` the protocol's error envelope (a protocol
/// `ErrorCode` only), with `extra` fields merged in.
fn fail(
    json: bool,
    code: ErrorCode,
    message: &str,
    exit: i32,
    extra: Option<serde_json::Value>,
) -> i32 {
    if json {
        let mut v = serde_json::json!({"outcome":"error","code":code,"message":message});
        if let (Some(serde_json::Value::Object(extra)), Some(obj)) = (extra, v.as_object_mut()) {
            obj.extend(extra);
        }
        println!("{v}");
    }
    eprintln!("error: {message}");
    exit
}

/// A page that failed, named with what was being read (`issues of SCRUM: search page 2`).
#[derive(Debug)]
struct Fetch {
    page: String,
    status: Option<u16>,
    body: String,
}
impl std::fmt::Display for Fetch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "jira: {} failed", self.page)?;
        if let Some(s) = self.status {
            write!(f, " (HTTP {s})")?;
        }
        write!(f, ": {}", self.body)
    }
}
fn named(what: &str) -> impl Fn(FetchError) -> Fetch + '_ {
    move |e| Fetch {
        page: format!("{what}: {}", e.page),
        status: e.status,
        body: e.body,
    }
}

type Fetched = (Vec<Issue>, Vec<Issue>, HashMap<String, Vec<Comment>>);

/// Ideas, issues and the comments of every story, bug and task (never of ideas or epics, which import none); the
/// first page that fails ends the fetch.
async fn fetch(jira: &JiraClient, m: &Mapping) -> Result<Fetched, Fetch> {
    let fields = search_fields(&m.bucket_field, &m.target_date_field);
    let jql = |project: &str| format!("project = \"{project}\" ORDER BY key ASC");
    let ideas = jira
        .search(&jql(&m.discovery_project), &fields)
        .await
        .map_err(named(&format!("ideas of {}", m.discovery_project)))?;
    let issues = jira
        .search(&jql(&m.delivery_project), &fields)
        .await
        .map_err(named(&format!("issues of {}", m.delivery_project)))?;
    let mut comments = HashMap::new();
    for issue in plan::comment_sources(&issues) {
        let cs = jira.comments(&issue.key).await.map_err(named(&issue.key))?;
        if !cs.is_empty() {
            comments.insert(issue.key.clone(), cs);
        }
    }
    Ok((ideas, issues, comments))
}

/// The wire spelling of a protocol enum (`in_progress`, `admin`).
fn wire<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

/// What `apply` needs from the server; `Client` implements it by POSTing to /import/{initiatives,epics,stories,comments}.
pub(crate) trait ImportTarget {
    async fn initiative(&self, body: &ImportInitiative) -> anyhow::Result<Outcome<Imported>>;
    async fn epic(&self, body: &ImportEpic) -> anyhow::Result<Outcome<Imported>>;
    async fn story(&self, body: &ImportStory) -> anyhow::Result<Outcome<Imported>>;
    async fn comment(&self, body: &ImportComment) -> anyhow::Result<Outcome<Imported>>;
}

impl ImportTarget for Client {
    async fn initiative(&self, body: &ImportInitiative) -> anyhow::Result<Outcome<Imported>> {
        self.post("/import/initiatives", serde_json::to_value(body)?)
            .await
    }
    async fn epic(&self, body: &ImportEpic) -> anyhow::Result<Outcome<Imported>> {
        self.post("/import/epics", serde_json::to_value(body)?)
            .await
    }
    async fn story(&self, body: &ImportStory) -> anyhow::Result<Outcome<Imported>> {
        self.post("/import/stories", serde_json::to_value(body)?)
            .await
    }
    async fn comment(&self, body: &ImportComment) -> anyhow::Result<Outcome<Imported>> {
        self.post("/import/comments", serde_json::to_value(body)?)
            .await
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct Counts {
    pub initiatives: usize,
    pub epics: usize,
    pub stories: usize,
    pub comments: usize,
}
impl Counts {
    fn bump(&mut self, step: &str) {
        match step {
            "initiative" => self.initiatives += 1,
            "epic" => self.epics += 1,
            "story" => self.stories += 1,
            _ => self.comments += 1,
        }
    }
    pub fn total(&self) -> usize {
        self.initiatives + self.epics + self.stories + self.comments
    }
}

/// Why a create failed: the server's error (with its code), its refusal with reasons, or no answer at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Cause {
    Error,
    Refused,
    Transport,
}

#[derive(Debug, Serialize)]
pub(crate) struct Failure {
    pub jira_key: String,
    pub step: &'static str,
    pub cause: Cause,
    pub code: Option<ErrorCode>,
    pub message: String,
    pub exit: i32,
}

impl Failure {
    /// The code as the report prints it: the protocol's wire name (`not_found`), `refused` or `transport`.
    pub fn code_word(&self) -> String {
        match (self.cause, self.code) {
            (Cause::Error, Some(c)) => wire(&c),
            (Cause::Error, None) => "error".into(),
            (Cause::Refused, _) => "refused".into(),
            (Cause::Transport, _) => "transport".into(),
        }
    }
    /// The server refused the caller, or did not answer: every later create would fail the same way.
    fn ends_the_run(&self) -> bool {
        self.cause == Cause::Transport
            || matches!(
                self.code,
                Some(ErrorCode::Forbidden | ErrorCode::Unauthenticated)
            )
    }
}

#[derive(Debug, Default, Serialize)]
pub(crate) struct ApplyReport {
    pub created: Counts,
    pub existing: Counts,
    pub failed: Vec<Failure>,
    /// (Jira key, reason): never sent because an ancestor failed or the run stopped.
    pub skipped: Vec<(String, String)>,
    /// Set when a refusal of the caller or a transport error stopped the run: the failure that did.
    pub stopped: Option<String>,
}

impl ApplyReport {
    /// 0 with no failures; 2 when every failure is a refusal (`Outcome::exit_code() == 2`: forbidden or unauthenticated); else 1.
    pub fn exit_code(&self) -> i32 {
        if self.failed.is_empty() {
            0
        } else if self.failed.iter().all(|f| f.exit == 2) {
            2
        } else {
            1
        }
    }

    /// True (and `jira_key` recorded as skipped) once the run has stopped: nothing more is sent.
    fn halted(&mut self, jira_key: &str) -> bool {
        let Some(why) = &self.stopped else {
            return false;
        };
        let reason = format!("skipped: run stopped after {why}");
        self.skipped.push((jira_key.to_string(), reason));
        true
    }

    /// Counts a created or existing item and returns it, or records the failure and returns `None`. `item` is true for
    /// initiatives, epics and stories, whose answer must carry the key their children are created under.
    fn settle(
        &mut self,
        jira_key: &str,
        step: &'static str,
        out: anyhow::Result<Outcome<Imported>>,
        item: bool,
    ) -> Option<Imported> {
        let (cause, code, message, exit) = match out {
            Ok(Outcome::Ok(i)) if item && i.key.is_none() => (
                Cause::Error,
                Some(ErrorCode::Internal),
                "the server answered without a key".to_string(),
                1,
            ),
            Ok(Outcome::Ok(i)) => {
                if i.created {
                    self.created.bump(step);
                } else {
                    self.existing.bump(step);
                }
                return Some(i);
            }
            Ok(o @ Outcome::Refused { .. }) => {
                let exit = o.exit_code();
                let Outcome::Refused { reasons, .. } = o else {
                    unreachable!()
                };
                let why: Vec<String> = reasons.iter().map(crate::render::reason).collect();
                (Cause::Refused, None, why.join("; "), exit)
            }
            Ok(o @ Outcome::Error { .. }) => {
                let exit = o.exit_code();
                let Outcome::Error { code, message, .. } = o else {
                    unreachable!()
                };
                (Cause::Error, Some(code), message, exit)
            }
            Err(e) => (Cause::Transport, None, format!("{e:#}"), 1),
        };
        let f = Failure {
            jira_key: jira_key.to_string(),
            step,
            cause,
            code,
            message,
            exit,
        };
        if f.ends_the_run() {
            self.stopped = Some(format!(
                "{} failed ({}: {})",
                f.jira_key,
                f.code_word(),
                f.message
            ));
        }
        self.failed.push(f);
        None
    }
}

/// Where each planned item stands while `apply` runs: its created (or existing) Reeve key, or the Jira key of the
/// failed create that stopped it (itself, or the ancestor whose failure skipped it).
#[derive(Default)]
struct Progress<'a> {
    keys: HashMap<&'a str, Key>,
    failed: HashMap<&'a str, String>,
}
impl<'a> Progress<'a> {
    /// The created key of `parent`, or the failed create to name in the skip reason.
    fn parent(&self, parent: &str) -> Result<Key, String> {
        self.keys.get(parent).cloned().ok_or_else(|| {
            self.failed
                .get(parent)
                .cloned()
                .unwrap_or_else(|| parent.to_string())
        })
    }
    /// Records the child as skipped because of `root`, so its own descendants are skipped for the same reason.
    fn skip(&mut self, r: &mut ApplyReport, child: &'a str, root: String) {
        r.skipped
            .push((child.to_string(), format!("skipped: parent {root} failed")));
        self.failed.insert(child, root);
    }
    fn settled(&mut self, jira_key: &'a str, imported: Option<Imported>) {
        match imported.and_then(|i| i.key) {
            Some(k) => {
                self.keys.insert(jira_key, k);
            }
            None => {
                self.failed.insert(jira_key, jira_key.to_string());
            }
        }
    }
}

/// Creates the plan in dependency order. A failed create is recorded and its descendants are never sent; the rest
/// continues, unless the failure refused the caller (`forbidden`, `unauthenticated`) or got no answer: then nothing
/// more is sent and every remaining item is skipped as "skipped: run stopped after <failure>".
pub(crate) async fn apply(plan: &ImportPlan, target: &impl ImportTarget) -> ApplyReport {
    let mut r = ApplyReport::default();
    let mut p = Progress::default();
    for (key, body) in &plan.initiatives {
        if r.halted(key) {
            continue;
        }
        let out = target.initiative(body).await;
        let done = r.settle(key, "initiative", out, true);
        p.settled(key, done);
    }
    for e in &plan.epics {
        if r.halted(&e.key) {
            continue;
        }
        let initiative = match e.idea.as_deref().map(|i| p.parent(i)).transpose() {
            Ok(k) => k,
            Err(root) => {
                p.skip(&mut r, &e.key, root);
                continue;
            }
        };
        let out = target.epic(&e.request(initiative)).await;
        let done = r.settle(&e.key, "epic", out, true);
        p.settled(&e.key, done);
    }
    for s in &plan.stories {
        if r.halted(&s.key) {
            continue;
        }
        let epic = match s.epic.as_deref().map(|e| p.parent(e)).transpose() {
            Ok(k) => k,
            Err(root) => {
                p.skip(&mut r, &s.key, root);
                continue;
            }
        };
        let out = target.story(&s.request(epic)).await;
        let done = r.settle(&s.key, "story", out, true);
        p.settled(&s.key, done);
    }
    for c in &plan.comments {
        if r.halted(&c.external_ref) {
            continue;
        }
        let story = match p.parent(&c.story) {
            Ok(k) => k,
            Err(root) => {
                r.skipped.push((
                    c.external_ref.clone(),
                    format!("skipped: parent {root} failed"),
                ));
                continue;
            }
        };
        let out = target.comment(&c.request(story)).await;
        r.settle(&c.external_ref, "comment", out, false);
    }
    r
}

/// The report a rehearsal prints, and an apply extends with what it did.
#[derive(Debug, Serialize)]
pub(crate) struct Report {
    pub planned: Planned,
    pub warnings: Vec<WarningGroup>,
    pub link_types_seen: BTreeMap<String, usize>,
    pub grandfathered_stories: Vec<JiraKey>,
    pub marked_epics: Vec<JiraKey>,
    pub marked_initiatives: Vec<JiraKey>,
    /// False for a rehearsal: nothing written.
    pub written: bool,
    pub apply: Option<ApplyReport>,
}

#[derive(Debug, Serialize)]
pub(crate) struct Planned {
    pub initiatives: usize,
    pub epics: usize,
    pub stories: usize,
    pub comments: usize,
    pub warnings: usize,
    /// Stories by kind (`story`, `bug`, `chore`).
    pub by_kind: BTreeMap<String, usize>,
    /// Stories by delivery state.
    pub by_delivery: BTreeMap<String, usize>,
}

#[derive(Debug, Serialize)]
pub(crate) struct WarningGroup {
    pub group: &'static str,
    pub count: usize,
    pub lines: Vec<String>,
}

/// The skipped-comment group, which the text report counts rather than lists in full.
pub(crate) const SKIPPED_COMMENTS: &str = "skipped comments";

/// The warning groups in report order; every variant has exactly one.
fn group_of(w: &Warning) -> &'static str {
    match w {
        Warning::IdeaSkipped { .. } => "skipped ideas",
        Warning::OrphanNoIdeaLink { .. } => "orphan epics: no idea link",
        Warning::OrphanIdeaSkipped { .. } => "orphan epics: idea skipped",
        Warning::IssueSkipped { .. } => "skipped issues",
        Warning::CommentSkipped { .. } => SKIPPED_COMMENTS,
        Warning::ParentNotInPlan { .. } => "parent not in the plan",
        Warning::DefaultResolution { .. } => "default resolution",
        Warning::UnresolvedOwner { .. } => "owner left empty",
        Warning::TitleTruncated { .. } => "title truncated",
    }
}
const GROUP_ORDER: [&str; 9] = [
    "skipped ideas",
    "orphan epics: no idea link",
    "orphan epics: idea skipped",
    "skipped issues",
    SKIPPED_COMMENTS,
    "parent not in the plan",
    "default resolution",
    "owner left empty",
    "title truncated",
];

impl Report {
    pub fn of(plan: &ImportPlan) -> Self {
        let mut by_kind = BTreeMap::new();
        let mut by_delivery = BTreeMap::new();
        for s in &plan.stories {
            *by_kind.entry(wire(&s.kind)).or_default() += 1;
            *by_delivery.entry(wire(&s.delivery)).or_default() += 1;
        }
        let warnings = GROUP_ORDER
            .iter()
            .filter_map(|&g| {
                let lines: Vec<String> = plan
                    .warnings
                    .iter()
                    .filter(|w| group_of(w) == g)
                    .map(ToString::to_string)
                    .collect();
                (!lines.is_empty()).then_some(WarningGroup {
                    group: g,
                    count: lines.len(),
                    lines,
                })
            })
            .collect();
        let c = plan.counts();
        Report {
            planned: Planned {
                initiatives: c.initiatives,
                epics: c.epics,
                stories: c.stories,
                comments: c.comments,
                warnings: c.warnings,
                by_kind,
                by_delivery,
            },
            warnings,
            link_types_seen: plan.link_types_seen.clone(),
            grandfathered_stories: plan.grandfathered_stories.clone(),
            marked_epics: plan.marked_epics.clone(),
            marked_initiatives: plan.marked_initiatives.clone(),
            written: false,
            apply: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jira::plan::{PlannedComment, PlannedEpic, PlannedStory};
    use reeve_protocol::{DeliveryState, Kind};
    use std::sync::Mutex;

    /// Records every external reference sent and answers from `refuse` (by reference), else creates.
    #[derive(Default)]
    struct Fake {
        refuse: HashMap<String, ErrorCode>,
        sent: Mutex<Vec<String>>,
        /// Initiatives and epics sent, with the parent key in the request.
        parents: Mutex<Vec<(String, Option<Key>)>>,
    }
    impl Fake {
        fn answer(&self, r: &str, item: bool) -> anyhow::Result<Outcome<Imported>> {
            let mut sent = self.sent.lock().unwrap();
            sent.push(r.to_string());
            if let Some(code) = self.refuse.get(r) {
                return Ok(Outcome::Error {
                    code: *code,
                    message: format!("no {r}"),
                    envelope: true,
                    retry_after: None,
                    current: None,
                });
            }
            let n = sent.len() as u64;
            Ok(Outcome::Ok(Imported {
                key: item.then(|| Key::new("REV", n).unwrap()),
                id: (!item).then_some(n as i64),
                created: true,
            }))
        }
    }
    impl ImportTarget for Fake {
        async fn initiative(&self, b: &ImportInitiative) -> anyhow::Result<Outcome<Imported>> {
            self.answer(&b.external_ref, true)
        }
        async fn epic(&self, b: &ImportEpic) -> anyhow::Result<Outcome<Imported>> {
            self.parents
                .lock()
                .unwrap()
                .push((b.external_ref.clone(), b.initiative.clone()));
            self.answer(&b.external_ref, true)
        }
        async fn story(&self, b: &ImportStory) -> anyhow::Result<Outcome<Imported>> {
            self.parents
                .lock()
                .unwrap()
                .push((b.external_ref.clone(), b.epic.clone()));
            self.answer(&b.external_ref, true)
        }
        async fn comment(&self, b: &ImportComment) -> anyhow::Result<Outcome<Imported>> {
            self.parents
                .lock()
                .unwrap()
                .push((b.external_ref.clone(), Some(b.story.clone())));
            self.answer(&b.external_ref, false)
        }
    }

    fn epic(key: &str, idea: Option<&str>) -> PlannedEpic {
        PlannedEpic {
            key: key.into(),
            idea: idea.map(String::from),
            title: key.into(),
            brief: None,
            discovery_complete: false,
        }
    }
    fn story(key: &str, epic: &str) -> PlannedStory {
        PlannedStory {
            key: key.into(),
            epic: Some(epic.into()),
            title: key.into(),
            kind: Kind::Story,
            description: None,
            criteria: vec![],
            owner: None,
            labels: vec![],
            delivery: DeliveryState::Todo,
            resolution: None,
        }
    }

    /// idea JPD-1 → epic SCRUM-10 → story SCRUM-11 → comment 900; epic SCRUM-30 (no idea) → story SCRUM-31.
    fn plan() -> ImportPlan {
        ImportPlan {
            initiatives: vec![(
                "JPD-1".into(),
                ImportInitiative {
                    external_ref: "JPD-1".into(),
                    title: "Idea".into(),
                    brief: None,
                    horizon: "Now".into(),
                    target_month: None,
                    owner: None,
                    discovery_complete: false,
                },
            )],
            epics: vec![epic("SCRUM-10", Some("JPD-1")), epic("SCRUM-30", None)],
            stories: vec![story("SCRUM-11", "SCRUM-10"), story("SCRUM-31", "SCRUM-30")],
            comments: vec![PlannedComment {
                story: "SCRUM-11".into(),
                external_ref: "jira-comment:900".into(),
                author: "Dan (dan@example.com)".into(),
                body: "hi".into(),
                at: chrono::Utc::now(),
            }],
            ..Default::default()
        }
    }

    fn refusing(pairs: &[(&str, ErrorCode)]) -> Fake {
        Fake {
            refuse: pairs.iter().map(|(r, c)| (r.to_string(), *c)).collect(),
            ..Default::default()
        }
    }

    /// Catches an apply that sends a story with `epic: None` after its epic failed.
    #[tokio::test]
    async fn a_failed_parent_skips_its_descendants_and_the_rest_continues() {
        let fake = refusing(&[("SCRUM-10", ErrorCode::Validation)]);
        let r = apply(&plan(), &fake).await;
        let sent = fake.sent.lock().unwrap().clone();
        assert_eq!(sent, ["JPD-1", "SCRUM-10", "SCRUM-30", "SCRUM-31"]);
        assert_eq!(
            r.skipped,
            [
                (
                    "SCRUM-11".to_string(),
                    "skipped: parent SCRUM-10 failed".to_string()
                ),
                (
                    "jira-comment:900".to_string(),
                    "skipped: parent SCRUM-10 failed".to_string()
                ),
            ]
        );
        assert_eq!(r.failed.len(), 1);
        let f = &r.failed[0];
        assert_eq!(
            (
                f.jira_key.as_str(),
                f.step,
                f.code,
                f.message.as_str(),
                f.exit
            ),
            (
                "SCRUM-10",
                "epic",
                Some(ErrorCode::Validation),
                "no SCRUM-10",
                1
            )
        );
        assert_eq!(
            r.created,
            Counts {
                initiatives: 1,
                epics: 1,
                stories: 1,
                comments: 0
            }
        );
        // Requests carry the created keys of their parents.
        let parents = fake.parents.lock().unwrap().clone();
        assert_eq!(
            parents,
            [
                ("SCRUM-10".to_string(), Some(Key::new("REV", 1).unwrap())),
                ("SCRUM-30".to_string(), None),
                ("SCRUM-31".to_string(), Some(Key::new("REV", 3).unwrap())),
            ]
        );
        assert_eq!(r.exit_code(), 1);
    }

    /// Catches a run that keeps sending after the server refused the caller: one failure, the rest never sent.
    #[tokio::test]
    async fn a_refused_caller_stops_the_run_and_exits_2() {
        for code in [ErrorCode::Forbidden, ErrorCode::Unauthenticated] {
            let fake = refusing(&[("JPD-1", code)]);
            let r = apply(&plan(), &fake).await;
            assert_eq!(*fake.sent.lock().unwrap(), ["JPD-1"]);
            assert_eq!(r.failed.len(), 1);
            let stop = format!("JPD-1 failed ({}: no JPD-1)", wire(&code));
            let why = format!("skipped: run stopped after {stop}");
            let skipped: Vec<(&str, &str)> = r
                .skipped
                .iter()
                .map(|(k, why)| (k.as_str(), why.as_str()))
                .collect();
            assert_eq!(
                skipped,
                [
                    ("SCRUM-10", why.as_str()),
                    ("SCRUM-30", why.as_str()),
                    ("SCRUM-11", why.as_str()),
                    ("SCRUM-31", why.as_str()),
                    ("jira-comment:900", why.as_str()),
                ]
            );
            assert_eq!(r.stopped.as_deref(), Some(stop.as_str()));
            assert_eq!(r.exit_code(), 2);
        }
    }

    /// A `not_found` can be item-specific (a deleted parent), so it does not stop the run.
    #[tokio::test]
    async fn not_found_does_not_stop_the_run() {
        let fake = refusing(&[("JPD-1", ErrorCode::NotFound)]);
        let r = apply(&plan(), &fake).await;
        assert_eq!(
            *fake.sent.lock().unwrap(),
            ["JPD-1", "SCRUM-30", "SCRUM-31"]
        );
        assert!(r.stopped.is_none());
        assert_eq!(r.exit_code(), 1);
    }

    /// With the previous test, catches a classifier that returns 2 on any refusal, or 1 on all of them.
    #[tokio::test]
    async fn a_refusal_and_a_validation_failure_exit_1() {
        let fake = refusing(&[
            ("SCRUM-30", ErrorCode::Validation),
            ("SCRUM-11", ErrorCode::Forbidden),
        ]);
        let r = apply(&plan(), &fake).await;
        let exits: Vec<_> = r
            .failed
            .iter()
            .map(|f| (f.jira_key.as_str(), f.exit))
            .collect();
        assert_eq!(exits, [("SCRUM-30", 1), ("SCRUM-11", 2)]);
        assert_eq!(r.exit_code(), 1);
    }

    #[tokio::test]
    async fn a_transport_error_is_a_failure_without_a_code() {
        struct Down;
        impl ImportTarget for Down {
            async fn initiative(&self, _: &ImportInitiative) -> anyhow::Result<Outcome<Imported>> {
                anyhow::bail!("connection refused")
            }
            async fn epic(&self, _: &ImportEpic) -> anyhow::Result<Outcome<Imported>> {
                anyhow::bail!("connection refused")
            }
            async fn story(&self, _: &ImportStory) -> anyhow::Result<Outcome<Imported>> {
                anyhow::bail!("connection refused")
            }
            async fn comment(&self, _: &ImportComment) -> anyhow::Result<Outcome<Imported>> {
                anyhow::bail!("connection refused")
            }
        }
        let r = apply(&plan(), &Down).await;
        assert_eq!(r.failed.len(), 1, "the first transport error stops the run");
        let f = &r.failed[0];
        assert_eq!((f.cause, f.code, f.exit), (Cause::Transport, None, 1));
        assert!(f.message.contains("connection refused"));
        assert_eq!(r.skipped.len(), 5);
        assert!(
            r.skipped
                .iter()
                .all(|(_, why)| why
                    .starts_with("skipped: run stopped after JPD-1 failed (transport"))
        );
        assert_eq!(r.exit_code(), 1);
    }

    #[tokio::test]
    async fn existing_items_are_counted_as_existing() {
        struct Existing;
        fn old(item: bool) -> anyhow::Result<Outcome<Imported>> {
            Ok(Outcome::Ok(Imported {
                key: item.then(|| Key::new("REV", 7).unwrap()),
                id: (!item).then_some(7),
                created: false,
            }))
        }
        impl ImportTarget for Existing {
            async fn initiative(&self, _: &ImportInitiative) -> anyhow::Result<Outcome<Imported>> {
                old(true)
            }
            async fn epic(&self, _: &ImportEpic) -> anyhow::Result<Outcome<Imported>> {
                old(true)
            }
            async fn story(&self, _: &ImportStory) -> anyhow::Result<Outcome<Imported>> {
                old(true)
            }
            async fn comment(&self, _: &ImportComment) -> anyhow::Result<Outcome<Imported>> {
                old(false)
            }
        }
        let r = apply(&plan(), &Existing).await;
        assert_eq!(r.created, Counts::default());
        assert_eq!(
            r.existing,
            Counts {
                initiatives: 1,
                epics: 2,
                stories: 2,
                comments: 1
            }
        );
        assert_eq!(r.exit_code(), 0);
    }

    /// Catches a warning kind that falls out of the report, or two kinds merged into one group.
    #[test]
    fn every_warning_kind_has_its_own_group_in_report_order() {
        use crate::jira::plan::SkipWhy;
        let k = || "SCRUM-1".to_string();
        let all = [
            Warning::IdeaSkipped {
                idea: k(),
                why: SkipWhy::UnmappedBucket("x".into()),
            },
            Warning::OrphanNoIdeaLink { epic: k() },
            Warning::OrphanIdeaSkipped {
                epic: k(),
                idea: k(),
                why: SkipWhy::UnmappedBucket("x".into()),
            },
            Warning::IssueSkipped {
                issue: k(),
                why: SkipWhy::UnmappedType("x".into()),
            },
            Warning::CommentSkipped {
                issue: k(),
                comment_id: "1".into(),
            },
            Warning::ParentNotInPlan {
                story: k(),
                parent: k(),
            },
            Warning::DefaultResolution {
                issue: k(),
                jira: None,
            },
            Warning::UnresolvedOwner {
                issue: k(),
                account_id: "a".into(),
            },
            Warning::TitleTruncated { issue: k() },
        ];
        let groups: Vec<&str> = all.iter().map(group_of).collect();
        assert_eq!(groups, GROUP_ORDER);
        let r = Report::of(&ImportPlan {
            warnings: all.to_vec(),
            ..Default::default()
        });
        assert_eq!(r.warnings.len(), GROUP_ORDER.len());
        assert!(
            r.warnings
                .iter()
                .all(|g| g.count == 1 && g.lines.len() == 1)
        );
    }

    /// The text report says a rehearsal wrote nothing, and names each failure and skip after an apply.
    #[tokio::test]
    async fn the_text_report_names_failures_skips_and_a_rehearsal() {
        let p = plan();
        let mut report = Report::of(&p);
        let text = crate::render::import_report(&report);
        assert!(
            text.contains("Planned: 1 initiatives, 2 epics, 2 stories, 1 comments"),
            "{text}"
        );
        assert!(text.contains("nothing written"), "{text}");
        let fake = refusing(&[("SCRUM-10", ErrorCode::Validation)]);
        report.apply = Some(apply(&p, &fake).await);
        let text = crate::render::import_report(&report);
        assert!(!text.contains("nothing written"), "{text}");
        assert!(
            text.contains("SCRUM-10 (epic): validation: no SCRUM-10"),
            "{text}"
        );
        assert!(
            text.contains("SCRUM-11: skipped: parent SCRUM-10 failed"),
            "{text}"
        );
    }

    /// Catches a code rendered from Debug (`notfound`) or a refusal shown as a transport error.
    #[test]
    fn failure_codes_render_by_their_wire_names() {
        let f = |cause, code| Failure {
            jira_key: "SCRUM-1".into(),
            step: "story",
            cause,
            code,
            message: "m".into(),
            exit: 1,
        };
        assert_eq!(
            f(Cause::Error, Some(ErrorCode::NotFound)).code_word(),
            "not_found"
        );
        assert_eq!(
            f(Cause::Error, Some(ErrorCode::Forbidden)).code_word(),
            "forbidden"
        );
        assert_eq!(
            f(Cause::Error, Some(ErrorCode::Unauthenticated)).code_word(),
            "unauthenticated"
        );
        assert_eq!(f(Cause::Refused, None).code_word(), "refused");
        assert_eq!(f(Cause::Transport, None).code_word(), "transport");
        let mut report = Report::of(&plan());
        report.apply = Some(ApplyReport {
            failed: vec![f(Cause::Error, Some(ErrorCode::NotFound))],
            ..Default::default()
        });
        let text = crate::render::import_report(&report);
        assert!(text.contains("SCRUM-1 (story): not_found: m"), "{text}");
    }
}
