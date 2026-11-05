use super::finish;
use crate::client::{Client, Outcome};
use clap::Subcommand;
use reeve_protocol::*;
use serde::Serialize;

#[derive(Subcommand)]
pub enum Sprint {
    /// Show a sprint with its goals and counts. Without an id: the active sprint, else the draft. A closed sprint whose
    /// counts changed since its close also shows them at close.
    Show {
        /// Sprint id.
        id: Option<i64>,
    },
    /// List the active sprint, the draft and the closed sprints (newest first) with their goals and verdicts.
    Ls,
    /// Open the draft sprint (interactive tokens only); this creates the next draft.
    Open {
        /// Last day of the sprint (YYYY-MM-DD); defaults to one configured sprint length after today.
        #[arg(long)]
        ends: Option<chrono::NaiveDate>,
    },
    /// Close the active sprint with a verdict for every goal (interactive tokens only).
    Close {
        /// A goal's verdict: `<goal id>=<met|partly_met|missed>[:<note>]`; repeat for every goal.
        #[arg(long = "verdict", value_parser = parse_verdict)]
        verdicts: Vec<VerdictArg>,
    },
}

/// One `--verdict` argument: the goal id, the verdict and an optional note.
#[derive(Debug, Clone, PartialEq)]
pub struct VerdictArg(pub i64, pub Verdict, pub Option<String>);

/// `12=partly_met:one booking` → `(12, PartlyMet, Some("one booking"))`; the note runs to the end and may hold colons.
fn parse_verdict(s: &str) -> Result<VerdictArg, String> {
    let usage = || format!("`{s}`: expected <goal id>=<met|partly_met|missed>[:<note>]");
    let (id, rest) = s.split_once('=').ok_or_else(usage)?;
    let id: i64 = id.trim().parse().map_err(|_| usage())?;
    let (word, note) = match rest.split_once(':') {
        Some((w, n)) => (w, Some(n.to_string()).filter(|n| !n.is_empty())),
        None => (rest, None),
    };
    let verdict = match word.trim() {
        "met" => Verdict::Met,
        "partly_met" => Verdict::PartlyMet,
        "missed" => Verdict::Missed,
        _ => return Err(usage()),
    };
    Ok(VerdictArg(id, verdict, note))
}

/// `sprint show`'s value: the view and, for a closed sprint only, its counts at close (from the `closed` entry, always,
/// so a reader has blocked-by-decisions at close) and whether any window number changed since (kb/ADR-0014 R6).
#[derive(Serialize)]
struct Shown {
    #[serde(flatten)]
    view: SprintViewDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    at_close: Option<SprintCountsDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    changed_since_close: Option<bool>,
}

/// An outcome of another type carried through unchanged (a failed read ends the command with its own answer).
fn carry<A, B>(o: Outcome<A>) -> Outcome<B> {
    match o {
        Outcome::Ok(_) => unreachable!("only failures are carried"),
        Outcome::Refused {
            reasons,
            overridable,
        } => Outcome::Refused {
            reasons,
            overridable,
        },
        Outcome::Error {
            code,
            message,
            envelope,
            retry_after,
            current,
        } => Outcome::Error {
            code,
            message,
            envelope,
            retry_after,
            current,
        },
    }
}

/// A local answer for a command that found nothing to act on; exits 1.
fn missing<T>(message: &str) -> Outcome<T> {
    Outcome::Error {
        code: ErrorCode::NotFound,
        message: message.into(),
        envelope: false,
        retry_after: None,
        current: None,
    }
}

fn bad_entry<T>(message: String) -> Outcome<T> {
    Outcome::Error {
        code: ErrorCode::Internal,
        message,
        envelope: false,
        retry_after: None,
        current: None,
    }
}

/// The counts at close, from the sprint's `closed` activity entry (the CLI composes this; planning never reads activity).
/// A closed sprint always has one; a missing entry, or one without counts, is an error.
async fn counts_at_close(c: &Client, id: i64) -> anyhow::Result<Outcome<SprintCountsDto>> {
    let q = [
        ("entity_type", "sprint".to_string()),
        ("entity_key", id.to_string()),
        ("action", "closed".to_string()),
        ("limit", "1".to_string()),
    ];
    Ok(
        match c
            .get_query::<Vec<ActivityEntryDto>>("/activity", &q)
            .await?
        {
            Outcome::Ok(entries) => match entries.into_iter().next() {
                None => bad_entry(format!(
                    "sprint {id} is closed but has no `closed` activity entry"
                )),
                Some(e) => match e.payload.get("counts").cloned().map(serde_json::from_value) {
                    Some(Ok(counts)) => Outcome::Ok(counts),
                    _ => bad_entry(format!(
                        "the `closed` activity entry of sprint {id} carries no readable counts"
                    )),
                },
            },
            other => carry(other),
        },
    )
}

