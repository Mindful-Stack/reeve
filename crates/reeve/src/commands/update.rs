//! `reeve update` (kb/ADR-0030 R5, R7): replaces this executable with the build the server offers, verified against the
//! single SHA-256 line for this platform's file, over HTTPS, with no credentials and no redirects.
// The rules below are wired into the command in the next change.
#![allow(dead_code)]
use reeve_protocol::CLI_FILES;
use std::path::{Path, PathBuf};

/// This platform's file under `/cli/`, from the protocol's fixed list; `None` where the server ships none.
pub(crate) fn platform_file() -> Option<&'static str> {
    let want = if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "reeve-linux-x86_64"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "reeve-linux-aarch64"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "reeve-macos-aarch64"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "reeve-macos-x86_64"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "reeve-windows-x86_64.exe"
    } else {
        return None;
    };
    CLI_FILES.iter().copied().find(|n| *n == want)
}

/// The server url, if downloading from it is allowed: https, or http only to a loopback host; never with a user name or
/// password (reqwest would send them as `Authorization: Basic`). The messages never quote the url's credentials.
pub(crate) fn check_url(base: &str) -> Result<url::Url, String> {
    let u = url::Url::parse(base).map_err(|_| "the server url is not a url".to_string())?;
    if !u.username().is_empty() || u.password().is_some() {
        return Err(format!(
            "the server url must not carry a user name or password (host {})",
            u.host_str().unwrap_or("?")
        ));
    }
    let loopback = match u.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(a)) => a.is_loopback(),
        Some(url::Host::Ipv6(a)) => a.is_loopback(),
        None => false,
    };
    match u.scheme() {
        "https" => Ok(u),
        "http" if loopback => Ok(u),
        s => Err(format!(
            "refusing to download over {s}: the server url must be https (plain http only to a loopback address)"
        )),
    }
}

/// The SHA-256 (lowercase hex) on the one `SHA256SUMS` line for `file` (`<hash>  <name>` or `<hash> *<name>`).
pub(crate) fn expected_sum(sums: &str, file: &str) -> Result<String, String> {
    let lines: Vec<&str> = sums
        .lines()
        .filter(|l| {
            let mut p = l.split_whitespace();
            p.next().is_some()
                && p.next().map(|n| n.strip_prefix('*').unwrap_or(n)) == Some(file)
                && p.next().is_none()
        })
        .collect();
    let [line] = lines.as_slice() else {
        return Err(format!(
            "SHA256SUMS has {} lines for {file}, not one",
            lines.len()
        ));
    };
    let hash = line.split_whitespace().next().unwrap_or_default();
    if hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(hash.to_string())
    } else {
        Err(format!("SHA256SUMS's line for {file} holds no SHA-256"))
    }
}

/// The `n`th backup name for `exe`: `reeve.exe.old.<n>` beside it.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn backup_name(exe: &Path, n: u32) -> PathBuf {
    let mut s = exe.as_os_str().to_owned();
    s.push(format!(".old.{n}"));
    PathBuf::from(s)
}

/// Deletes earlier updates' backups that can be deleted; one a running process still uses stays (Windows refuses).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn remove_old_backups(exe: &Path) {
    let (Some(dir), Some(name)) = (exe.parent(), exe.file_name()) else {
        return;
    };
    let prefix = format!("{}.old.", name.to_string_lossy());
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n.strip_prefix(&prefix)
            .is_some_and(|rest| rest.parse::<u32>().is_ok())
        {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Moves `exe` aside to the first free backup name, then `new` into its place; if that fails, moves `exe` back, so a
/// failure leaves the original where it was. Returns the backup's path. `fault` is the test hook.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn swap_aside(
    exe: &Path,
    new: &Path,
    fault: impl Fn(&str) -> std::io::Result<()>,
) -> std::io::Result<PathBuf> {
    let old = (0..1000)
        .map(|n| backup_name(exe, n))
        .find(|p| !p.exists())
        .ok_or_else(|| {
            std::io::Error::other(
                "1000 backups beside the executable; delete the reeve.exe.old.* files",
            )
        })?;
    std::fs::rename(exe, &old)?;
    if let Err(e) = fault("after_move").and_then(|()| std::fs::rename(new, exe)) {
        return match std::fs::rename(&old, exe) {
            Ok(()) => Err(e),
            Err(back) => Err(std::io::Error::other(format!(
                "{e}; putting {} back also failed ({back}): the previous binary is {}",
                exe.display(),
                old.display()
            ))),
        };
    }
    Ok(old)
}

/// The debug-only fault hook: `REEVE_UPDATE_FAULT=<point>` fails that step. The read sits inside
/// `#[cfg(debug_assertions)]`, so release builds do not contain it.
pub(crate) fn fault(point: &str) -> std::io::Result<()> {
    #[cfg(debug_assertions)]
    if std::env::var("REEVE_UPDATE_FAULT").as_deref() == Ok(point) {
        return Err(std::io::Error::other(format!("injected fault at {point}")));
    }
    let _ = point;
    Ok(())
}

