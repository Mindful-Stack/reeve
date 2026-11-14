use super::{finish, whoami};
use crate::client::Client;
use crate::glyphs::Level;
use clap::Subcommand;
use reeve_protocol::*;

#[derive(Subcommand)]
pub enum Story {
    /// Create a story, bug or chore. A repeated --idempotency-key returns the same story.
    Create {
        /// Caller-chosen key that makes a retried create safe.
        #[arg(long)]
        idempotency_key: String,
        /// Short title.
        #[arg(long)]
        title: String,
        /// Kind of work.
        #[arg(long, value_enum, default_value = "story")]
        kind: KindArg,
        /// Epic key (e.g. ABC-12). Without one the item is untriaged until triage places it.
        #[arg(long)]
        epic: Option<KeyRef>,
        /// Longer description.
        #[arg(long, default_value = "")]
        description: String,
        /// Read the description from this file, unchanged (no shell quoting in between); `-` reads stdin.
        #[arg(long, conflicts_with = "description", value_name = "PATH")]
        description_file: Option<std::path::PathBuf>,
        /// An acceptance criterion; repeat for several.
        #[arg(long = "criterion")]
        criteria: Vec<String>,
        /// A label; repeat for several.
        #[arg(long = "label")]
        labels: Vec<String>,
        /// The owner (email); it must be an allowed user.
        #[arg(long)]
        owner: Option<String>,
        /// The key this item was migrated from, e.g. JPD-30. A later `reeve import jira` finds this item by it
        /// instead of creating another.
        #[arg(long)]
        external_ref: Option<String>,
    },
    /// Show one story.
    Show {
        /// Story key, e.g. ABC-412.
        key: KeyRef,
    },
    /// List stories, optionally filtered. Prints key, state, readiness and title.
    Ls {
        /// Only stories in this epic.
        #[arg(long)]
        epic: Option<KeyRef>,
        /// Only stories owned by this email.
        #[arg(long)]
        owner: Option<String>,
        /// Only stories in this delivery state.
        #[arg(long, value_enum)]
        delivery: Option<StateArg>,
        /// Only stories with this label.
        #[arg(long)]
        label: Option<String>,
        /// Only items with no epic that are not done.
        #[arg(long)]
        untriaged: bool,
        /// Only pickable work: ready stories still in todo.
        #[arg(long)]
        ready: bool,
        /// Only stories owned by you (resolved with GET /me).
        #[arg(long)]
        mine: bool,
        /// Only stories serving the active sprint's goals: a goal id, `any` (some goal) or `none` (upkeep).
        #[arg(long, value_name = "ID|any|none")]
        goal: Option<String>,
        /// Only stories whose title, description or external reference contains this text (any case, 1 to 200
        /// characters); a key-shaped text (`12`, `#12`, `ABC-12`) also finds that key.
        #[arg(long)]
        text: Option<String>,
        /// Only stories of this kind.
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
        /// Only stories in this initiative's epics (a key or external reference).
        #[arg(long)]
        initiative: Option<KeyRef>,
        /// Only stories shown as urgent.
        #[arg(long)]
        urgent: bool,
        /// Only stories that are not done; combines with --delivery (both must hold, so with --delivery done it
        /// lists nothing).
        #[arg(long)]
        open: bool,
        /// Only stories with a promise (their own, or their epic's) in this state; kept, late and dropped promises never
        /// match.
        #[arg(long, value_enum)]
        promised: Option<PromisedArg>,
        /// The order: `rank` (the default) or `relevance` (needs --text: the exact key or external reference first,
        /// then title matches).
        #[arg(long, value_enum)]
        sort: Option<SortArg>,
        /// At most this many stories, 1 to 100: one page of the list, in its order. Without it, the whole list.
        #[arg(long)]
        limit: Option<u32>,
        /// Skip this many stories first (needs --limit). After a full page, the flags for the next page are
        /// printed on standard error.
        #[arg(long)]
        offset: Option<u32>,
    },
    /// Stories with a similar title (fuzzy match on the title): open ones, and ones done recently with their resolution. Check before filing.
    Similar {
        /// The text, 8 to 200 characters; several words are one text.
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
        /// At most this many, 1 to 10.
        #[arg(long, default_value_t = 5)]
        limit: i64,
    },
    /// Edit a story; the --version must be the one you last read.
    Edit {
        /// Story key.
        key: KeyRef,
        /// The story version you are editing (optimistic concurrency).
        #[arg(long)]
        version: i64,
        /// New title.
        #[arg(long)]
        title: Option<String>,
        /// Reclassify the story.
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
        /// Move the story to this epic.
        #[arg(long, conflicts_with = "no_epic")]
        epic: Option<KeyRef>,
        /// Detach the story from its epic.
        #[arg(long)]
        no_epic: bool,
        /// Replace the acceptance criteria; repeat for several. A criterion whose text is unchanged at the same position keeps its done flag.
        #[arg(long = "criterion")]
        criteria: Vec<String>,
        /// New rank within the backlog.
        #[arg(long)]
        rank: Option<i32>,
        /// Assign the story (email); the owner must be an allowed user.
        #[arg(long, conflicts_with = "no_owner")]
        owner: Option<String>,
        /// Clear the story's owner.
        #[arg(long)]
        no_owner: bool,
        /// New description (replaces the whole description).
        #[arg(long)]
        description: Option<String>,
        /// Read the description from this file, unchanged (no shell quoting in between); `-` reads stdin.
        #[arg(long, conflicts_with = "description", value_name = "PATH")]
        description_file: Option<std::path::PathBuf>,
        /// Add a label; repeat for several. A label the story already has is kept once.
        #[arg(long = "add-label")]
        add_labels: Vec<String>,
        /// Remove a label; repeat for several.
        #[arg(long = "remove-label")]
        remove_labels: Vec<String>,
        /// Record the key this item was migrated from, e.g. JPD-30, on an item that has none. It cannot be changed
        /// once set; a later `reeve import jira` finds this item by it instead of creating another.
        #[arg(long)]
        external_ref: Option<String>,
    },
    /// Move between todo, in-progress, in-review, testing, done. Entering active work from todo or done needs readiness or `--override`.
    Mv {
        /// Story key.
        key: KeyRef,
        /// Target delivery state.
        to: StateArg,
        /// The story version you last read. Omitting it fetches the current version and uses it, which skips optimistic concurrency.
        #[arg(long)]
        version: Option<i64>,
        /// Required when moving to done.
        #[arg(long, value_enum)]
        resolution: Option<ResolutionArg>,
        /// Override the readiness gate, giving a reason (interactive tokens only).
        #[arg(long = "override")]
        override_reason: Option<String>,
    },
    /// Show whether the story is ready to start, and if not, why.
    Ready {
        /// Story key.
        key: KeyRef,
    },
    /// Print everything needed before starting: brief, decisions with record paths, blockers, criteria, comments (Markdown, or JSON with --json).
    Context {
        /// Story key.
        key: KeyRef,
    },
    /// Record that another story blocks this one.
    Block {
        /// The blocked story.
        key: KeyRef,
        /// The blocking story.
        #[arg(long)]
        by: KeyRef,
    },
    /// Remove the record that another story blocks this one. Removing a block that does not exist succeeds and changes nothing.
    Unblock {
        /// The blocked story.
        key: KeyRef,
        /// The blocking story.
        #[arg(long)]
        by: KeyRef,
    },
    /// Mark the story urgent for one cycle, or renew its mark; --clear removes it (interactive tokens only).
    Urgent {
        /// Story key.
        key: KeyRef,
        /// Why it is urgent (at most 200 characters); left out, marking again keeps the current reason; --reason '' clears it.
        #[arg(long, conflicts_with = "clear")]
        reason: Option<String>,
        /// Remove the story's urgent mark (clearing one with none changes nothing).
        #[arg(long)]
        clear: bool,
    },
    /// Promise the story by a date, change the promise or its recipient, or --clear it (interactive tokens only). A
    /// promise shows on the story and gates nothing.
    Promise {
        /// Story key.
        key: KeyRef,
        #[command(flatten)]
        args: super::PromiseArgs,
    },
}
#[derive(Clone, Copy, clap::ValueEnum)]
pub enum KindArg {
    Story,
    Bug,
    Chore,
}
/// A query value as the server spells it: the serialised name of a unit enum variant; anything that does not serialise
/// to a string is an error, never an empty value.
fn wire<T: serde::Serialize>(v: T) -> anyhow::Result<String> {
    match serde_json::to_value(v)? {
        serde_json::Value::String(s) => Ok(s),
        other => anyhow::bail!("{other} is not a query value"),
    }
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum SortArg {
    Rank,
    Relevance,
}
impl From<SortArg> for StorySort {
    fn from(s: SortArg) -> Self {
        match s {
            SortArg::Rank => StorySort::Rank,
            SortArg::Relevance => StorySort::Relevance,
        }
    }
}
#[derive(Clone, Copy, clap::ValueEnum)]
pub enum PromisedArg {
    /// Overdue, or open and due by the end of the active sprint.
    #[value(name = "this-sprint")]
    ThisSprint,
    /// Open and due by the end of the draft (next) sprint.
    #[value(name = "next-sprint")]
    NextSprint,
    /// Overdue: past the date and not done.
    Overdue,
    /// Every open or overdue promise.
    Any,
}
impl From<PromisedArg> for PromisedFilter {
    fn from(p: PromisedArg) -> Self {
        match p {
            PromisedArg::ThisSprint => PromisedFilter::ThisSprint,
            PromisedArg::NextSprint => PromisedFilter::NextSprint,
            PromisedArg::Overdue => PromisedFilter::Overdue,
            PromisedArg::Any => PromisedFilter::Any,
        }
    }
}
#[derive(Clone, Copy, clap::ValueEnum)]
pub enum StateArg {
    Todo,
    #[value(name = "in-progress")]
    InProgress,
    #[value(name = "in-review")]
    InReview,
    Testing,
    Done,
}
#[derive(Clone, Copy, clap::ValueEnum)]
pub enum ResolutionArg {
    Completed,
    Dropped,
}
impl From<KindArg> for Kind {
    fn from(k: KindArg) -> Self {
        match k {
            KindArg::Story => Kind::Story,
            KindArg::Bug => Kind::Bug,
            KindArg::Chore => Kind::Chore,
        }
    }
}
impl From<StateArg> for DeliveryState {
    fn from(s: StateArg) -> Self {
        match s {
            StateArg::Todo => DeliveryState::Todo,
            StateArg::InProgress => DeliveryState::InProgress,
            StateArg::InReview => DeliveryState::InReview,
            StateArg::Testing => DeliveryState::Testing,
            StateArg::Done => DeliveryState::Done,
        }
    }
}
impl From<ResolutionArg> for Resolution {
    fn from(r: ResolutionArg) -> Self {
        match r {
            ResolutionArg::Completed => Resolution::Completed,
            ResolutionArg::Dropped => Resolution::Dropped,
        }
    }
}

pub async fn run(c: &Client, s: Story, json: bool) -> anyhow::Result<i32> {
    Ok(match s {
        Story::Create {
            idempotency_key,
            title,
            kind,
            epic,
            description,
            description_file,
            criteria,
            labels,
            owner,
            external_ref,
        } => finish(
            c.post::<StoryDto>(
                "/stories",
                serde_json::to_value(CreateStory {
                    idempotency_key,
                    title,
                    kind: kind.into(),
                    epic,
                    description: Some(
                        super::text_arg(Some(description), description_file)?.unwrap_or_default(),
                    ),
                    criteria,
                    labels,
                    external_ref,
                    owner,
                    attachments: vec![],
                })?,
            )
            .await?,
            json,
            crate::render::story,
        ),
        Story::Similar { text, limit } => finish(
            c.get_query::<Vec<SimilarStory>>(
                "/stories/similar",
                &[("q", text.join(" ")), ("limit", limit.to_string())],
            )
            .await?,
            json,
            |hits| render_similar(hits),
        ),
        Story::Urgent { key, reason, clear } => {
            super::urgent::<StoryDto>(c, format!("/stories/{key}"), reason, clear, json, |s| {
                super::UrgentView {
                    level: Level::of_kind(s.kind),
                    key: &s.key,
                    own: s.urgent_mark.as_ref(),
                    sources: &s.urgent_sources,
                }
            })
            .await?
        }
        Story::Promise { key, args } => {
            super::promise::<StoryDto, StoryPromiseCleared>(
                c,
                format!("/stories/{key}"),
                args,
                json,
                crate::render::story_promise_cleared,
                crate::render::story_promise_answer,
            )
            .await?
        }
        Story::Show { key } => {
            let out = c.get::<StoryDto>(&format!("/stories/{key}")).await?;
            let now = chrono::Utc::now();
            finish(out, json, |s| crate::render::story_show(s, now))
        }
        Story::Ls {
            epic,
            owner,
            delivery,
            label,
            untriaged,
            ready,
            mine,
            goal,
            text,
            kind,
            initiative,
            urgent,
            open,
            promised,
            sort,
            limit,
            offset,
        } => {
            let owner = if mine {
                match whoami(c).await? {
                    Ok(me) => Some(me),
                    Err(refused) => return Ok(finish(refused, json, |_| String::new())),
                }
            } else {
                owner
            }
            .map(|o| reeve_protocol::normalise_email(&o));
            let mut q: Vec<(&str, String)> = vec![];
            if let Some(e) = epic {
                q.push(("epic", e.to_string()));
            }
            if let Some(o) = owner {
                q.push(("owner", o));
            }
            if let Some(d) = delivery {
                q.push(("delivery", wire(DeliveryState::from(d))?));
            }
            if let Some(l) = label {
                q.push(("label", l));
            }
            if untriaged {
                q.push(("untriaged", "true".into()));
            }
            if ready {
                q.push(("ready", "true".into()));
            }
            if let Some(g) = goal {
                q.push(("goal", g));
            }
            if let Some(t) = text {
                q.push(("q", t));
            }
            if let Some(k) = kind {
                q.push(("kind", wire(Kind::from(k))?));
            }
            if let Some(i) = initiative {
                q.push(("initiative", i.to_string()));
            }
            if urgent {
                q.push(("urgent", "true".into()));
            }
            if open {
                q.push(("done", wire(DoneFilter::None)?));
            }
            if let Some(p) = promised {
                q.push(("promised", wire(PromisedFilter::from(p))?));
            }
            if let Some(s) = sort {
                q.push(("sort", wire(StorySort::from(s))?));
            }
            if let Some(l) = limit {
                q.push(("limit", l.to_string()));
            }
            if let Some(o) = offset {
                q.push(("offset", o.to_string()));
            }
            let out = c
                .get_query::<Vec<StoryWithReadiness>>("/stories", &q)
                .await?;
            // A page of exactly `limit` rows may have a successor. Text mode says how to ask for it, on standard
            // error, so nothing that reads rows from standard output takes the line for one; JSON mode prints the
            // envelope and nothing else.
            let more = match (&out, limit) {
                (crate::client::Outcome::Ok(rows), Some(l))
                    if !json && rows.len() == l as usize =>
                {
                    Some(format!(
                        "more: --limit {l} --offset {}",
                        u64::from(offset.unwrap_or(0)) + u64::from(l)
                    ))
                }
                _ => None,
            };
            let code = finish(out, json, |rows| {
                rows.iter()
                    .map(|r| crate::render::story_row(&r.story, &r.readiness))
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            if let Some(line) = more {
                eprintln!("{line}");
            }
            code
        }
        Story::Edit {
            key,
            version,
            title,
            kind,
            epic,
            no_epic,
            criteria,
            rank,
            owner,
            no_owner,
            description,
            description_file,
            add_labels,
            remove_labels,
            external_ref,
        } => {
            let mut body = serde_json::json!({"version": version});
            if let Some(t) = title {
                body["title"] = t.into();
            }
            if let Some(k) = kind {
                body["kind"] = serde_json::to_value(Kind::from(k))?;
            }
            if no_epic {
                body["epic"] = serde_json::Value::Null;
            } else if let Some(e) = epic {
                body["epic"] = serde_json::to_value(e)?;
            }
            if !criteria.is_empty() {
                let current = match c.get::<StoryDto>(&format!("/stories/{key}")).await? {
                    crate::client::Outcome::Ok(s) => s.criteria,
                    other => return Ok(finish(other, json, crate::render::story)),
                };
                body["criteria"] = serde_json::to_value(replace_criteria(&current, &criteria))?;
            }
            if let Some(r) = rank {
                body["rank"] = r.into();
            }
            if no_owner {
                body["owner"] = serde_json::Value::Null;
            } else if let Some(o) = owner {
                body["owner"] = o.into();
            }
            if let Some(d) = super::text_arg(description, description_file)? {
                body["description"] = d.into();
            }
            if !add_labels.is_empty() || !remove_labels.is_empty() {
                let current = match c.get::<StoryDto>(&format!("/stories/{key}")).await? {
                    crate::client::Outcome::Ok(s) => s.labels,
                    other => return Ok(finish(other, json, crate::render::story)),
                };
                body["labels"] = edit_labels(current, &add_labels, &remove_labels).into();
            }
            if let Some(r) = external_ref {
                body["external_ref"] = r.into();
            }
            finish(
                c.patch::<StoryDto>(&format!("/stories/{key}"), body)
                    .await?,
                json,
                crate::render::story,
            )
        }
        Story::Mv {
            key,
            to,
            version,
            resolution,
            override_reason,
        } => {
            let explicit_version = version.is_some();
            let version = match version {
                Some(v) => v,
                None => match c.get::<StoryDto>(&format!("/stories/{key}")).await? {
                    crate::client::Outcome::Ok(s) => s.version,
                    other => return Ok(finish(other, json, crate::render::story)),
                },
            };
            let mut out = c
                .post::<StoryDto>(
                    &format!("/stories/{key}/move"),
                    serde_json::json!({"version": version, "to": DeliveryState::from(to), "resolution": resolution.map(Resolution::from), "override_reason": override_reason}),
                )
                .await?;
            if !explicit_version
                && let crate::client::Outcome::Error {
                    code: ErrorCode::Conflict,
                    message,
                    ..
                } = &mut out
            {
                message.push_str(" (the story changed while moving; re-run to retry)");
            }
            finish(out, json, crate::render::story)
        }
        Story::Ready { key } => finish(
            c.get::<Readiness>(&format!("/stories/{key}/ready")).await?,
            json,
            crate::render::readiness,
        ),
        Story::Context { key } => {
            let out = c
                .get::<StoryContextDto>(&format!("/stories/{key}/context"))
                .await?;
            let now = chrono::Utc::now();
            finish(out, json, |x| crate::render::context_markdown(x, now))
        }
        Story::Block { key, by } => finish(
            c.post::<reeve_protocol::Empty>(
                &format!("/stories/{key}/blocks"),
                serde_json::json!({"blocker": by}),
            )
            .await?,
            json,
            |_| "blocked".into(),
        ),
        Story::Unblock { key, by } => finish(
            c.delete::<reeve_protocol::Empty>(&format!("/stories/{key}/blocks/{by}"))
                .await?,
            json,
            |_| "unblocked".into(),
        ),
    })
}

/// Labels are replaced as a list on the wire: the story's labels in their order without the removed ones, then each
/// added label it does not have yet. The PATCH carries the version the caller read, so a label changed meanwhile is a
/// conflict, not a lost update.
fn edit_labels(current: Vec<String>, add: &[String], remove: &[String]) -> Vec<String> {
    let mut out: Vec<String> = current
        .into_iter()
        .filter(|l| !remove.contains(l))
        .collect();
    for l in add {
        if !out.contains(l) && !remove.contains(l) {
            out.push(l.clone());
        }
    }
    out
}

/// Criteria are replaced as a list, but a criterion whose text is unchanged at the same position keeps its done flag.
fn replace_criteria(current: &[CriterionDto], texts: &[String]) -> Vec<CriterionDto> {
    texts
        .iter()
        .enumerate()
        .map(|(i, t)| CriterionDto {
            text: t.clone(),
            done: current
                .get(i)
                .is_some_and(|c| c.done && c.text.trim() == t.trim()),
        })
        .collect()
}

fn render_similar(hits: &[SimilarStory]) -> String {
    if hits.is_empty() {
        return "nothing similar".into();
    }
    hits.iter()
        .map(|h| {
            let state = match (h.delivery, h.resolution) {
                (DeliveryState::Done, Some(Resolution::Dropped)) => "done, dropped",
                (DeliveryState::Done, Some(Resolution::Completed)) => "done, completed",
                (d, _) => crate::render::state_word(d),
            };
            format!(
                "{}\t{}\t{}",
                crate::glyphs::mark(crate::glyphs::Level::of_kind(h.kind), &h.key),
                state,
                h.title
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A query value is the variant's wire name; a value that does not serialise to a string is refused, not sent
    /// empty.
    #[test]
    fn wire_spells_variants_and_refuses_the_rest() {
        assert_eq!(wire(DeliveryState::InProgress).unwrap(), "in_progress");
        assert_eq!(wire(StorySort::Relevance).unwrap(), "relevance");
        assert!(wire(7u8).is_err());
        assert!(wire(Option::<Kind>::None).is_err());
    }

    fn hit(key: &str, delivery: DeliveryState, resolution: Option<Resolution>) -> SimilarStory {
        SimilarStory {
            key: key.parse().unwrap(),
            kind: Kind::Story,
            title: "Booking flow".into(),
            epic: None,
            delivery,
            resolution,
        }
    }

    #[test]
    fn similar_prints_state_and_marks_a_dropped_duplicate() {
        let out = render_similar(&[
            hit("TST-1", DeliveryState::InProgress, None),
            hit("TST-2", DeliveryState::Done, Some(Resolution::Dropped)),
            hit("TST-3", DeliveryState::Done, Some(Resolution::Completed)),
        ]);
        let lines: Vec<&str> = out.lines().collect();
        assert!(
            lines[0].contains("TST-1\tin-progress\tBooking flow"),
            "{out}"
        );
        assert!(
            lines[1].contains("TST-2\tdone, dropped\tBooking flow"),
            "{out}"
        );
        assert!(
            lines[2].contains("TST-3\tdone, completed\tBooking flow"),
            "{out}"
        );
        assert_eq!(render_similar(&[]), "nothing similar");
    }

    #[test]
    fn replacing_criteria_keeps_done_where_the_text_is_unchanged_at_the_same_index() {
        let cur = vec![
            CriterionDto {
                text: "a".into(),
                done: true,
            },
            CriterionDto {
                text: "b".into(),
                done: true,
            },
        ];
        let next = replace_criteria(&cur, &["a".into(), "B2".into(), "c".into()]);
        assert_eq!(
            next.iter().map(|c| c.done).collect::<Vec<_>>(),
            [true, false, false]
        );
        // Position matters: the same text at another index starts not done.
        let swapped = replace_criteria(&cur, &["b".into(), "a".into()]);
        assert!(swapped.iter().all(|c| !c.done));
    }
}
