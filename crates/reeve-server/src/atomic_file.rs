//! Writes a file atomically with mode 0600: a temporary file beside the target, created exclusively, synced, then
//! renamed over the target. A copy of `crates/reeve/src/commands/export.rs`'s writer, kept separate because
//! `reeve-server` must not depend on the CLI crate and `reeve-protocol` holds wire types only; change both together.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Writes `bytes` to a temporary file beside `path` (mode 0600 on unix: the export holds every user's email), syncs it
/// and renames it over `path`. The temporary file is created exclusively (`O_CREAT|O_EXCL`), so a file or symlink
/// already at the name is never opened, followed or truncated: the next name (`<path>.<pid>.<n>.tmp`) is tried
/// instead. A failure removes only the temporary file this call created and leaves any previous `path` intact.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let (tmp, mut f) = create_temporary(path)?;
    let result = (|| {
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    drop(f);
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// The temporary files tried before giving up: `<path>.tmp`, then `<path>.<pid>.<n>.tmp`.
const TEMPORARY_ATTEMPTS: u32 = 16;

fn create_temporary(path: &Path) -> std::io::Result<(PathBuf, std::fs::File)> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    for attempt in 0..TEMPORARY_ATTEMPTS {
        let mut name = path.as_os_str().to_owned();
        if attempt == 0 {
            name.push(".tmp");
        } else {
            name.push(format!(".{}.{attempt}.tmp", std::process::id()));
        }
        let tmp = PathBuf::from(name);
        match opts.open(&tmp) {
            Ok(f) => return Ok((tmp, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!(
            "could not create a temporary file beside {}: {TEMPORARY_ATTEMPTS} names already exist",
            path.display()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_file_whole_and_leaves_no_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("e.json");
        std::fs::write(&path, "old").unwrap();
        write_atomically(&path, b"{\"a\":1}\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":1}\n");
        assert!(!dir.path().join("e.json.tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_failed_write_leaves_no_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        // the target is a directory, so the final rename fails after the temporary file was written
        let path = dir.path().join("e.json");
        std::fs::create_dir(&path).unwrap();
        assert!(write_atomically(&path, b"{}\n").is_err());
        assert!(!dir.path().join("e.json.tmp").exists());
        assert!(path.is_dir(), "the target is untouched");
        // a missing directory fails before anything is created
        let missing = dir.path().join("nope").join("e.json");
        assert!(write_atomically(&missing, b"{}\n").is_err());
        assert!(!missing.exists());
    }

    /// Catches a pre-existing (world-readable, attacker-held) `<out>.tmp` being reused as the export.
    #[cfg(unix)]
    #[test]
    fn a_pre_existing_temporary_file_is_never_reused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("e.json");
        let squatted = dir.path().join("e.json.tmp");
        std::fs::write(&squatted, "theirs").unwrap();
        std::fs::set_permissions(&squatted, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_atomically(&path, b"{}\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}\n");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "the export is a file this call created"
        );
        assert_eq!(
            std::fs::read_to_string(&squatted).unwrap(),
            "theirs",
            "left alone"
        );
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left.len(), 2, "no temporary of ours remains: {left:?}");
    }

    /// Catches the export written through a symlink planted at the temporary name.
    #[cfg(unix)]
    #[test]
    fn a_symlink_at_the_temporary_name_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("e.json");
        let target = dir.path().join("elsewhere");
        std::fs::write(&target, "untouched").unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("e.json.tmp")).unwrap();
        write_atomically(&path, b"{}\n").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "untouched");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}\n");
        assert!(
            !std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            std::fs::symlink_metadata(dir.path().join("e.json.tmp"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}
