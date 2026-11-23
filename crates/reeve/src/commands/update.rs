//! `reeve update` (kb/ADR-0030 R5, R7): replaces this executable with the build the server offers, verified against the
//! single SHA-256 line for this platform's file, over HTTPS, with no credentials and no redirects.
use crate::cli_version::{self, short};
use reeve_protocol::{CLI_FILES, ErrorCode, is_cli_version};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(clap::Args)]
pub struct Update {}

/// A known way `reeve update` fails: an envelope code, a stable `reason` when an agent acts on it, and the message.
/// The `reason` values (`not_distributed`, `checksum_mismatch`, `cannot_replace`) are a contract agents match on
/// (kb/ADR-0030 R6): change them only through a record.
struct Failure {
    code: ErrorCode,
    reason: Option<&'static str>,
    message: String,
}

fn fail(code: ErrorCode, message: impl Into<String>) -> Failure {
    Failure {
        code,
        reason: None,
        message: message.into(),
    }
}

fn fail_because(code: ErrorCode, reason: &'static str, message: impl Into<String>) -> Failure {
    Failure {
        code,
        reason: Some(reason),
        message: message.into(),
    }
}

/// The largest answer read for each kind of file: the two text files and the binary. A server (or anything in front of
/// it) that sends more is not one `reeve update` should keep reading from.
const TEXT_CAP: usize = 64 * 1024;
const BINARY_CAP: usize = 256 * 1024 * 1024;

/// One downloaded file and its content type.
struct Got {
    bytes: Vec<u8>,
    content_type: String,
}

