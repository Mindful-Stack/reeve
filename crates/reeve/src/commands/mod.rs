pub mod apply;
pub mod auth;
pub mod comment;
pub mod decision;
pub mod epic;
pub mod export;
pub mod goal;
pub mod import;
pub mod initiative;
pub mod search;
pub mod sprint;
pub mod story;
use crate::client::{Client, Outcome};
use clap::Subcommand;
use reeve_protocol::ErrorCode;
use serde::Serialize;

/// A text argument (a brief, a description) given inline, or read from a file whose contents are sent unchanged, so no
/// shell quoting stands between an agent's Markdown and the server; the path `-` reads stdin. Clap keeps the two
/// exclusive.
pub fn text_arg(
    inline: Option<String>,
    file: Option<std::path::PathBuf>,
) -> anyhow::Result<Option<String>> {
    use anyhow::Context;
    use std::io::Read;
    match file {
        None => Ok(inline),
        Some(p) if p.as_os_str() == "-" => {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .context("reading stdin")?;
            Ok(Some(s))
        }
        Some(p) => std::fs::read_to_string(&p)
            .with_context(|| format!("reading {}", p.display()))
            .map(Some),
    }
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Configure and inspect the server connection.
    #[command(subcommand)]
    Auth(auth::Auth),
    /// Initiatives: the largest planning unit (key prefix per instance).
    #[command(subcommand)]
    Initiative(initiative::Initiative),
    /// Epics group stories and belong to an initiative.
    #[command(subcommand)]
    Epic(epic::Epic),
    /// Stories, bugs and chores: create, list, move through delivery, read context.
    #[command(subcommand)]
    Story(story::Story),
    /// Required decisions that gate stories until a record is accepted.
    #[command(subcommand)]
    Decision(decision::Decision),
    /// Sprints: time boxes holding the team's goals; work counts in the sprint it was completed in.
    #[command(subcommand)]
    Sprint(sprint::Sprint),
    /// Team goals in a sprint: add, edit, link work, record or suggest confidence.
    #[command(subcommand)]
    Goal(goal::Goal),
    /// Comments on stories and decisions.
    #[command(subcommand)]
    Comment(comment::Comment),
    /// Create a plan of items, required decisions and blockers from a YAML or JSON file, in one request.
    Apply(apply::Apply),
    /// Find items by text in their title, brief, description or Jira reference.
    Search(search::Search),
    /// Write the whole instance as one JSON document (admins with an interactive token only).
    Export(export::Export),
    /// Import from another tracker (admins with an interactive token only). Rehearses unless --apply.
    #[command(subcommand)]
    Import(import::Import),
    /// The terminal board for your own work (uses your `reeve auth login`, never REEVE_TOKEN).
    Tui,
}

pub async fn run(cmd: Cmd, json: bool) -> anyhow::Result<i32> {
    match cmd {
        Cmd::Auth(a) => auth::run(a, json).await,
        // The TUI resolves its own credential (kb/ADR-0011 R7), so it branches before `resolve()`.
        Cmd::Tui if json => {
            eprintln!("error: --json is not supported by `reeve tui`");
            Ok(1)
        }
        Cmd::Tui => Ok(crate::tui::main().await),
        other => {
            let r = crate::config::resolve()?;
            let c = Client::new(r.url, r.token);
            match other {
                Cmd::Initiative(x) => initiative::run(&c, x, json).await,
                Cmd::Epic(x) => epic::run(&c, x, json).await,
                Cmd::Story(x) => story::run(&c, x, json).await,
                Cmd::Decision(x) => decision::run(&c, x, json).await,
                Cmd::Sprint(x) => sprint::run(&c, x, json).await,
                Cmd::Goal(x) => goal::run(&c, x, json).await,
                Cmd::Comment(x) => comment::run(&c, x, json).await,
                Cmd::Apply(x) => apply::run(&c, x, json).await,
                Cmd::Search(x) => search::run(&c, x, json).await,
                Cmd::Export(x) => export::run(&c, x, json).await,
                Cmd::Import(x) => import::run(&c, x, json).await,
                Cmd::Auth(_) | Cmd::Tui => unreachable!(),
            }
        }
    }
}

/// The caller's email, from `GET /me`. Any other answer comes back as the server's outcome (never `Ok`), for the caller
/// to `finish` with: a refused caller (`unauthenticated`, `forbidden`) then exits 2 like every other command.
pub async fn whoami(c: &Client) -> anyhow::Result<Result<String, Outcome<()>>> {
    Ok(match c.get::<reeve_protocol::Me>("/me").await? {
        Outcome::Ok(me) => Ok(me.email),
        Outcome::Refused {
            reasons,
            overridable,
        } => Err(Outcome::Refused {
            reasons,
            overridable,
        }),
        Outcome::Error {
            code,
            message,
            envelope,
            retry_after,
            current,
        } => Err(Outcome::Error {
            code,
            message,
            envelope,
            retry_after,
            current,
        }),
    })
}

/// Print an outcome and return the exit code. JSON mode prints the whole envelope; text mode renders.
pub fn finish<T: Serialize>(out: Outcome<T>, json: bool, render: impl Fn(&T) -> String) -> i32 {
    match &out {
        Outcome::Ok(v) => {
            if json {
                println!("{}", serde_json::json!({"outcome":"ok","value":v}))
            } else {
                println!("{}", render(v))
            }
        }
        Outcome::Refused {
            reasons,
            overridable,
        } => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({"outcome":"refused","reasons":reasons,"overridable":overridable})
                )
            } else {
                eprintln!("refused:\n{}", crate::render::reasons(reasons))
            }
        }
        Outcome::Error {
            code,
            message,
            retry_after,
            current,
            ..
        } => {
            if json {
                let mut v = serde_json::json!({"outcome":"error","code":code,"message":message});
                if let Some(s) = retry_after {
                    v["retry_after_secs"] = (*s).into();
                }
                // A conflict carries the version to re-read from, exactly as the server sent it (-1 included).
                if let Some(c) = current {
                    v["current"] = (*c).into();
                }
                println!("{v}")
            } else {
                eprintln!("error ({code:?}): {message}");
                if let (ErrorCode::Conflict, Some(c)) = (code, current)
                    && *c >= 0
                {
                    eprintln!("current version: {c}")
                }
            }
        }
    }
    out.exit_code()
}
