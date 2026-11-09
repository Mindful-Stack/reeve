use super::finish;
use crate::client::Client;
use crate::glyphs::Level;
use clap::Subcommand;
use reeve_protocol::*;

#[derive(Subcommand)]
pub enum Epic {
    /// Create an epic. A repeated --idempotency-key returns the same epic.
    Create {
        /// Caller-chosen key that makes a retried create safe.
        #[arg(long)]
        idempotency_key: String,
        /// Short title.
        #[arg(long)]
        title: String,
        /// Initiative the epic belongs to.
        #[arg(long)]
        initiative: Option<KeyRef>,
        /// The brief: what this epic delivers.
        #[arg(long)]
        brief: Option<String>,
        /// Read the brief from this file, unchanged (no shell quoting in between); `-` reads stdin.
        #[arg(long, conflicts_with = "brief", value_name = "PATH")]
        brief_file: Option<std::path::PathBuf>,
        /// The key this item was migrated from, e.g. JPD-30. A later `reeve import jira` finds this item by it
        /// instead of creating another.
        #[arg(long)]
        external_ref: Option<String>,
    },
    /// Show one epic, with its advisory findings in text output.
    Show {
        /// Epic key.
        key: KeyRef,
    },
    /// Advisory findings on one epic: computed on read, they never block a move.
    Findings {
        /// Epic key.
        key: KeyRef,
    },
    /// List epics.
    Ls {
        /// Only epics in this initiative.
        #[arg(long)]
        initiative: Option<KeyRef>,
    },
    /// Edit an epic; the --version must be the one you last read.
    Edit {
        /// Epic key.
        key: KeyRef,
        /// The epic version you are editing (optimistic concurrency).
        #[arg(long)]
        version: i64,
        /// New title.
        #[arg(long)]
        title: Option<String>,
        /// New brief.
        #[arg(long)]
        brief: Option<String>,
        /// Read the brief from this file, unchanged (no shell quoting in between); `-` reads stdin.
        #[arg(long, conflicts_with = "brief", value_name = "PATH")]
        brief_file: Option<std::path::PathBuf>,
        /// Move the epic to this initiative.
        #[arg(long, conflicts_with = "no_initiative")]
        initiative: Option<KeyRef>,
        /// Detach the epic from its initiative.
        #[arg(long)]
        no_initiative: bool,
        /// Record the key this item was migrated from, e.g. JPD-30, on an item that has none. It cannot be changed
        /// once set; a later `reeve import jira` finds this item by it instead of creating another.
        #[arg(long)]
        external_ref: Option<String>,
    },
    /// Mark discovery complete (--complete) or reopen it (--reopen). Stories in it are not ready until discovery is complete.
    Discovery {
        /// Epic key.
        key: KeyRef,
        /// The epic version you last read.
        #[arg(long)]
        version: i64,
        /// Mark discovery complete.
        #[arg(long, conflicts_with = "reopen", required_unless_present = "reopen")]
        complete: bool,
        /// Reopen discovery.
        #[arg(long)]
        reopen: bool,
    },
    /// Mark the epic urgent for one cycle, or renew its mark; --clear removes it (interactive tokens only).
    Urgent {
        /// Epic key.
        key: KeyRef,
        /// Why it is urgent (optional, at most 200 characters).
        #[arg(long, conflicts_with = "clear")]
        reason: Option<String>,
        /// Remove the epic's urgent mark (clearing one with none changes nothing).
        #[arg(long)]
        clear: bool,
    },
}

pub async fn run(c: &Client, e: Epic, json: bool) -> anyhow::Result<i32> {
    Ok(match e {
        Epic::Create {
            idempotency_key,
            title,
            initiative,
            brief,
            brief_file,
            external_ref,
        } => finish(
            c.post::<EpicDto>(
                "/epics",
                serde_json::to_value(CreateEpic {
                    idempotency_key,
                    title,
                    initiative,
                    brief: super::text_arg(brief, brief_file)?,
                    external_ref,
                })?,
            )
            .await?,
            json,
            crate::render::epic,
        ),
        Epic::Urgent { key, reason, clear } => {
            let path = format!("/epics/{key}/urgent");
            if clear {
                finish(c.delete::<EpicDto>(&path).await?, json, |s| {
                    format!("urgent mark cleared on {}", s.key)
                })
            } else {
                let now = chrono::Utc::now();
                let reason = reason
                    .map(|r| r.trim().to_string())
                    .filter(|r| !r.is_empty());
                finish(
                    c.post::<EpicDto>(&path, serde_json::to_value(MarkUrgent { reason })?)
                        .await?,
                    json,
                    |s| {
                        crate::render::urgent_answer(
                            Level::Epic,
                            &s.key,
                            s.urgent_mark.as_ref(),
                            &s.urgent_sources,
                            now,
                        )
                    },
                )
            }
        }
        Epic::Show { key } => {
            let out = c.get::<EpicDto>(&format!("/epics/{key}")).await?;
            let now = chrono::Utc::now();
            if json || !matches!(out, crate::client::Outcome::Ok(_)) {
                finish(out, json, |v| crate::render::epic_show(v, now))
            } else {
                // Advisory and best effort: a failed findings read (an older server, a dropped connection) still shows the
                // epic.
                let fs = match c
                    .get::<Vec<Finding>>(&format!("/epics/{key}/findings"))
                    .await
                {
                    Ok(crate::client::Outcome::Ok(fs)) => fs,
                    _ => vec![],
                };
                // Its comments, best effort like the findings.
                let cs = match c
                    .get::<Vec<CommentDto>>(&format!("/items/{key}/comments"))
                    .await
                {
                    Ok(crate::client::Outcome::Ok(cs)) => cs,
                    _ => vec![],
                };
                finish(out, false, |v| {
                    let f = [crate::render::findings(&fs), crate::render::comments(&cs)]
                        .into_iter()
                        .filter(|b| !b.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n");
                    let s = crate::render::epic_show(v, now);
                    if f.is_empty() { s } else { format!("{s}\n{f}") }
                })
            }
        }
        Epic::Findings { key } => finish(
            c.get::<Vec<Finding>>(&format!("/epics/{key}/findings"))
                .await?,
            json,
            |fs| {
                if fs.is_empty() {
                    "no findings".to_string()
                } else {
                    crate::render::findings(fs)
                }
            },
        ),
        Epic::Ls { initiative } => {
            let path = match initiative {
                Some(i) => format!("/epics?initiative={i}"),
                None => "/epics".into(),
            };
            finish(c.get::<Vec<EpicDto>>(&path).await?, json, |xs| {
                xs.iter()
                    .map(crate::render::epic)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        }
        Epic::Edit {
            key,
            version,
            title,
            brief,
            brief_file,
            initiative,
            no_initiative,
            external_ref,
        } => {
            let edit = EditItem {
                version,
                title,
                brief: super::text_arg(brief, brief_file)?,
                initiative: if no_initiative {
                    Some(None)
                } else {
                    initiative.map(Some)
                },
                external_ref,
                ..Default::default()
            };
            finish(
                c.patch::<EpicDto>(&format!("/epics/{key}"), serde_json::to_value(edit)?)
                    .await?,
                json,
                crate::render::epic,
            )
        }
        Epic::Discovery {
            key,
            version,
            complete,
            reopen: _,
        } => finish(
            c.post::<EpicDto>(
                &format!("/epics/{key}/discovery"),
                serde_json::to_value(DiscoveryMark { version, complete })?,
            )
            .await?,
            json,
            crate::render::epic,
        ),
    })
}
