//! `reeve export [--out FILE]`: the whole instance as one JSON document (`GET /api/v1/export`), written as the document
//! itself, not the envelope. Admins with an interactive token only; any refusal exits through `Outcome::exit_code()`.
//!
//! The document is read as an untyped JSON value and written back unchanged (pretty-printed), so a CLI older than the
//! server neither drops fields it does not know nor fails on an enum value it does not know.
use super::finish;
use crate::client::{Client, Outcome};
use clap::Args;
use std::path::PathBuf;
use std::time::Duration;

/// A whole-instance export with its full activity log can outlast the client's default 30-second timeout.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Args)]
pub struct Export {
    /// Write the document to this file instead of standard output.
    #[arg(long)]
    out: Option<PathBuf>,
}

pub async fn run(c: &Client, cmd: Export, json: bool) -> anyhow::Result<i32> {
    let out = c
        .get_with_timeout::<serde_json::Value>("/export", EXPORT_TIMEOUT)
        .await?;
    let Outcome::Ok(doc) = &out else {
        return Ok(finish(out, json, |_| String::new()));
    };
    let text = format!("{}\n", serde_json::to_string_pretty(doc)?);
    match &cmd.out {
        Some(path) => {
            crate::atomic_file::write_atomically(path, text.as_bytes())?;
            let count = |k: &str| doc.get(k).and_then(|v| v.as_array()).map(Vec::len);
            match (count("stories"), count("activity")) {
                (Some(s), Some(a)) => eprintln!(
                    "exported {s} stories and {a} activity entries to {}",
                    path.display()
                ),
                _ => eprintln!("exported to {}", path.display()),
            }
        }
        None => print!("{text}"),
    }
    Ok(out.exit_code())
}
