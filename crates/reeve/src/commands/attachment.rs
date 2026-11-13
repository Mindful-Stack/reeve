//! `reeve attachment get <ID> --dir DIR`: download one attachment through `GET /api/v1/attachments/{id}` (the
//! authenticated route, kb/ADR-0023 R5) and print the path of the local copy.
use super::finish;
use crate::client::{Client, Outcome};
use anyhow::Context;
use clap::Subcommand;
use reeve_protocol::ErrorCode;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Subcommand)]
pub enum Attachment {
    /// Download an attachment (a screenshot) and print the file's path. The file is a local copy outside the
    /// planner's access check; delete it when done.
    Get {
        /// The attachment's id (`attachments[].id` in `reeve story context --json`).
        id: Uuid,
        /// The directory to write `<id>.<png|jpg|webp>` into; created owner-only when missing.
        #[arg(long)]
        dir: PathBuf,
    },
}

#[derive(serde::Serialize)]
struct Written {
    path: PathBuf,
}

fn extension(content_type: &str) -> Option<&'static str> {
    match content_type {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

/// Creates `dir` (and its parents) owner-only when missing; an existing directory is used as it is.
fn ensure_dir(dir: &std::path::Path) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(dir)
}

pub async fn run(c: &Client, cmd: Attachment, json: bool) -> anyhow::Result<i32> {
    let Attachment::Get { id, dir } = cmd;
    let render = |w: &Written| w.path.display().to_string();
    let (content_type, bytes) = match c.get_image(&format!("/attachments/{id}")).await? {
        Ok(x) => x,
        Err(out) => return Ok(finish(out, json, |_| String::new())),
    };
    let Some(ext) = extension(&content_type) else {
        let out = Outcome::<Written>::Error {
            code: ErrorCode::Internal,
            message: format!("the planner answered {content_type:?}, not an image"),
            envelope: false,
            retry_after: None,
            current: None,
        };
        return Ok(finish(out, json, render));
    };
    ensure_dir(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(format!("{id}.{ext}"));
    crate::atomic_file::write_atomically(&path, &bytes)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(finish(Outcome::Ok(Written { path }), json, render))
}