/// Fetches `/cli/<name>`, reading at most `cap` bytes (a longer `Content-Length` is refused before reading, and
/// reading stops at the cap when the length is absent or wrong). Over the cap is `not_distributed` for VERSION (what
/// answers there is not a Reeve server's file) and a plain `internal` error otherwise. VERSION's content type is checked
/// before its body is read: an older server answers with its web page.
async fn get(
    http: &reqwest::Client,
    base: &url::Url,
    name: &str,
    cap: usize,
) -> Result<Got, Failure> {
    let url = format!("{}/cli/{name}", base.as_str().trim_end_matches('/'));
    let mut res = http
        .get(&url)
        .send()
        .await
        .map_err(|e| fail(ErrorCode::Internal, format!("GET {url}: {e}")))?;
    let status = res.status();
    let header = |n: &str| {
        res.headers()
            .get(n)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    if status.is_redirection() {
        let to = header("location").unwrap_or_else(|| "?".into());
        return Err(fail(
            ErrorCode::Validation,
            format!(
                "the server redirected /cli/{name} to {to}; reeve update follows no redirects (is the server url {} right?)",
                base.as_str().trim_end_matches('/')
            ),
        ));
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(fail(
            ErrorCode::RateLimited,
            "the server's proxy is rate-limiting /cli/; wait a minute and run reeve update again",
        ));
    }
    if status == reqwest::StatusCode::NOT_FOUND && name == "VERSION" {
        return Err(not_distributed(base));
    }
    if !status.is_success() {
        return Err(fail(
            ErrorCode::Internal,
            format!("GET /cli/{name}: HTTP {status}"),
        ));
    }
    let content_type = header("content-type").unwrap_or_default();
    let too_big = || {
        if name == "VERSION" {
            not_distributed(base)
        } else {
            fail(
                ErrorCode::Internal,
                format!(
                    "GET /cli/{name}: the answer is larger than {cap} bytes; nothing was replaced"
                ),
            )
        }
    };
    if name == "VERSION" && !content_type.starts_with("text/plain") {
        return Err(not_distributed(base));
    }
    if res.content_length().is_some_and(|n| n > cap as u64) {
        return Err(too_big());
    }
    let mut bytes = Vec::with_capacity(res.content_length().unwrap_or(0).min(cap as u64) as usize);
    while let Some(chunk) = res
        .chunk()
        .await
        .map_err(|e| fail(ErrorCode::Internal, format!("GET {url}: {e}")))?
    {
        if bytes.len() + chunk.len() > cap {
            return Err(too_big());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Got {
        bytes,
        content_type,
    })
}

fn not_distributed(base: &url::Url) -> Failure {
    fail_because(
        ErrorCode::NotFound,
        "not_distributed",
        format!(
            "this server ({}) does not distribute the CLI; ask whoever runs it to enable it (deploy/README.md, \"CLI distribution\")",
            base.as_str().trim_end_matches('/')
        ),
    )
}

/// The install line for this platform, against `base`.
fn install_line(base: &url::Url) -> String {
    let b = base.as_str().trim_end_matches('/');
    if cfg!(windows) {
        format!("irm {b}/cli/install.ps1 | iex")
    } else {
        format!("curl --proto '=https' --tlsv1.2 -fsS {b}/cli/install.sh | sh")
    }
}

/// Writes `bin` beside `exe` and swaps it in. The temporary file comes from `atomic_file::create_temporary` (exclusive,
/// mode 0600 on Unix, a fresh name when one exists); from then on it is this run's, and only it is removed on a failure.
/// `0755` is set through the open handle before it is closed, so the file is never writable by others.
fn install(exe: &Path, bin: &[u8]) -> std::io::Result<()> {
    let (tmp, mut f) = crate::atomic_file::create_temporary(exe)?;
    let written = (|| -> std::io::Result<()> {
        f.write_all(bin)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(std::fs::Permissions::from_mode(0o755))?;
        }
        f.sync_all()
    })();
    drop(f);
    let result = written.and_then(|()| swap(exe, &tmp));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// What to tell someone whose executable could not be replaced. An executable a package manager installed read-only
/// (the nix store) lands here too, as permission denied.
fn cannot_replace(exe: &Path, base: &url::Url, e: &std::io::Error) -> Failure {
    let next = if cfg!(windows) && matches!(e.raw_os_error(), Some(5 | 32)) {
        // ERROR_ACCESS_DENIED / ERROR_SHARING_VIOLATION: what Windows answers while antivirus or another window holds
        // the file.
        "another program is using it: close other reeve windows (or wait for antivirus to finish scanning it), then run reeve update again".to_string()
    } else if matches!(
        e.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
    ) {
        format!(
            "permission denied: run reeve update as the owner of that file (with REEVE_URL={}), update it the way it was installed, or reinstall to ~/.local/bin with `{}`",
            base.as_str().trim_end_matches('/'),
            install_line(base)
        )
    } else {
        e.to_string()
    };
    fail_because(
        ErrorCode::Internal,
        "cannot_replace",
        format!("cannot replace {}: {next}", exe.display()),
    )
}

pub async fn run(_u: Update, json: bool) -> anyhow::Result<i32> {
    match attempt(json).await {
        Ok(code) => Ok(code),
        Err(f) => {
            if json {
                let mut v =
                    serde_json::json!({"outcome":"error","code":f.code,"message":f.message});
                if let Some(r) = f.reason {
                    v["reason"] = r.into();
                }
                println!("{v}");
            }
            eprintln!("error: {}", f.message);
            Ok(1)
        }
    }
}

async fn attempt(json: bool) -> Result<i32, Failure> {
    // The url only: no token is read, from REEVE_TOKEN or the stored login (kb/ADR-0030 R5).
    let base = crate::config::env_nonempty("REEVE_URL")
        .or(crate::config::load().url)
        .ok_or_else(|| fail(ErrorCode::Validation, crate::config::NO_SERVER_URL))?;
    let base = check_url(base.trim_end_matches('/')).map_err(|m| fail(ErrorCode::Validation, m))?;
    let file = platform_file().ok_or_else(|| {
        fail(
            ErrorCode::Internal,
            "the server ships no CLI for this platform",
        )
    })?;
    // A client of its own: no default headers, no cookie store, no redirects. It is not a `crate::client::Client`, so
    // it never records the offered build: `reeve update` never ends with the update notice.
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        // Not one timeout for the whole request: a slow link may take minutes for the binary, but must keep moving.
        .connect_timeout(std::time::Duration::from_secs(10))
        .read_timeout(std::time::Duration::from_secs(60));
    if base.scheme() == "http" {
        // Plain http is allowed to loopback only; HTTP_PROXY must not carry it anywhere else (kb/ADR-0030 R5).
        builder = builder.no_proxy();
    }
    let http = builder
        .build()
        .map_err(|e| fail(ErrorCode::Internal, e.to_string()))?;
    let current = cli_version::current();
    let exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(|e| {
            fail_because(
                ErrorCode::Internal,
                "cannot_replace",
                format!("cannot locate the running executable: {e}"),
            )
        })?;
    let v = get(&http, &base, "VERSION", TEXT_CAP).await?;
    let offered = String::from_utf8_lossy(&v.bytes).trim().to_string();
    // An older server answers every unknown path with its web page: not text/plain, not a CLI version.
    if !v.content_type.starts_with("text/plain") || !is_cli_version(&offered) {
        return Err(not_distributed(&base));
    }
    if current.as_deref() == Some(offered.as_str()) {
        if json {
            println!(
                "{}",
                serde_json::json!({"outcome":"ok","value":{"from":current,"to":offered,"updated":false}})
            );
        } else {
            println!("reeve is up to date (build {})", short(&offered));
        }
        return Ok(0);
    }
    let sums = get(&http, &base, "SHA256SUMS", TEXT_CAP).await?;
    let bin = get(&http, &base, file, BINARY_CAP).await?;
    // A deploy between these requests can serve the sums and the binary from different bundles (the checksum below
    // catches that), or sums and binary that agree with each other but are not the build VERSION named (the second
    // VERSION read after the checksum catches that).
    let mismatch = |why: String| {
        fail_because(
            ErrorCode::Internal,
            "checksum_mismatch",
            format!(
                "{why}; nothing was replaced. Try again; if it repeats, tell whoever runs the server"
            ),
        )
    };
    let want = expected_sum(&String::from_utf8_lossy(&sums.bytes), file).map_err(mismatch)?;
    if hex::encode(Sha256::digest(&bin.bytes)) != want {
        return Err(mismatch(format!(
            "{file} does not match its SHA256SUMS line"
        )));
    }
    let again = get(&http, &base, "VERSION", TEXT_CAP).await?;
    if String::from_utf8_lossy(&again.bytes).trim() != offered {
        return Err(mismatch(
            "the server's build changed while updating".to_string(),
        ));
    }
    install(&exe, &bin.bytes).map_err(|e| cannot_replace(&exe, &base, &e))?;
    let from = current
        .as_deref()
        .map_or("dev build".to_string(), |c| format!("build {}", short(c)));
    if json {
        println!(
            "{}",
            serde_json::json!({"outcome":"ok","value":{"from":current,"to":offered,"updated":true}})
        );
    } else {
        println!(
            "replaced reeve {from} with the server's {}",
            short(&offered)
        );
    }
    Ok(0)
}

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
    if u.query().is_some() || u.fragment().is_some() {
        return Err("the server url must not carry a query or a fragment".to_string());
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

/// `rename`, tried up to 5 times, `delay` apart, while Windows answers access denied (5) or sharing violation (32):
/// antivirus and indexers hold a freshly written file for a moment.
#[cfg_attr(not(windows), allow(dead_code))]
fn rename_retrying(
    rename: &impl Fn(&Path, &Path) -> std::io::Result<()>,
    from: &Path,
    to: &Path,
    delay: std::time::Duration,
) -> std::io::Result<()> {
    let mut tries = 1;
    loop {
        match rename(from, to) {
            Err(e) if tries < 5 && matches!(e.raw_os_error(), Some(5 | 32)) => {
                tries += 1;
                std::thread::sleep(delay);
            }
            r => return r,
        }
    }
}

/// Moves `exe` aside to the first free backup name, then `new` into its place (retrying briefly, see
/// `rename_retrying`); if that fails, moves `exe` back, so a failure leaves the original where it was. Returns the
/// backup's path. `fault` is the test hook; `rename` is `std::fs::rename` outside tests.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn swap_aside(
    exe: &Path,
    new: &Path,
    fault: impl Fn(&str) -> std::io::Result<()>,
    rename: impl Fn(&Path, &Path) -> std::io::Result<()>,
    delay: std::time::Duration,
) -> std::io::Result<PathBuf> {
    let old = (0..1000)
        .map(|n| backup_name(exe, n))
        .find(|p| !p.exists())
        .ok_or_else(|| {
            std::io::Error::other(
                "1000 backups beside the executable; delete the reeve.exe.old.* files",
            )
        })?;
    rename(exe, &old)?;
    if let Err(e) = fault("after_move").and_then(|()| rename_retrying(&rename, new, exe, delay)) {
        return match rename(&old, exe) {
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
        swap_aside(
            exe,
            new,
            fault,
            |a, b| std::fs::rename(a, b),
            std::time::Duration::from_millis(200),
        )
        .map(|_| ())
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
            "https://reeve.example.com/?a=1",
            "https://reeve.example.com/#x",
            "https://reeve.example.com?",
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
        let err = swap_aside(
            &exe,
            &new,
            |p| {
                if p == "after_move" {
                    Err(std::io::Error::other("boom"))
                } else {
                    Ok(())
                }
            },
            |a, b| std::fs::rename(a, b),
            std::time::Duration::ZERO,
        )
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
        let used = swap_aside(
            &exe,
            &new,
            |_| Ok(()),
            |a, b| std::fs::rename(a, b),
            std::time::Duration::ZERO,
        )
        .unwrap();
        assert_ne!(used, held, "a fresh name");
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new binary");
        assert_eq!(std::fs::read_to_string(&used).unwrap(), "old binary");
        assert_eq!(
            std::fs::read_to_string(&held).unwrap(),
            "held",
            "the held backup is untouched"
        );
    }

    /// A rename that fails `fails` times with `code`, then works.
    fn flaky(
        code: i32,
        fails: usize,
    ) -> (
        std::rc::Rc<std::cell::Cell<usize>>,
        impl Fn(&Path, &Path) -> std::io::Result<()>,
    ) {
        let n = std::rc::Rc::new(std::cell::Cell::new(0));
        let f = {
            let n = n.clone();
            move |a: &Path, b: &Path| {
                if a.file_name().is_some_and(|f| f == "new") && n.get() < fails {
                    n.set(n.get() + 1);
                    return Err(std::io::Error::from_raw_os_error(code));
                }
                std::fs::rename(a, b)
            }
        };
        (n, f)
    }

    #[test]
    fn a_held_new_file_is_retried_then_given_up_with_the_original_back() {
        let d = tempfile::tempdir().unwrap();
        let (exe, new) = two_files(d.path());
        let z = std::time::Duration::ZERO;
        let (n, r) = flaky(32, 2);
        swap_aside(&exe, &new, |_| Ok(()), r, z).unwrap();
        assert_eq!(n.get(), 2, "two failures, then success");
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new binary");

        let d = tempfile::tempdir().unwrap();
        let (exe, new) = two_files(d.path());
        let (n, r) = flaky(5, 99);
        let e = swap_aside(&exe, &new, |_| Ok(()), r, z).unwrap_err();
        assert_eq!(e.raw_os_error(), Some(5));
        assert_eq!(n.get(), 5, "five tries, no more");
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old binary");

        // Any other error is not retried.
        let d = tempfile::tempdir().unwrap();
        let (exe, new) = two_files(d.path());
        let (n, r) = flaky(2, 99);
        swap_aside(&exe, &new, |_| Ok(()), r, z).unwrap_err();
        assert_eq!(n.get(), 1);
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
