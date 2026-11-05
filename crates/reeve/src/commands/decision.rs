use super::{finish, whoami};
use crate::client::{Client, Outcome};
use clap::Subcommand;
use reeve_protocol::*;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The CLI's spelling of the protocol's `Reversibility` (kebab-case); the protocol crate stays free of clap.
#[derive(Clone, Copy, clap::ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum ReversibilityArg {
    OneWay,
    TwoWay,
}

impl From<ReversibilityArg> for Reversibility {
    fn from(r: ReversibilityArg) -> Self {
        match r {
            ReversibilityArg::OneWay => Reversibility::OneWay,
            ReversibilityArg::TwoWay => Reversibility::TwoWay,
        }
    }
}

#[derive(Subcommand)]
pub enum Decision {
    /// Create a required decision on an initiative, epic or story.
    Require {
        /// Caller-chosen key that makes a retried create safe.
        #[arg(long)]
        idempotency_key: String,
        /// The initiative, epic or story the decision gates.
        #[arg(long)]
        on: KeyRef,
        /// What has to be decided.
        #[arg(long)]
        title: String,
        /// Who decides (email).
        #[arg(long)]
        decider: String,
        /// Optional due date (YYYY-MM-DD).
        #[arg(long)]
        due: Option<chrono::NaiveDate>,
        /// How hard the decision is to undo; a one-way decision is decided only by its accepted record.
        #[arg(long)]
        reversibility: Option<ReversibilityArg>,
    },
    /// Show one required decision.
    Show {
        /// Decision id.
        id: i64,
    },
    /// List required decisions.
    Ls {
        /// Only decisions on this item.
        #[arg(long, conflicts_with = "initiative")]
        on: Option<KeyRef>,
        /// Every decision under this initiative: on it, its epics and their stories.
        #[arg(long)]
        initiative: Option<KeyRef>,
        /// Only decisions you are the decider of (resolved with GET /me).
        #[arg(long)]
        mine: bool,
        /// The decision queue: grouped and ordered by urgency, with the stories each decision blocks.
        #[arg(long, conflicts_with = "mine")]
        queue: bool,
    },
    /// Link a record by repository name (from the server's configuration), path inside that repository, and record id. Runs an immediate check from `--household`; exits 1 if that check could neither store its result nor hold it for the scheduled check (a record a non-admin found accepted is held), or if the decision could not be read back afterwards.
    Link {
        /// Decision id.
        id: i64,
        /// Record id as written in its front matter, e.g. kb/ADR-0031.
        #[arg(long)]
        record: String,
        /// Repository name as configured on the server.
        #[arg(long)]
        repo: String,
        /// Path of the record file inside the repository.
        #[arg(long)]
        path: String,
        /// Url of the PR proposing the record, while it is not yet merged.
        #[arg(long)]
        pr: Option<String>,
        /// Household checkout directory; the repository is `<household>/<repo>`.
        #[arg(long, default_value = ".")]
        household: PathBuf,
    },
    /// Check every open required decision with a reference against its repository's default branch, from a household checkout. Exits 1 if any record could not be read or any result could not be stored; a result held for an admin's run is reported, not a failure, unless `--strict`; a post refused because the decision was closed or changed after the run listed it is skipped (the next run checks it), never a failure.
    Check {
        /// Household checkout directory; each repository is `<household>/<repo>`.
        #[arg(long, default_value = ".")]
        household: PathBuf,
        /// Check only this decision id.
        #[arg(long)]
        only: Option<i64>,
        /// Count a result held for an admin's run as a failure (exit 1). The scheduled run passes it: its token is an
        /// admin's, so a held result means the role was lost after the run started, and the run must fail, not pass.
        #[arg(long)]
        strict: bool,
    },
    /// Close a decision: --decided says what was decided (not for a one-way decision), --withdrawn why it is no longer
    /// needed.
    #[command(group(clap::ArgGroup::new("outcome").required(true).multiple(false)))]
    Close {
        /// Decision id.
        id: i64,
        /// What was decided. A one-way decision is decided by its accepted record, so the server refuses this for one.
        #[arg(long, group = "outcome")]
        decided: Option<String>,
        /// Why the decision is no longer needed.
        #[arg(long, group = "outcome")]
        withdrawn: Option<String>,
    },
    /// Edit an open decision's title, decider, due date or reversibility; the --version must be the one you last read. The record
    /// reference is changed with `link`.
    #[command(group(clap::ArgGroup::new("change").required(true).multiple(true).args(["title", "decider", "due", "no_due", "reversibility", "no_reversibility"])))]
    Edit {
        /// Decision id.
        id: i64,
        /// The decision version you are editing (optimistic concurrency).
        #[arg(long)]
        version: i64,
        /// New title.
        #[arg(long)]
        title: Option<String>,
        /// New decider (email).
        #[arg(long)]
        decider: Option<String>,
        /// New due date (YYYY-MM-DD).
        #[arg(long, conflicts_with = "no_due")]
        due: Option<chrono::NaiveDate>,
        /// Remove the due date.
        #[arg(long)]
        no_due: bool,
        /// New reversibility.
        #[arg(long, conflicts_with = "no_reversibility")]
        reversibility: Option<ReversibilityArg>,
        /// Remove the reversibility.
        #[arg(long)]
        no_reversibility: bool,
    },
}