async fn show(c: &Client, id: Option<i64>) -> anyhow::Result<Outcome<Shown>> {
    let view = match id {
        Some(id) => c.get::<SprintViewDto>(&format!("/sprints/{id}")).await?,
        None => match c.get::<SprintsDto>("/sprints").await? {
            Outcome::Ok(SprintsDto {
                active: Some(a), ..
            }) => Outcome::Ok(a),
            Outcome::Ok(SprintsDto { draft: Some(d), .. }) => {
                c.get::<SprintViewDto>(&format!("/sprints/{}", d.id))
                    .await?
            }
            Outcome::Ok(_) => {
                return Ok(missing(
                    "no sprint yet: `reeve goal add` starts the first draft",
                ));
            }
            other => return Ok(carry(other)),
        },
    };
    let view = match view {
        Outcome::Ok(v) => v,
        other => return Ok(carry(other)),
    };
    let at_close = if view.sprint.state == SprintState::Closed {
        match counts_at_close(c, view.sprint.id).await? {
            Outcome::Ok(a) => Some(a),
            other => return Ok(carry(other)),
        }
    } else {
        None
    };
    let changed_since_close = at_close
        .as_ref()
        .map(|a| crate::render::counts_differ(&view.counts, a));
    Ok(Outcome::Ok(Shown {
        view,
        at_close,
        changed_since_close,
    }))
}

/// Open and close read the sprint's version themselves, so a version the caller never saw is no use to them: in text
/// mode a conflict says to run the command again instead of naming the current version. `--json` keeps the envelope.
fn retry_on_conflict<T>(out: Outcome<T>, json: bool) -> Outcome<T> {
    match out {
        Outcome::Error {
            code: ErrorCode::Conflict,
            envelope,
            retry_after,
            ..
        } if !json => Outcome::Error {
            code: ErrorCode::Conflict,
            message: "the sprint changed; run the command again".into(),
            envelope,
            retry_after,
            current: None,
        },
        other => other,
    }
}

pub async fn run(c: &Client, s: Sprint, json: bool) -> anyhow::Result<i32> {
    Ok(match s {
        Sprint::Show { id } => finish(show(c, id).await?, json, |s| {
            crate::render::sprint_view(&s.view, s.at_close.as_ref())
        }),
        Sprint::Ls => finish(
            c.get::<SprintsDto>("/sprints").await?,
            json,
            crate::render::sprint_list,
        ),
        Sprint::Open { ends } => {
            let draft = match c.get::<SprintsDto>("/sprints").await? {
                Outcome::Ok(SprintsDto { draft: Some(d), .. }) => d,
                Outcome::Ok(_) => {
                    return Ok(finish(
                        missing::<SprintDto>(
                            "no draft sprint: `reeve goal add` starts the first draft",
                        ),
                        json,
                        crate::render::sprint,
                    ));
                }
                other => {
                    return Ok(finish(
                        carry::<_, SprintDto>(other),
                        json,
                        crate::render::sprint,
                    ));
                }
            };
            let out = c
                .post::<SprintDto>(
                    &format!("/sprints/{}/open", draft.id),
                    serde_json::to_value(OpenSprint {
                        version: draft.version,
                        ends_on: ends,
                    })?,
                )
                .await?;
            finish(retry_on_conflict(out, json), json, crate::render::sprint)
        }
        Sprint::Close { verdicts } => {
            let active = match c.get::<SprintsDto>("/sprints").await? {
                Outcome::Ok(SprintsDto {
                    active: Some(a), ..
                }) => a.sprint,
                Outcome::Ok(_) => {
                    return Ok(finish(
                        missing::<SprintDto>("no active sprint to close"),
                        json,
                        crate::render::sprint,
                    ));
                }
                other => {
                    return Ok(finish(
                        carry::<_, SprintDto>(other),
                        json,
                        crate::render::sprint,
                    ));
                }
            };
            let out = c
                .post::<SprintDto>(
                    &format!("/sprints/{}/close", active.id),
                    serde_json::to_value(CloseSprint {
                        version: active.version,
                        verdicts: verdicts
                            .into_iter()
                            .map(|VerdictArg(goal_id, verdict, note)| VerdictInput {
                                goal_id,
                                verdict,
                                note,
                            })
                            .collect(),
                    })?,
                )
                .await?;
            finish(retry_on_conflict(out, json), json, crate::render::sprint)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verdict_argument_parses_its_goal_verdict_and_note() {
        assert_eq!(
            parse_verdict("12=partly_met:one booking"),
            Ok(VerdictArg(
                12,
                Verdict::PartlyMet,
                Some("one booking".into())
            ))
        );
        assert_eq!(
            parse_verdict("3=met"),
            Ok(VerdictArg(3, Verdict::Met, None))
        );
        assert_eq!(
            parse_verdict("3=missed:a: b"),
            Ok(VerdictArg(3, Verdict::Missed, Some("a: b".into())))
        );
        for bad in ["12=maybe", "12", "x=met", "=met", "12=Met"] {
            assert!(parse_verdict(bad).is_err(), "{bad}");
        }
    }
}
