//! Reads a decision record's front matter from a repository's default branch, never from the working tree.
use reeve_protocol::Reversibility;
use std::{
    path::Path,
    process::{Output, Stdio},
    time::Duration,
};
use tokio::process::Command;

const GIT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordStatus {
    /// The front matter's `id`, compared by the checker with the id the decision links.
    pub id: Option<String>,
    pub status: String,
    pub superseded_by: Option<String>,
    /// The record's own `reversibility` (kb/ADR-0013 R2); absent when missing or unrecognised.
    pub reversibility: Option<Reversibility>,
    pub commit: String,
}

/// What the check reads from a record's front matter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontMatter {
    pub id: Option<String>,
    pub status: String,
    pub superseded_by: Option<String>,
    /// The record's own `reversibility` (kb/ADR-0013 R2); absent when missing or unrecognised.
    pub reversibility: Option<Reversibility>,
}

#[derive(Debug, thiserror::Error)]
pub enum RecordReadError {
    #[error("file not found on the default branch")]
    NotFound,
    #[error("git failed: {0}")]
    Git(String),
    #[error("no front matter or no status in {0}")]
    FrontMatter(String),
    /// The file exists at `commit` but its front matter cannot be parsed: a fact about the record, so the checker
    /// records it as not accepted instead of leaving the decision's last result in place.
    #[error("unparsable front matter in {path} at {commit}")]
    Unparsable { path: String, commit: String },
}

/// The git command `run` starts: never prompting, with a stable locale, killed when dropped.
fn command(repo: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        // The token is the planner's: not git's, nor that of the ssh, credential helper or lazy fetch git starts.
        .env_remove("REEVE_TOKEN")
        .kill_on_drop(true);
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    cmd
}

/// Runs git (`command`) with a hard timeout (the child is killed on expiry).
async fn run(repo: &Path, args: &[&str]) -> Result<Output, RecordReadError> {
    let mut cmd = command(repo, args);
    match tokio::time::timeout(GIT_TIMEOUT, cmd.output()).await {
        Err(_) => Err(RecordReadError::Git(format!(
            "git {} timed out after {}s",
            args.join(" "),
            GIT_TIMEOUT.as_secs()
        ))),
        Ok(Err(e)) => Err(RecordReadError::Git(e.to_string())),
        Ok(Ok(out)) => Ok(out),
    }
}