pub async fn run(c: &Client, d: Decision, json: bool) -> anyhow::Result<i32> {
    Ok(match d {
        Decision::Require {
            idempotency_key,
            on,
            title,
            decider,
            due,
            reversibility,
        } => {
            if due.is_none() {
                eprintln!("no decide-by date; the queue will flag it");
            }
            // The protocol type, not a hand-built object: a renamed field fails to compile here, not at the server.
            let req = RequireDecision {
                idempotency_key,
                on,
                title,
                decider,
                due,
                reversibility: reversibility.map(Reversibility::from),
            };
            finish(
                c.post::<RequiredDecisionDto>("/decisions", serde_json::to_value(req)?)
                    .await?,
                json,
                crate::render::decision,
            )
        }
        Decision::Show { id } => finish(
            c.get::<RequiredDecisionDto>(&format!("/decisions/{id}"))
                .await?,
            json,
            crate::render::decision,
        ),
        Decision::Ls {
            on,
            initiative,
            mine,
            queue,
        } => {
            let base = if queue {
                "/decisions/queue"
            } else {
                "/decisions"
            };
            let path = match (on, initiative) {
                (Some(k), _) => format!("{base}?on={k}"),
                (None, Some(k)) => format!("{base}?initiative={k}"),
                (None, None) => base.to_string(),
            };
            if queue {
                return Ok(finish(
                    c.get::<DecisionQueueDto>(&path).await?,
                    json,
                    crate::render::queue,
                ));
            }
            let mut out = c.get::<Vec<RequiredDecisionDto>>(&path).await?;
            // The list's own failure is the answer; only a listed result needs the caller's email.
            if mine && let Outcome::Ok(ds) = &mut out {
                let me = match whoami(c).await? {
                    Ok(me) => normalise_email(&me),
                    Err(refused) => return Ok(finish(refused, json, |_| String::new())),
                };
                ds.retain(|d| normalise_email(&d.decider) == me);
            }
            finish(out, json, |ds| {
                ds.iter()
                    .map(crate::render::decision)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        }
        Decision::Link {
            id,
            record,
            repo,
            path,
            pr,
            household,
        } => {
            let linked = c
                .post::<RequiredDecisionDto>(
                    &format!("/decisions/{id}/link"),
                    serde_json::json!({"repo": repo, "path": path, "record_id": record, "pr_url": pr}),
                )
                .await?;
            let Outcome::Ok(d) = linked else {
                return Ok(finish(linked, json, crate::render::decision));
            };
            // The immediate check examines the reference the server stored (trimmed), not the raw arguments, so a
            // padded `--path` cannot link one path and check another.
            let Some(stored) = &d.record else {
                anyhow::bail!(
                    "decision {id}: the server returned no record reference after linking"
                );
            };
            let target = Target {
                id,
                repo: &stored.repo,
                path: &stored.path,
                record_id: &stored.record_id,
                version: d.version,
                has_pr: d.pr_url.is_some(),
            };
            // A repository the server does not check yet has no clone to read: the reference is kept, and the scheduled
            // check reads it once the repository is configured. A failed config read checks as before.
            let configured = match c.get::<InstanceConfigDto>("/config").await {
                Ok(Outcome::Ok(cfg)) => cfg.repos.contains(&stored.repo),
                _ => true,
            };
            let entry = if configured {
                check_one(c, &household, &mut Fetched::new(), &target).await?
            } else {
                CheckEntry {
                    id,
                    outcome: "repo_not_configured",
                    check: None,
                    error: None,
                    refused: None,
                }
            };
            report(id, &entry);
            // The link is stored and the check has run, so the answer is always the ok envelope with both. `value` is the
            // decision read back after the check, as before; when that read fails (a transport error or an error
            // envelope), it falls back to the link's own response and `refresh_error` says why.
            let (value, refresh_error) = match c
                .get::<RequiredDecisionDto>(&format!("/decisions/{id}"))
                .await
            {
                Ok(Outcome::Ok(now)) => (now, None),
                Ok(Outcome::Refused { reasons: rs, .. }) => (
                    d.clone(),
                    Some(format!("refused:\n{}", crate::render::reasons(&rs))),
                ),
                Ok(Outcome::Error { code, message, .. }) => {
                    (d.clone(), Some(format!("{code:?} {message}")))
                }
                Err(e) => (d.clone(), Some(format!("{e:#}"))),
            };
            if let Some(e) = &refresh_error {
                eprintln!(
                    "decision {id}: linked, but it could not be read back after the check: {e}"
                );
            }
            if json {
                let mut out =
                    serde_json::json!({"outcome":"ok","value":value,"immediate_check":entry});
                if let Some(e) = &refresh_error {
                    out["refresh_error"] = e.clone().into();
                }
                println!("{out}");
            } else {
                println!("{}", crate::render::decision(&value));
            }
            // 0 only when the check stored its result or held it for the scheduled run, and the decision shown is current.
            if refresh_error.is_none()
                && matches!(entry.outcome, "stored" | "held" | "repo_not_configured")
            {
                0
            } else {
                1
            }
        }
        Decision::Check {
            household,
            only,
            strict,
        } => {
            let ds = match c
                .get::<Vec<RequiredDecisionDto>>("/decisions?open_with_record=true")
                .await?
            {
                Outcome::Ok(ds) => ds,
                other => return Ok(finish(other, json, |_| String::new())),
            };
            let selected: Vec<&RequiredDecisionDto> = ds
                .iter()
                .filter(|d| d.record.is_some() && only.is_none_or(|o| o == d.id))
                .collect();
            if let Some(o) = only
                && selected.is_empty()
            {
                let message = format!("no open decision with a record has id {o}");
                if json {
                    println!(
                        "{}",
                        serde_json::json!({"outcome":"error","code":"not_found","message":message})
                    );
                } else {
                    eprintln!("error: {message}");
                }
                return Ok(1);
            }
            let mut fetched = Fetched::new();
            let mut results = vec![];
            for d in selected {
                let r = d.record.as_ref().expect("filtered on record");
                let target = Target {
                    id: d.id,
                    repo: &r.repo,
                    path: &r.path,
                    record_id: &r.record_id,
                    version: d.version,
                    has_pr: d.pr_url.is_some(),
                };
                let entry = check_one(c, &household, &mut fetched, &target).await?;
                let entry = skip_if_moved_on(c, d.version, entry).await;
                report(d.id, &entry);
                results.push(entry);
            }
            let count = |o: &str| results.iter().filter(|e| e.outcome == o).count();
            let (checked, held, skipped, unreadable, failed) = (
                count("stored"),
                count("held"),
                count("skipped"),
                count("unreadable"),
                count("failed"),
            );
            let value = serde_json::json!({"checked": checked, "held": held, "skipped": skipped, "unreadable": unreadable, "failed": failed, "results": results});
            // Held is not bad, except under `--strict`; skipped is never bad.
            let bad = unreadable + failed + if strict { held } else { 0 };
            if json {
                if bad == 0 {
                    println!("{}", serde_json::json!({"outcome":"ok","value":value}))
                } else {
                    println!(
                        "{}",
                        serde_json::json!({"outcome":"error","code":"internal","message":format!("{bad} of {} checks failed", results.len()),"value":value})
                    )
                }
            } else {
                println!(
                    "checked {checked} decision(s), {held} held, {skipped} skipped, {unreadable} unreadable, {failed} failed"
                )
            }
            if bad == 0 { 0 } else { 1 }
        }
        Decision::Close {
            id,
            decided,
            withdrawn,
        } => {
            // clap's `outcome` group makes exactly one of the two present.
            let close = match (decided, withdrawn) {
                (Some(reason), None) => CloseDecision {
                    kind: ClosureKind::Decided,
                    reason,
                },
                (None, Some(reason)) => CloseDecision {
                    kind: ClosureKind::Withdrawn,
                    reason,
                },
                _ => unreachable!("clap requires exactly one of --decided and --withdrawn"),
            };
            finish(
                c.post::<RequiredDecisionDto>(
                    &format!("/decisions/{id}/close"),
                    serde_json::to_value(close)?,
                )
                .await?,
                json,
                crate::render::decision,
            )
        }
        Decision::Edit {
            id,
            version,
            title,
            decider,
            due,
            no_due,
            reversibility,
            no_reversibility,
        } => {
            // The protocol type, not a hand-built object: a renamed field fails to compile here, not at the server.
            let edit = EditDecision {
                version,
                title,
                decider,
                due: if no_due { Some(None) } else { due.map(Some) },
                reversibility: if no_reversibility {
                    Some(None)
                } else {
                    reversibility.map(|r| Some(r.into()))
                },
            };
            finish(
                c.patch::<RequiredDecisionDto>(
                    &format!("/decisions/{id}"),
                    serde_json::to_value(edit)?,
                )
                .await?,
                json,
                crate::render::decision,
            )
        }
    })
}

/// The repository directory is `<household>/<repo>`, or `<household>` itself when the household is that repository: its
/// basename is the repo and it is a repository itself (it has a `.git` directory, or a `.git` file as a linked
/// worktree has). The household is canonicalised first, so `.` (or `sub/..`) run from inside the repository's checkout
/// is recognised as it. A plain directory that only shares the name (the scheduled check's household) is never taken.
fn repo_dir(household: &Path, repo: &str) -> PathBuf {
    let household = household
        .canonicalize()
        .unwrap_or_else(|_| household.to_path_buf());
    if household.file_name().is_some_and(|n| n == repo) && household.join(".git").exists() {
        household
    } else {
        household.join(repo)
    }
}

/// Printed (on stderr) for a check whose `accepted` the server refused: only an admin's check stores it (kb/ADR-0003
/// R3), so the decision stays unchecked until the scheduled run.
const HELD: &str =
    "held: accepted on the default branch; stored by the next scheduled check run by an admin";

/// The stderr line for an entry that did not simply store: its error, or the held line.
fn report(id: i64, e: &CheckEntry) {
    if let Some(err) = &e.error {
        eprintln!("decision {id}: {err}");
    }
    if e.outcome == "held" {
        eprintln!("decision {id}: {HELD}");
    }
    if e.outcome == "repo_not_configured" {
        eprintln!(
            "decision {id}: linked; its repository is not configured on the server, so the record is not checked yet (the scheduled check reads it once the repository is configured)"
        );
    }
}

/// Repository directories already fetched in this run, with the fetch error if there was one.
type Fetched = HashMap<PathBuf, Option<String>>;

struct Target<'a> {
    id: i64,
    repo: &'a str,
    path: &'a str,
    record_id: &'a str,
    version: i64,
    has_pr: bool,
}

#[derive(Serialize)]
struct CheckEntry {
    id: i64,
    /// `stored`; `held` (the record is accepted but the server stores that only from an admin: nothing stored, the
    /// scheduled run stores it); `unreadable` (nothing stored: the record could not be read, see `check_one`); `failed`
    /// (the server did not store the result); `repo_not_configured` (`decision link` only: the record is in a repository
    /// the server does not check yet, so no check ran); `skipped` (`decision check` only: the server refused the post because the
    /// decision changed after the run listed it, so the next run checks its new state).
    outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    check: Option<CheckResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// The code of the server's own refusal of the post, if that is why the entry failed.
    #[serde(skip)]
    refused: Option<ErrorCode>,
}

/// A repository name from the server is used as one directory under the household: it must be a single normal path
/// component that git cannot read as an option, or the checker would run git somewhere the household does not own.
fn valid_repo_name(repo: &str) -> Result<(), String> {
    if repo.is_empty()
        || repo == "."
        || repo == ".."
        || repo.contains('/')
        || repo.contains('\\')
        || repo.starts_with('-')
    {
        return Err(format!(
            "repository name `{repo}` is not a single directory name"
        ));
    }
    Ok(())
}

/// The result for a record that was read. A front-matter `id` that differs from the linked record id (or is missing) means
/// the path points at some other record: not accepted, with status `id mismatch`, whatever that record's own status is.
fn record_result(r: crate::git::RecordStatus, record_id: &str) -> CheckResult {
    if r.id.as_deref() != Some(record_id.trim()) {
        return CheckResult::NotAccepted {
            status: "id mismatch".into(),
            superseded_by: None,
            commit: r.commit,
            // Another record's front matter is not a fact about the linked record.
            reversibility: None,
        };
    }
    if r.status == "accepted" {
        CheckResult::Accepted {
            commit: r.commit,
            reversibility: r.reversibility,
        }
    } else {
        CheckResult::NotAccepted {
            status: r.status,
            superseded_by: r.superseded_by,
            commit: r.commit,
            reversibility: r.reversibility,
        }
    }
}

/// Maps what the checker read to what it posts. `not_found` is posted only when `origin/HEAD` resolved to a commit
/// and the file is absent from it (kb/ADR-0004 R4); a file that exists at that commit but whose front matter cannot be
/// parsed is a fact about the record and is posted as not accepted with status `unparsable front matter`, so a broken
/// edit on the default branch unsatisfies the decision instead of leaving it accepted. Anything that says more about
/// this checkout than about the record (an invalid repository name, a missing clone, a failed fetch, an unresolvable
/// `origin/HEAD`) posts nothing: the decision keeps its last result and the entry is reported `unreadable`, which
/// makes the run exit 1.
fn check_from_read(
    read: Result<crate::git::RecordStatus, crate::git::RecordReadError>,
    record_id: &str,
    has_pr: bool,
) -> Result<CheckResult, String> {
    use crate::git::RecordReadError;
    Ok(match read {
        Ok(r) => record_result(r, record_id),
        Err(RecordReadError::NotFound) if has_pr => CheckResult::ProposedPrOpen,
        Err(RecordReadError::NotFound) => CheckResult::NotFound,
        Err(RecordReadError::Unparsable { commit, .. }) => CheckResult::NotAccepted {
            status: "unparsable front matter".into(),
            superseded_by: None,
            commit,
            reversibility: None,
        },
        Err(e) => return Err(e.to_string()),
    })
}

/// The run itself.
async fn check_one(
    c: &Client,
    household: &Path,
    fetched: &mut Fetched,
    t: &Target<'_>,
) -> anyhow::Result<CheckEntry> {
    use crate::git::{self, RecordReadError};
    let unreadable = |e: String| CheckEntry {
        id: t.id,
        outcome: "unreadable",
        check: None,
        error: Some(format!("{e}; nothing recorded")),
        refused: None,
    };
    if let Err(e) = valid_repo_name(t.repo) {
        return Ok(unreadable(e));
    }
    let dir = repo_dir(household, t.repo);
    if !fetched.contains_key(&dir) {
        let r = git::fetch(&dir).await.err().map(|e| e.to_string());
        fetched.insert(dir.clone(), r);
    }
    let read = match &fetched[&dir] {
        Some(e) => Err(RecordReadError::Git(e.clone())),
        None => git::read_record(&dir, t.path).await,
    };
    let check = match check_from_read(read, t.record_id, t.has_pr) {
        Ok(c) => c,
        Err(e) => return Ok(unreadable(e)),
    };
    let posted = match serde_json::to_value(RecordCheckResult {
        record_id: t.record_id.to_string(),
        version: t.version,
        check: check.clone(),
    }) {
        Ok(body) => {
            c.post::<RequiredDecisionDto>(&format!("/decisions/{}/check", t.id), body)
                .await
        }
        Err(e) => Err(e.into()),
    };
    Ok(posted_entry(t.id, check, posted))
}

/// What the run records for a check that was read and then posted. A transport or decoding error on the post is a
/// `failed` entry carrying the error text, not an abort: the remaining decisions are still checked and the summary
/// counts it, so the run reports and exits 1.
fn posted_entry(
    id: i64,
    check: CheckResult,
    posted: anyhow::Result<Outcome<RequiredDecisionDto>>,
) -> CheckEntry {
    let refused = match &posted {
        Ok(Outcome::Error {
            code,
            envelope: true,
            ..
        }) => Some(*code),
        _ => None,
    };
    let (outcome, error) = match posted {
        Ok(Outcome::Ok(_)) => ("stored", None),
        Ok(Outcome::Refused { reasons: rs, .. }) => (
            "failed",
            Some(format!("refused:\n{}", crate::render::reasons(&rs))),
        ),
        // Property: held is exactly a forbidden answer to the satisfying result; anything else refused is failed.
        Ok(Outcome::Error {
            code: ErrorCode::Forbidden,
            envelope: true,
            ..
        }) if matches!(check, CheckResult::Accepted { .. }) => ("held", None),
        Ok(Outcome::Error { code, message, .. }) => ("failed", Some(format!("{code:?} {message}"))),
        Err(e) => ("failed", Some(format!("{e:#}"))),
    };
    CheckEntry {
        id,
        outcome,
        check: Some(check),
        error,
        refused,
    }
}

/// A post the server refused because the decision moved on after the run listed it is `skipped`: a 409 (stale) when
/// the decision, read again, is closed or at another version (someone linked, relinked or closed it, or another check
/// stored a result), and a 400 `validation` only when it is closed ("decision is closed"; a relink resets the check and
/// keeps a reference, so a close is the only change that makes a valid post invalid). It matches the error code, not the
/// status. The refusal is right and the next run checks the new state, so it is not a failure, even under `--strict`.
/// Anything else stays `failed`, and so does a decision that cannot be read again.
async fn skip_if_moved_on(c: &Client, listed_version: i64, mut e: CheckEntry) -> CheckEntry {
    if e.outcome != "failed"
        || !matches!(e.refused, Some(ErrorCode::Conflict | ErrorCode::Validation))
    {
        return e;
    }
    let Ok(Outcome::Ok(now)) = c
        .get::<RequiredDecisionDto>(&format!("/decisions/{}", e.id))
        .await
    else {
        return e;
    };
    let why = if now.closed.is_some() {
        "it was closed".to_string()
    } else if e.refused == Some(ErrorCode::Conflict) && now.version != listed_version {
        format!(
            "it changed (version {listed_version} is now {})",
            now.version
        )
    } else {
        return e;
    };
    e.outcome = "skipped";
    e.error = Some(format!(
        "{}; skipped: {why} during the run, so the next run checks it",
        e.error.unwrap_or_default()
    ));
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::RecordReadError;

    /// Distinguishes a post that dies in transport (aborting the batch with `?`) from one recorded as `failed`.
    #[test]
    fn a_transport_error_on_post_is_a_failed_entry_not_an_abort() {
        let e = posted_entry(
            7,
            CheckResult::NotFound,
            Err(anyhow::anyhow!("connection reset by peer")),
        );
        assert_eq!(e.outcome, "failed");
        assert_eq!(e.id, 7);
        assert!(e.error.unwrap().contains("connection reset"));
        assert!(e.check.is_some());
    }

    /// Distinguishes a checker that fails open on a broken record (posting nothing, leaving it accepted) from one that
    /// posts not-accepted; a fetch failure must still post nothing.
    #[test]
    fn unparsable_front_matter_is_posted_as_not_accepted_but_checkout_errors_are_not() {
        let unparsable = check_from_read(
            Err(RecordReadError::Unparsable {
                path: "docs/adr/0001.md".into(),
                commit: "abc".into(),
            }),
            "kb/ADR-0001",
            true,
        );
        assert_eq!(
            unparsable,
            Ok(CheckResult::NotAccepted {
                status: "unparsable front matter".into(),
                superseded_by: None,
                commit: "abc".into(),
                reversibility: None,
            })
        );
        assert!(
            check_from_read(
                Err(RecordReadError::Git("fetch failed".into())),
                "kb/ADR-0001",
                true
            )
            .is_err()
        );
        assert_eq!(
            check_from_read(Err(RecordReadError::NotFound), "kb/ADR-0001", true),
            Ok(CheckResult::ProposedPrOpen)
        );
        assert_eq!(
            check_from_read(Err(RecordReadError::NotFound), "kb/ADR-0001", false),
            Ok(CheckResult::NotFound)
        );
    }

    #[test]
    fn a_record_whose_id_differs_from_the_link_is_an_id_mismatch() {
        let rec = |id: Option<&str>, status: &str| crate::git::RecordStatus {
            id: id.map(String::from),
            status: status.into(),
            superseded_by: None,
            reversibility: Some(reeve_protocol::Reversibility::OneWay),
            commit: "c".into(),
        };
        assert_eq!(
            record_result(rec(Some("kb/ADR-0031"), "accepted"), "kb/ADR-0031"),
            CheckResult::Accepted {
                commit: "c".into(),
                reversibility: Some(reeve_protocol::Reversibility::OneWay)
            }
        );
        for other in [Some("kb/ADR-0007"), None] {
            assert_eq!(
                record_result(rec(other, "accepted"), "kb/ADR-0031"),
                CheckResult::NotAccepted {
                    status: "id mismatch".into(),
                    superseded_by: None,
                    commit: "c".into(),
                    reversibility: None,
                },
                "{other:?}"
            );
        }
        assert!(matches!(
            record_result(rec(Some("kb/ADR-0031"), "proposed"), "kb/ADR-0031"),
            CheckResult::NotAccepted { status, .. } if status == "proposed"
        ));
    }

    /// R2: the record's own reversibility rides along on the result, accepted or not.
    #[test]
    fn the_result_carries_the_records_reversibility() {
        use reeve_protocol::Reversibility::OneWay;
        let rec = |status: &str| crate::git::RecordStatus {
            id: Some("kb/ADR-0031".into()),
            status: status.into(),
            superseded_by: None,
            reversibility: Some(OneWay),
            commit: "c".into(),
        };
        assert_eq!(
            record_result(rec("accepted"), "kb/ADR-0031"),
            CheckResult::Accepted {
                commit: "c".into(),
                reversibility: Some(OneWay)
            }
        );
        assert_eq!(
            record_result(rec("proposed"), "kb/ADR-0031"),
            CheckResult::NotAccepted {
                status: "proposed".into(),
                superseded_by: None,
                commit: "c".into(),
                reversibility: Some(OneWay)
            }
        );
    }

    #[test]
    fn mine_compares_normalised_emails() {
        assert_eq!(
            normalise_email(" Dan@Example.com "),
            normalise_email("dan@example.com")
        );
    }

    #[test]
    fn repository_names_must_be_one_directory() {
        for ok in ["knowledge-base", "orion", "a.b"] {
            assert!(valid_repo_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", ".", "..", "../x", "a/b", "/abs", "a\\b", "-c"] {
            assert!(valid_repo_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn repo_dir_recognises_a_household_that_is_the_repository_after_canonicalising() {
        let root = tempfile::tempdir().unwrap();
        let kb = root.path().join("knowledge-base");
        std::fs::create_dir_all(kb.join("knowledge")).unwrap();
        std::fs::create_dir_all(kb.join(".git")).unwrap();
        let kb = kb.canonicalize().unwrap();
        // `<kb>/knowledge/..` has no basename until it is resolved; it is the repository itself.
        assert_eq!(
            repo_dir(&kb.join("knowledge").join(".."), "knowledge-base"),
            kb
        );
        assert_eq!(repo_dir(&kb, "knowledge-base"), kb);
        // A household above the repository joins the name.
        assert_eq!(
            repo_dir(root.path(), "knowledge-base"),
            root.path().canonicalize().unwrap().join("knowledge-base")
        );
    }

    /// `held` is exactly a `forbidden` answer to `accepted`: the same answer to any other result, and any other answer to
    /// `accepted`, is still `failed` (a CLI that mapped every 403 to held would hide a refused token).
    #[test]
    fn only_a_forbidden_answer_to_accepted_is_held() {
        let answer = |code: ErrorCode| -> anyhow::Result<Outcome<RequiredDecisionDto>> {
            Ok(Outcome::Error {
                code,
                message: "m".into(),
                envelope: true,
                retry_after: None,
                current: None,
            })
        };
        let accepted = CheckResult::Accepted {
            commit: "c".into(),
            reversibility: None,
        };
        let held = posted_entry(1, accepted.clone(), answer(ErrorCode::Forbidden));
        // A 403 whose body the server did not write (a proxy's page) is not the server's refusal: failed.
        let proxy = posted_entry(
            1,
            accepted.clone(),
            Ok(Outcome::Error {
                code: ErrorCode::Forbidden,
                message: "HTTP 403".into(),
                envelope: false,
                retry_after: None,
                current: None,
            }),
        );
        assert_eq!(proxy.outcome, "failed");
        assert_eq!((held.outcome, held.error.is_none()), ("held", true));
        for other in [
            CheckResult::NotFound,
            CheckResult::ProposedPrOpen,
            CheckResult::NotAccepted {
                status: "proposed".into(),
                superseded_by: None,
                commit: "c".into(),
                reversibility: None,
            },
        ] {
            let e = posted_entry(1, other.clone(), answer(ErrorCode::Forbidden));
            assert_eq!(e.outcome, "failed", "{other:?}");
        }
        for code in [
            ErrorCode::Unauthenticated,
            ErrorCode::Conflict,
            ErrorCode::Validation,
        ] {
            let e = posted_entry(1, accepted.clone(), answer(code));
            assert_eq!(e.outcome, "failed", "{code:?}");
        }
    }

    /// A household that merely shares the repository's name (the scheduled check's volume when a repository is called
    /// `household`) is not the repository: the clone inside it is. Only a directory that is itself a repository (a
    /// `.git` directory, or a `.git` file in a linked worktree) is taken as the repository.
    #[test]
    fn a_household_named_like_the_repository_is_not_the_repository() {
        let root = tempfile::tempdir().unwrap();
        let hh = root.path().join("household");
        std::fs::create_dir_all(hh.join("household")).unwrap();
        let hh = hh.canonicalize().unwrap();
        assert_eq!(repo_dir(&hh, "household"), hh.join("household"));
        std::fs::write(hh.join(".git"), "gitdir: /elsewhere\n").unwrap();
        assert_eq!(
            repo_dir(&hh, "household"),
            hh,
            "a linked worktree of the repository"
        );
    }
}
