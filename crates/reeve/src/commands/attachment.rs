//! `reeve attachment get <ID> [--dir DIR]`: download one attachment through `GET /api/v1/attachments/{id}` (the
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
        /// The directory to write `<id>.<png|jpg|webp>` into. Default: <temp dir>/reeve-attachments.
        #[arg(long)]
        dir: Option<PathBuf>,
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

/// `<temp>/reeve-attachments`, created owner-only: the images are private screenshots.
fn default_dir() -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join("reeve-attachments");
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(&dir)?;
    // A shared temp directory lets another user pre-create the name; refuse anything but a private directory.
    let meta = std::fs::symlink_metadata(&dir)?;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o7777
    };
    #[cfg(not(unix))]
    let mode = 0;
    let shared = mode & 0o077 != 0;
    // A directory another user owns (reachable by root, or pre-created with a loose parent) is not ours either:
    // a file created inside is owned by this process's user, so compare the directory's owner with that.
    #[cfg(unix)]
    let foreign = {
        use std::os::unix::fs::MetadataExt;
        let probe = dir.join(format!(".owner-probe.{}", std::process::id()));
        let f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)?;
        let own = f.metadata()?.uid();
        drop(f);
        let _ = std::fs::remove_file(&probe);
        meta.uid() != own
    };
    #[cfg(not(unix))]
    let foreign = false;
    if !meta.is_dir() || shared || foreign {
        return Err(std::io::Error::other(format!(
            "{} is not a private directory of yours (mode {mode:o}); remove it or pass --dir",
            dir.display()
        )));
    }
    Ok(dir)
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
    let dir = match dir {
        Some(d) => d,
        None => default_dir()?,
    };
    let path = dir.join(format!("{id}.{ext}"));
    crate::atomic_file::write_atomically(&path, &bytes)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(finish(Outcome::Ok(Written { path }), json, render))
}
