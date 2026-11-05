use super::finish;
use crate::client::{Client, Outcome};
use clap::Subcommand;
use reeve_protocol::*;

#[derive(Subcommand)]
pub enum Goal {
    /// Add a goal to the draft sprint, or to the active one with --active (interactive tokens only). A repeated
    /// --idempotency-key returns the same goal.
    Add {
        /// Caller-chosen key that makes a retried add safe.
        #[arg(long)]
        idempotency_key: String,
        /// The outcome the team commits to.
        #[arg(long)]
        outcome: String,
        /// How the team will know: "we'll know when …".
        #[arg(long)]
        check: String,
        /// The goal's owner (email); defaults to you.
        #[arg(long)]
        owner: Option<String>,
        /// Free notes.
        #[arg(long)]
        notes: Option<String>,
        /// Add to the active sprint instead of the draft.
        #[arg(long)]
        active: bool,
    },
    /// Edit a goal (interactive tokens only); the --version must be the one you last read.
    Edit {
        /// Goal id.
        id: i64,
        /// The goal version you are editing (optimistic concurrency).
        #[arg(long)]
        version: i64,
        /// New outcome.
        #[arg(long)]
        outcome: Option<String>,
        /// New check.
        #[arg(long)]
        check: Option<String>,
        /// New owner (email).
        #[arg(long)]
        owner: Option<String>,
        /// New notes (replace the notes).
        #[arg(long)]
        notes: Option<String>,
    },
    /// Remove a draft sprint's goal and its links (interactive tokens only).
    Rm {
        /// Goal id.
        id: i64,
        /// The goal version you last read.
        #[arg(long)]
        version: i64,
    },
    /// Record your confidence in an active sprint's goal (interactive tokens only); it clears any suggestion.
    Confidence {
        /// Goal id.
        id: i64,
        /// How the goal is going.
        confidence: ConfidenceArg,
        /// Why.
        note: Option<String>,
    },
    /// Suggest a confidence for a person to record or discard (any token, agents included).
    Suggest {
        /// Goal id.
        id: i64,
        /// How the goal looks to be going.
        confidence: ConfidenceArg,
        /// Why (e.g. "2 in review").
        note: Option<String>,
    },
    /// Link initiatives, epics or stories to a goal (any token); an epic or initiative link covers their stories.
    Link {
        /// Goal id.
        id: i64,
        /// Item keys, e.g. ABC-12.
        #[arg(required = true)]
        keys: Vec<KeyRef>,
    },
    /// Unlink items from a goal (any token), one request per key, stopping at the first failure. Unlinking an item that
    /// is not linked succeeds and changes nothing.
    Unlink {
        /// Goal id.
        id: i64,
        /// Item keys.
        #[arg(required = true)]
        keys: Vec<KeyRef>,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum ConfidenceArg {
    #[value(name = "on_track")]
    OnTrack,
    #[value(name = "at_risk")]
    AtRisk,
    #[value(name = "off_track")]
    OffTrack,
}
impl From<ConfidenceArg> for Confidence {
    fn from(c: ConfidenceArg) -> Self {
        match c {
            ConfidenceArg::OnTrack => Confidence::OnTrack,
            ConfidenceArg::AtRisk => Confidence::AtRisk,
            ConfidenceArg::OffTrack => Confidence::OffTrack,
        }
    }
}

fn created(g: &CreatedGoalDto) -> String {
    let w = crate::render::warnings(&g.warnings);
    if w.is_empty() {
        crate::render::goal(&g.goal)
    } else {
        format!("{}\n{w}", crate::render::goal(&g.goal))
    }
}

pub async fn run(c: &Client, g: Goal, json: bool) -> anyhow::Result<i32> {
    Ok(match g {
        Goal::Add {
            idempotency_key,
            outcome,
            check,
            owner,
            notes,
            active,
        } => finish(
            c.post::<CreatedGoalDto>(
                "/goals",
                serde_json::to_value(CreateGoal {
                    idempotency_key,
                    outcome,
                    check,
                    owner,
                    notes,
                    active,
                })?,
            )
            .await?,
            json,
            created,
        ),
        Goal::Edit {
            id,
            version,
            outcome,
            check,
            owner,
            notes,
        } => finish(
            c.patch::<GoalDto>(
                &format!("/goals/{id}"),
                serde_json::to_value(EditGoal {
                    version,
                    outcome,
                    check,
                    owner,
                    notes,
                })?,
            )
            .await?,
            json,
            crate::render::goal,
        ),
        Goal::Rm { id, version } => finish(
            c.call::<Empty>(
                reqwest::Method::DELETE,
                &format!("/goals/{id}"),
                &[("version", version.to_string())],
                None,
            )
            .await?,
            json,
            |_| "removed".into(),
        ),
        Goal::Confidence {
            id,
            confidence,
            note,
        } => finish(
            c.post::<GoalDto>(
                &format!("/goals/{id}/confidence"),
                serde_json::to_value(RecordConfidence {
                    confidence: confidence.into(),
                    note,
                })?,
            )
            .await?,
            json,
            crate::render::goal,
        ),
        Goal::Suggest {
            id,
            confidence,
            note,
        } => finish(
            c.post::<GoalDto>(
                &format!("/goals/{id}/suggestion"),
                serde_json::to_value(SuggestConfidence {
                    confidence: confidence.into(),
                    note,
                })?,
            )
            .await?,
            json,
            crate::render::goal,
        ),
        Goal::Link { id, keys } => finish(
            c.post::<GoalDto>(
                &format!("/goals/{id}/links"),
                serde_json::to_value(LinkGoal { items: keys })?,
            )
            .await?,
            json,
            crate::render::goal,
        ),
        Goal::Unlink { id, keys } => {
            let mut last = None;
            let mut done: Vec<String> = vec![];
            for k in keys {
                match c
                    .delete::<GoalDto>(&format!("/goals/{id}/links/{k}"))
                    .await?
                {
                    Outcome::Ok(g) => {
                        done.push(k.to_string());
                        last = Some(Outcome::Ok(g))
                    }
                    failed => {
                        if !json && !done.is_empty() {
                            eprintln!("unlinked: {}; stopped at {k}", done.join(", "));
                        }
                        return Ok(finish(failed, json, crate::render::goal));
                    }
                }
            }
            finish(
                last.expect("clap requires at least one key"),
                json,
                crate::render::goal,
            )
        }
    })
}
