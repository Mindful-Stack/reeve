use super::finish;
use crate::client::Client;
use clap::Subcommand;
use reeve_protocol::*;

#[derive(Subcommand)]
pub enum Initiative {
    /// Create an initiative. A repeated --idempotency-key returns the same initiative.
    Create {
        /// Caller-chosen key that makes a retried create safe.
        #[arg(long)]
        idempotency_key: String,
        /// Short title.
        #[arg(long)]
        title: String,
        /// Planning horizon, e.g. "H1 2027" or "now".
        #[arg(long)]
        horizon: String,
        /// The brief: why this initiative exists.
        #[arg(long)]
        brief: Option<String>,
        /// Read the brief from this file, unchanged (no shell quoting in between); `-` reads stdin.
        #[arg(long, conflicts_with = "brief", value_name = "PATH")]
        brief_file: Option<std::path::PathBuf>,
        /// Target month, e.g. 2027-03.
        #[arg(long)]
        target_month: Option<String>,
        /// Owner (email).
        #[arg(long)]
        owner: Option<String>,
        /// The key this item was migrated from, e.g. JPD-30. A later `reeve import jira` finds this item by it
        /// instead of creating another.
        #[arg(long)]
        external_ref: Option<String>,
    },
    /// Show one initiative, with its advisory findings in text output.
    Show {
        /// Initiative key.
        key: KeyRef,
    },
    /// Advisory findings on one initiative: computed on read, they never block a move.
    Findings {
        /// Initiative key.
        key: KeyRef,
    },
    /// List initiatives.
    Ls,
    /// Edit an initiative; the --version must be the one you last read.
    Edit {
        /// Initiative key.
        key: KeyRef,
        /// The initiative version you are editing (optimistic concurrency).
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
        /// New horizon.
        #[arg(long)]
        horizon: Option<String>,
        /// New target month, e.g. 2027-03.
        #[arg(long, conflicts_with = "no_target_month")]
        target_month: Option<String>,
        /// Clear the target month.
        #[arg(long)]
        no_target_month: bool,
        /// New owner (email).
        #[arg(long, conflicts_with = "no_owner")]
        owner: Option<String>,
        /// Clear the owner.
        #[arg(long)]
        no_owner: bool,
        /// Record the key this item was migrated from, e.g. JPD-30, on an item that has none. It cannot be changed
        /// once set; a later `reeve import jira` finds this item by it instead of creating another.
        #[arg(long)]
        external_ref: Option<String>,
    },
    /// Mark discovery complete (--complete) or reopen it (--reopen). Stories under it are not ready until discovery is complete.
    Discovery {
        /// Initiative key.
        key: KeyRef,
        /// The initiative version you last read.
        #[arg(long)]
        version: i64,
        /// Mark discovery complete.
        #[arg(long, conflicts_with = "reopen", required_unless_present = "reopen")]
        complete: bool,
        /// Reopen discovery.
        #[arg(long)]
        reopen: bool,
    },
}

pub async fn run(c: &Client, i: Initiative, json: bool) -> anyhow::Result<i32> {
    Ok(match i {
        Initiative::Create {
            idempotency_key,
            title,
            horizon,
            brief,
            brief_file,
            target_month,
            owner,
            external_ref,
        } => finish(
            c.post::<InitiativeDto>(
                "/initiatives",
                serde_json::to_value(CreateInitiative {
                    idempotency_key,
                    title,
                    brief: super::text_arg(brief, brief_file)?,
                    horizon,
                    target_month,
                    owner,
                    external_ref,
                })?,
            )
            .await?,
            json,
            crate::render::initiative,
        ),
        Initiative::Show { key } => {
            let out = c
                .get::<InitiativeDto>(&format!("/initiatives/{key}"))
                .await?;
            if json || !matches!(out, crate::client::Outcome::Ok(_)) {
                finish(out, json, crate::render::initiative)
            } else {
                // Advisory and best effort: a failed findings read (an older server, a dropped connection) still shows the
                // initiative.
                let fs = match c
                    .get::<Vec<Finding>>(&format!("/initiatives/{key}/findings"))
                    .await
                {
                    Ok(crate::client::Outcome::Ok(fs)) => fs,
                    _ => vec![],
                };
                // Every decision under it, best effort like the findings.
                let ds = match c
                    .get::<Vec<RequiredDecisionDto>>(&format!("/decisions?initiative={key}"))
                    .await
                {
                    Ok(crate::client::Outcome::Ok(ds)) => ds,
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
                    let f = [
                        crate::render::findings(&fs),
                        crate::render::decisions_block(&ds),
                        crate::render::comments(&cs),
                    ]
                    .into_iter()
                    .filter(|b| !b.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                    let s = crate::render::initiative(v);
                    if f.is_empty() { s } else { format!("{s}\n{f}") }
                })
            }
        }
        Initiative::Findings { key } => finish(
            c.get::<Vec<Finding>>(&format!("/initiatives/{key}/findings"))
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
        Initiative::Ls => finish(
            c.get::<Vec<InitiativeDto>>("/initiatives").await?,
            json,
            |xs| {
                xs.iter()
                    .map(crate::render::initiative)
                    .collect::<Vec<_>>()
                    .join("\n")
            },
        ),
        Initiative::Edit {
            key,
            version,
            title,
            brief,
            brief_file,
            horizon,
            target_month,
            no_target_month,
            owner,
            no_owner,
            external_ref,
        } => {
            let edit = EditItem {
                version,
                title,
                brief: super::text_arg(brief, brief_file)?,
                horizon,
                target_month: if no_target_month {
                    Some(None)
                } else {
                    target_month.map(Some)
                },
                owner: if no_owner {
                    Some(None)
                } else {
                    owner.map(Some)
                },
                external_ref,
                ..Default::default()
            };
            finish(
                c.patch::<InitiativeDto>(
                    &format!("/initiatives/{key}"),
                    serde_json::to_value(edit)?,
                )
                .await?,
                json,
                crate::render::initiative,
            )
        }
        Initiative::Discovery {
            key,
            version,
            complete,
            reopen: _,
        } => finish(
            c.post::<InitiativeDto>(
                &format!("/initiatives/{key}/discovery"),
                serde_json::to_value(DiscoveryMark { version, complete })?,
            )
            .await?,
            json,
            crate::render::initiative,
        ),
    })
}