async fn git(repo: &Path, args: &[&str]) -> Result<String, RecordReadError> {
    let out = run(repo, args).await?;
    if !out.status.success() {
        return Err(RecordReadError::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Update `origin`. Call once per repository per run, before `read_record`. The directory must exist and be the top of
/// its own clone; a plain directory inside some other repository would otherwise answer for that repository.
pub async fn fetch(repo: &Path) -> Result<(), RecordReadError> {
    if !repo.is_dir() {
        return Err(RecordReadError::Git(format!(
            "{} does not exist; clone the repository into the household",
            repo.display()
        )));
    }
    let top = git(repo, &["rev-parse", "--show-toplevel"]).await?;
    let same = match (Path::new(top.trim()).canonicalize(), repo.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !same {
        return Err(RecordReadError::Git(format!(
            "{} is not the top of a git clone",
            repo.display()
        )));
    }
    git(repo, &["fetch", "-q", "origin"]).await?;
    // `fetch` never moves `refs/remotes/origin/HEAD`; follow the remote's advertised default branch so a renamed
    // default branch is read (kb/ADR-0004.R4). A failure here (for example a single-branch clone that never fetched
    // the new default branch) would leave a resolvable but stale ref, so it is an error: the decision is reported
    // unreadable rather than checked against the wrong branch.
    git(repo, &["remote", "set-head", "origin", "-a"])
        .await
        .map_err(|e| {
            RecordReadError::Git(format!("could not follow the remote default branch: {e}"))
        })?;
    Ok(())
}

/// Reads the record at `origin/HEAD` (the remote's default branch) as of the last fetch: one commit is resolved, the
/// path's existence is read from that commit's tree, and the file is read from that same commit.
pub async fn read_record(repo: &Path, path: &str) -> Result<RecordStatus, RecordReadError> {
    let commit = git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "-q",
            "refs/remotes/origin/HEAD^{commit}",
        ],
    )
    .await
    .map_err(|_| {
        RecordReadError::Git(format!(
            "origin/HEAD is not set; run `git -C {} remote set-head origin -a`",
            repo.display()
        ))
    })?
    .trim()
    .to_string();
    // `ls-tree` names its entry in normal form, so the comparison below, and both reads, use the normal form too.
    let path = &normal_path(path);
    if path.is_empty() {
        // The tree's root is not a file.
        return Err(RecordReadError::NotFound);
    }
    let spec = format!("{commit}:{path}");
    // Existence comes from the commit's tree: `ls-tree` lists the entry at the path without reading the blob. In a
    // partial clone the blob may still be on origin; `cat-file -e` would fetch it and, with origin unreachable, fail
    // exactly like an absent file, which is posted as "not found". Here only `git show` needs the blob, and its failure
    // is a git error: unreadable, nothing posted.
    // Only an empty listing is "no such entry". Any failure (a tree on the path that cannot be read, in a damaged or a
    // treeless clone, or a path outside the tree) is a git error, because only an absent file may be reported not found
    // (kb/ADR-0004.R4). The commit is hex, so it cannot be read as an option, and `--` ends the options before the path.
    let listing = git(repo, &["ls-tree", "-z", &commit, "--", path]).await?;
    if !is_blob_entry(&listing, path) {
        return Err(RecordReadError::NotFound);
    }
    let content = git(repo, &["show", &spec]).await?;
    let fm = parse_front_matter(&content, path).map_err(|_| RecordReadError::Unparsable {
        path: path.to_string(),
        commit: commit.clone(),
    })?;
    Ok(RecordStatus {
        id: fm.id,
        status: fm.status,
        superseded_by: fm.superseded_by,
        reversibility: fm.reversibility,
        commit,
    })
}

/// `path` without empty and `.` components: `./a//b/./c` is `a/b/c`. Nothing else changes: the name is still matched
/// literally, and a `..` component stays (git refuses a path outside the tree, which is reported unreadable).
fn normal_path(path: &str) -> String {
    path.split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .collect::<Vec<_>>()
        .join("/")
}

/// Whether `ls-tree -z` output lists `path` itself as a file. The path is matched exactly, so a pattern-like path (or
/// a directory, whose entry is a tree) does not count as the record being there.
fn is_blob_entry(listing: &str, path: &str) -> bool {
    listing.split('\0').any(|entry| {
        entry
            .split_once('\t')
            .is_some_and(|(meta, name)| name == path && meta.split(' ').nth(1) == Some("blob"))
    })
}

pub fn parse_front_matter(content: &str, path: &str) -> Result<FrontMatter, RecordReadError> {
    let bad = || RecordReadError::FrontMatter(path.into());
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut lines = content.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Err(bad());
    }
    let mut fm = String::new();
    let mut closed = false;
    for l in lines {
        if l.trim() == "---" {
            closed = true;
            break;
        }
        fm.push_str(l);
        fm.push('\n');
    }
    if !closed {
        return Err(bad());
    }
    let yaml: serde_yaml::Value = serde_yaml::from_str(&fm).map_err(|_| bad())?;
    let status = yaml
        .get("status")
        .and_then(|v| v.as_str())
        .ok_or_else(bad)?
        .to_string();
    let text = |k: &str| {
        yaml.get(k)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let reversibility = match text("reversibility").as_deref() {
        Some("one-way") => Some(Reversibility::OneWay),
        Some("two-way") => Some(Reversibility::TwoWay),
        _ => None,
    };
    Ok(FrontMatter {
        id: text("id"),
        status,
        superseded_by: text("superseded_by"),
        reversibility,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// R2: hyphenated values map to the planner's; missing or unknown is absent and the parse still succeeds (status is
    /// read as before). Catches a strict parse that turns an odd reversibility into "unparsable front matter".
    #[test]
    fn front_matter_reversibility_is_read_leniently() {
        let doc = |rv: &str| format!("---\nid: kb/ADR-0001\nstatus: accepted\n{rv}\n---\n# x\n");
        let rv = |s: &str| parse_front_matter(&doc(s), "a.md").unwrap().reversibility;
        assert_eq!(
            rv("reversibility: one-way      # one-way | two-way"),
            Some(Reversibility::OneWay)
        );
        assert_eq!(rv("reversibility: two-way"), Some(Reversibility::TwoWay));
        assert_eq!(rv("reversibility: sideways"), None);
        assert_eq!(rv("reversibility: [one-way]"), None);
        assert_eq!(rv("confidence: high"), None);
        assert_eq!(
            parse_front_matter(&doc("reversibility: sideways"), "a.md")
                .unwrap()
                .status,
            "accepted"
        );
    }

    fn sh(dir: &Path, args: &[&str]) {
        let st = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(st.success(), "git {:?}", args);
    }

    /// A working repository whose `origin` is a bare repository nested in the same temp dir.
    struct Fixture {
        root: tempfile::TempDir,
        work: PathBuf,
        bare: PathBuf,
    }

    fn repo_with(path: &str, content: &str) -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        let bare = root.path().join("origin.git");
        std::fs::create_dir_all(&work).unwrap();
        sh(&work, &["init", "-q", "-b", "main"]);
        sh(&work, &["config", "user.email", "t@example.com"]);
        sh(&work, &["config", "user.name", "t"]);
        std::fs::create_dir_all(work.join(Path::new(path).parent().unwrap())).unwrap();
        std::fs::write(work.join(path), content).unwrap();
        sh(&work, &["add", "."]);
        sh(&work, &["commit", "-q", "-m", "add record"]);
        sh(
            &work,
            &[
                "clone",
                "-q",
                "--bare",
                work.to_str().unwrap(),
                bare.to_str().unwrap(),
            ],
        );
        sh(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);
        sh(&work, &["fetch", "-q", "origin"]);
        sh(&work, &["remote", "set-head", "origin", "-a"]);
        Fixture { root, work, bare }
    }

    async fn read(f: &Fixture, path: &str) -> Result<RecordStatus, RecordReadError> {
        fetch(&f.work).await.unwrap();
        read_record(&f.work, path).await
    }

    const P: &str = "knowledge/adrs/0031-queue.md";

    #[tokio::test]
    async fn reads_status_and_superseded_by_from_the_default_branch() {
        let f = repo_with(
            P,
            "---\nid: kb/ADR-0031\nstatus: accepted\nsuperseded_by:\n---\n# x\n",
        );
        let r = read(&f, P).await.unwrap();
        assert_eq!(r.id.as_deref(), Some("kb/ADR-0031"));
        assert_eq!(r.status, "accepted");
        assert_eq!(r.superseded_by, None);
        assert_eq!(r.commit.len(), 40);
        // A working-tree change that is not on origin's default branch must not be seen.
        std::fs::write(f.work.join(P), "---\nstatus: rejected\n---\n").unwrap();
        assert_eq!(read(&f, P).await.unwrap().status, "accepted");
        // Nor a committed but unpushed change.
        sh(&f.work, &["commit", "-q", "-am", "reject locally"]);
        assert_eq!(read(&f, P).await.unwrap().status, "accepted");
    }

    #[tokio::test]
    async fn a_push_to_origin_is_seen_after_fetch() {
        let f = repo_with(P, "---\nstatus: accepted\n---\n");
        let other = f.root.path().join("other");
        sh(
            f.root.path(),
            &[
                "clone",
                "-q",
                f.bare.to_str().unwrap(),
                other.to_str().unwrap(),
            ],
        );
        sh(&other, &["config", "user.email", "t@example.com"]);
        sh(&other, &["config", "user.name", "t"]);
        std::fs::write(
            other.join(P),
            "---\nstatus: superseded\nsuperseded_by: kb/ADR-0040\n---\n",
        )
        .unwrap();
        sh(&other, &["commit", "-q", "-am", "supersede"]);
        sh(&other, &["push", "-q", "origin", "HEAD"]);
        // Without a fetch the old state is still what origin/HEAD points at.
        assert_eq!(read_record(&f.work, P).await.unwrap().status, "accepted");
        let r = read(&f, P).await.unwrap();
        assert_eq!(r.status, "superseded");
        assert_eq!(r.superseded_by.as_deref(), Some("kb/ADR-0040"));
    }

    #[tokio::test]
    async fn missing_file_is_not_found_and_superseded_carries_target() {
        let f = repo_with(
            "knowledge/adrs/0002-old.md",
            "---\nstatus: superseded\nsuperseded_by: kb/ADR-0016\n---\n",
        );
        assert!(matches!(
            read(&f, "knowledge/adrs/none.md").await,
            Err(RecordReadError::NotFound)
        ));
        assert_eq!(
            read(&f, "knowledge/adrs/0002-old.md")
                .await
                .unwrap()
                .superseded_by
                .as_deref(),
            Some("kb/ADR-0016")
        );
    }

    /// The scheduled check clones with `--filter=blob:none --no-checkout`, so a record's blob arrives only when it is read.
    /// If origin is unreachable then, existence must still come from the commit's tree: the file is there, so the read
    /// is a git error (reported unreadable, nothing posted), never NotFound (posted as "not found", which would unsatisfy
    /// an accepted decision because of a network blip).
    #[tokio::test]
    async fn a_partial_clone_that_cannot_reach_origin_is_a_git_error_not_not_found() {
        let f = repo_with(P, "---\nid: kb/ADR-0031\nstatus: proposed\n---\n");
        sh(&f.bare, &["config", "uploadpack.allowFilter", "true"]);
        let clone = f.root.path().join("household").join("kb");
        let url = format!("file://{}", f.bare.display());
        sh(
            f.root.path(),
            &[
                "clone",
                "-q",
                "--filter=blob:none",
                "--no-checkout",
                &url,
                clone.to_str().unwrap(),
            ],
        );
        // The clone really is partial: a server that ignored the filter would make this test pass vacuously.
        let promisor = std::process::Command::new("git")
            .arg("-C")
            .arg(&clone)
            .args(["config", "--get", "remote.origin.promisor"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&promisor.stdout).trim(), "true");
        // A record reaches origin after the clone; the fetch brings its commit and tree, not its blob.
        const Q: &str = "knowledge/adrs/0032-cache.md";
        std::fs::write(
            f.work.join(Q),
            "---\nid: kb/ADR-0032\nstatus: accepted\n---\n",
        )
        .unwrap();
        sh(&f.work, &["add", "."]);
        sh(&f.work, &["commit", "-q", "-m", "cache"]);
        sh(&f.work, &["push", "-q", "origin", "main"]);
        fetch(&clone).await.unwrap();
        let away = f.root.path().join("origin-away.git");
        std::fs::rename(&f.bare, &away).unwrap();
        let offline = read_record(&clone, Q).await;
        assert!(
            matches!(&offline, Err(RecordReadError::Git(m)) if m.contains("promisor remote")),
            "{offline:?}"
        );
        // A path absent from the tree is still NotFound offline: existence needs no blob.
        let absent = read_record(&clone, "knowledge/adrs/9999-none.md").await;
        assert!(
            matches!(absent, Err(RecordReadError::NotFound)),
            "{absent:?}"
        );
        std::fs::rename(&away, &f.bare).unwrap();
        let online = read_record(&clone, Q).await.unwrap();
        assert_eq!(
            (online.id.as_deref(), online.status.as_str()),
            (Some("kb/ADR-0032"), "accepted")
        );
    }

    /// A tree on the path that cannot be read (a damaged clone, or a treeless clone whose origin is unreachable) is a
    /// git error: only an absent file may be reported not found (kb/ADR-0004 R4). `rev-parse --verify -q` answered
    /// exit 1 for both, so a lost tree object posted "not found" and unsatisfied an accepted decision.
    #[tokio::test]
    async fn a_missing_tree_object_is_a_git_error_not_not_found() {
        let f = repo_with(P, "---\nstatus: accepted\n---\n");
        fetch(&f.work).await.unwrap();
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&f.work)
            .args(["rev-parse", "refs/remotes/origin/HEAD:knowledge/adrs"])
            .output()
            .unwrap();
        let tree = String::from_utf8(out.stdout).unwrap().trim().to_string();
        assert_eq!(tree.len(), 40, "{tree}");
        let loose = f
            .work
            .join(".git/objects")
            .join(&tree[..2])
            .join(&tree[2..]);
        assert!(loose.exists(), "the fixture's objects are loose");
        std::fs::remove_file(&loose).unwrap();
        let r = read_record(&f.work, P).await;
        assert!(matches!(r, Err(RecordReadError::Git(_))), "{r:?}");
        // An absent file next to it still needs that tree, so it cannot be told apart: a git error too.
        let r = read_record(&f.work, "knowledge/adrs/none.md").await;
        assert!(matches!(r, Err(RecordReadError::Git(_))), "{r:?}");
        // One outside the damaged tree is still read as not found.
        let r = read_record(&f.work, "none.md").await;
        assert!(matches!(r, Err(RecordReadError::NotFound)), "{r:?}");
    }

    /// A directory at the path is not the record: not found, not a tree listing parsed as front matter.
    #[tokio::test]
    async fn a_directory_at_the_path_is_not_found() {
        let f = repo_with(P, "---\nstatus: accepted\n---\n");
        let r = read(&f, "knowledge/adrs").await;
        assert!(matches!(r, Err(RecordReadError::NotFound)), "{r:?}");
    }

    /// Existence is an exact path, not a pattern: a path that only matches as a pathspec is not found.
    #[tokio::test]
    async fn the_existence_probe_takes_the_path_literally() {
        let f = repo_with(P, "---\nstatus: accepted\n---\n");
        for p in ["knowledge/adrs/0031-*.md", "knowledge/adrs/0031-queue.m?"] {
            let r = read(&f, p).await;
            assert!(matches!(r, Err(RecordReadError::NotFound)), "{p}: {r:?}");
        }
    }

    /// `ls-tree` prints the entry's name in normal form, so a path linked with `./`, `//` or `/./` (which the server
    /// accepted before it normalised paths) is the same record, not an absent one.
    #[tokio::test]
    async fn a_path_with_dot_or_empty_components_is_the_same_record() {
        let f = repo_with(P, "---\nid: kb/ADR-0031\nstatus: accepted\n---\n");
        for p in [
            "./knowledge/adrs/0031-queue.md",
            "knowledge//adrs/0031-queue.md",
            "knowledge/./adrs/0031-queue.md",
            ".//knowledge/adrs/./0031-queue.md",
        ] {
            let r = read(&f, p).await;
            assert!(
                matches!(&r, Ok(s) if s.status == "accepted" && s.id.as_deref() == Some("kb/ADR-0031")),
                "{p}: {r:?}"
            );
        }
        // Normalising does not make a directory, or the tree's root, a file.
        for p in ["./knowledge/adrs/", ".", "./"] {
            let r = read(&f, p).await;
            assert!(matches!(r, Err(RecordReadError::NotFound)), "{p}: {r:?}");
        }
    }

    #[tokio::test]
    async fn a_path_outside_the_tree_is_a_git_error_not_not_found() {
        let f = repo_with(P, "---\nstatus: proposed\n---\n");
        fetch(&f.work).await.unwrap();
        let r = read_record(&f.work, "../x").await;
        assert!(matches!(r, Err(RecordReadError::Git(_))), "{r:?}");
    }

    #[tokio::test]
    async fn fetch_refuses_a_missing_directory_and_a_directory_inside_a_clone() {
        let f = repo_with(P, "---\nstatus: accepted\n---\n");
        let missing = fetch(&f.root.path().join("absent")).await.unwrap_err();
        assert!(missing.to_string().contains("does not exist"), "{missing}");
        let inner = fetch(&f.work.join("knowledge")).await.unwrap_err();
        assert!(inner.to_string().contains("not the top"), "{inner}");
    }

    #[tokio::test]
    async fn fetch_follows_a_changed_default_branch() {
        let f = repo_with(P, "---\nstatus: accepted\n---\n");
        assert_eq!(read(&f, P).await.unwrap().status, "accepted");
        let other = f.root.path().join("other");
        sh(
            f.root.path(),
            &[
                "clone",
                "-q",
                f.bare.to_str().unwrap(),
                other.to_str().unwrap(),
            ],
        );
        sh(&other, &["config", "user.email", "t@example.com"]);
        sh(&other, &["config", "user.name", "t"]);
        sh(&other, &["checkout", "-q", "-b", "trunk"]);
        std::fs::write(
            other.join(P),
            "---\nstatus: superseded\nsuperseded_by: kb/ADR-0040\n---\n",
        )
        .unwrap();
        sh(&other, &["commit", "-q", "-am", "supersede on trunk"]);
        sh(&other, &["push", "-q", "origin", "trunk"]);
        sh(&f.bare, &["symbolic-ref", "HEAD", "refs/heads/trunk"]);
        let r = read(&f, P).await.unwrap();
        assert_eq!(r.status, "superseded");
    }

    #[tokio::test]
    async fn unset_origin_head_gives_the_actionable_hint() {
        let f = repo_with(P, "---\nstatus: accepted\n---\n");
        sh(&f.work, &["remote", "set-head", "origin", "-d"]);
        let e = read_record(&f.work, P).await.unwrap_err().to_string();
        assert!(e.contains("remote set-head origin -a"), "{e}");
    }

    /// The CLI's token is for the planner only: git, and the ssh, credential helpers and lazy fetches it starts, never
    /// get it in their environment.
    #[test]
    fn git_never_inherits_the_token() {
        let cmd = command(Path::new("."), &["status"]);
        let token = cmd
            .as_std()
            .get_envs()
            .find(|(k, _)| *k == "REEVE_TOKEN")
            .map(|(_, v)| v);
        assert_eq!(
            token,
            Some(None),
            "REEVE_TOKEN must be removed, not inherited"
        );
    }

    #[test]
    fn front_matter_edge_cases() {
        let ok = parse_front_matter("\u{feff}---\nstatus: accepted\n---\nbody", "p").unwrap();
        assert_eq!(ok.status, "accepted");
        assert_eq!(ok.id, None);
        let crlf = parse_front_matter(
            "---\r\nstatus: superseded\r\nsuperseded_by: kb/ADR-9\r\n---\r\n",
            "p",
        )
        .unwrap();
        assert_eq!(
            crlf,
            FrontMatter {
                id: None,
                status: "superseded".into(),
                superseded_by: Some("kb/ADR-9".into()),
                reversibility: None
            }
        );
        assert!(matches!(
            parse_front_matter("---\nstatus: accepted\n# no closing fence\n", "p"),
            Err(RecordReadError::FrontMatter(_))
        ));
        assert!(parse_front_matter("# no front matter\n", "p").is_err());
    }
}