/// Puts the verified file `new` (beside `exe`, same file system) in place of the running executable.
///
/// Unix: one `rename(2)` over the target. POSIX: "If the link named by the new argument exists, it shall be removed
/// and old renamed to new. In this case, a link named new shall remain visible to other threads throughout the
/// renaming operation and refer either to the file referred to by new or old before the operation began" (The Open
/// Group Base Specifications Issue 7, `rename()`). The running process keeps its inode; `exe` is canonical, so a
/// symlink is never replaced by a file.
///
/// Windows: a running `.exe` can be renamed but not overwritten or deleted, and stays undeletable while anything runs
/// it. So: delete what earlier backups can be deleted, move the target aside under a fresh `reeve.exe.old.<n>`, move
/// the new file in, and move the target back if that fails (`swap_aside`). A TUI still running an old build keeps only
/// its own backup; the next update takes another name.
pub(crate) fn swap(exe: &Path, new: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        remove_old_backups(exe);
        swap_aside(exe, new, fault).map(|_| ())
    }
    #[cfg(not(windows))]
    {
        fault("after_move")?;
        std::fs::rename(new, exe)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_or_http_to_loopback_only() {
        for ok in [
            "https://reeve.example.com",
            "http://127.0.0.1:7474",
            "http://[::1]:1",
            "http://localhost:9",
        ] {
            assert!(check_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://reeve.example.com",
            "http://10.0.0.1",
            "ftp://reeve.example.com",
            "not a url",
        ] {
            assert!(check_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_url_with_credentials_is_refused() {
        // reqwest turns userinfo into `Authorization: Basic` (0.12.28 src/async_impl/request.rs 176–179, 581–601).
        for bad in [
            "https://user:secret@reeve.example.com",
            "http://user@127.0.0.1:1",
            "https://:pw@reeve.example.com",
        ] {
            let e = check_url(bad).unwrap_err();
            assert!(
                e.contains("user name or password") && !e.contains("secret") && !e.contains(":pw"),
                "{bad}: {e}"
            );
        }
    }

    #[test]
    fn the_single_checksum_line_for_a_file() {
        let h = "ab".repeat(32);
        let sums = format!(
            "{h}  reeve-linux-x86_64\n{}  reeve-linux-aarch64\n",
            "cd".repeat(32)
        );
        assert_eq!(expected_sum(&sums, "reeve-linux-x86_64").unwrap(), h);
        assert_eq!(
            expected_sum(&format!("{h} *reeve-linux-x86_64\n"), "reeve-linux-x86_64").unwrap(),
            h,
            "binary mode"
        );
        assert!(
            expected_sum(&sums, "reeve-macos-aarch64").is_err(),
            "no line"
        );
        assert!(
            expected_sum(
                &format!("{sums}{h}  reeve-linux-x86_64\n"),
                "reeve-linux-x86_64"
            )
            .is_err(),
            "two lines"
        );
        assert!(
            expected_sum("xyz  reeve-linux-x86_64\n", "reeve-linux-x86_64").is_err(),
            "not a sha-256"
        );
        assert!(
            expected_sum(
                &format!("{h}  reeve-linux-x86_64.bak\n"),
                "reeve-linux-x86_64"
            )
            .is_err(),
            "a longer name"
        );
    }

    /// One arm per shipped target, written out rather than derived, so a wrong arm fails on the runner of the target
    /// it breaks. PR B runs the lib tests on Linux x86_64 and Windows; the other targets' arms are checked end to end
    /// there by each leg's smoke, which runs that leg's own binary's `reeve update`.
    #[test]
    fn this_target_downloads_its_own_file() {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        assert_eq!(platform_file(), Some("reeve-linux-x86_64"));
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        assert_eq!(platform_file(), Some("reeve-linux-aarch64"));
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        assert_eq!(platform_file(), Some("reeve-macos-aarch64"));
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        assert_eq!(platform_file(), Some("reeve-macos-x86_64"));
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        assert_eq!(platform_file(), Some("reeve-windows-x86_64.exe"));
        #[cfg(not(any(
            all(
                target_os = "linux",
                any(target_arch = "x86_64", target_arch = "aarch64")
            ),
            all(
                target_os = "macos",
                any(target_arch = "x86_64", target_arch = "aarch64")
            ),
            all(target_os = "windows", target_arch = "x86_64"),
        )))]
        assert_eq!(platform_file(), None);
    }

    fn two_files(dir: &Path) -> (PathBuf, PathBuf) {
        let (exe, new) = (dir.join("reeve.exe"), dir.join("new"));
        std::fs::write(&exe, "old binary").unwrap();
        std::fs::write(&new, "new binary").unwrap();
        (exe, new)
    }

    #[test]
    fn a_failure_after_moving_aside_puts_the_original_back() {
        let d = tempfile::tempdir().unwrap();
        let (exe, new) = two_files(d.path());
        let err = swap_aside(&exe, &new, |p| {
            if p == "after_move" {
                Err(std::io::Error::other("boom"))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert!(err.to_string().contains("boom"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&exe).unwrap(),
            "old binary",
            "the original is back in place"
        );
        let left: Vec<_> = std::fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left.len(), 2, "nothing left aside: {left:?}");
    }

    #[test]
    fn each_swap_takes_a_fresh_backup_name() {
        let d = tempfile::tempdir().unwrap();
        let (exe, new) = two_files(d.path());
        // A backup a still-running process holds: the next swap must not need its name.
        let held = backup_name(&exe, 0);
        std::fs::write(&held, "held").unwrap();
        let used = swap_aside(&exe, &new, |_| Ok(())).unwrap();
        assert_ne!(used, held, "a fresh name");
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new binary");
        assert_eq!(std::fs::read_to_string(&used).unwrap(), "old binary");
        assert_eq!(
            std::fs::read_to_string(&held).unwrap(),
            "held",
            "the held backup is untouched"
        );
    }

    #[test]
    fn cleanup_removes_what_it_can_and_only_backups() {
        let d = tempfile::tempdir().unwrap();
        let exe = d.path().join("reeve.exe");
        for n in [
            backup_name(&exe, 0),
            backup_name(&exe, 7),
            d.path().join("reeve.exe.other"),
            exe.clone(),
        ] {
            std::fs::write(n, "x").unwrap();
        }
        remove_old_backups(&exe);
        let mut left: Vec<String> = std::fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["reeve.exe", "reeve.exe.other"]);
    }
}
