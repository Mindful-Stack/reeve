//! `reeve update` (kb/ADR-0030 R5, R7): replaces this executable with the build the server offers, verified against the
//! single SHA-256 line for this platform's file, over HTTPS, with no credentials and no redirects.
use crate::cli_version::{self, short};
use reeve_protocol::{
    CLI_BINARIES, CLI_INSTALL_PS1, CLI_INSTALL_SH, CLI_PATH, CLI_SUMS_FILE, CLI_TEXT_CONTENT_TYPE,
    CLI_VERSION_FILE, ErrorCode, is_cli_version,
};
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

/// One downloaded file.
struct Got {
    bytes: Vec<u8>,
    content_type: String,
    /// The build the answer says it belongs to (`X-Reeve-Cli`), when it names a valid one; a garbled value is ignored,
    /// as `Client` ignores it (`cli_version::offered_in`).
    build: Option<String>,
}

/// How fetching one `/cli/` file failed, before any rule about that file: `offered_build` decides what a missing or
/// over-long `VERSION` means, `file` what it means for the others.
enum Fetch {
    NotFound,
    TooLarge,
    Failed(Failure),
}

/// The server url without its trailing slash, as messages show it.
fn shown(base: &url::Url) -> &str {
    base.as_str().trim_end_matches('/')
}

/// A request that got no answer, with its cause: reqwest's own message names only the kind ("error sending request"),
/// the cause (refused, DNS, TLS, proxy, timeout) is in its `source()` chain, which is what someone needs to act on.
fn transport(url: &str, e: reqwest::Error) -> Failure {
    let e = e.without_url();
    let mut message = format!("GET {url}: {e}");
    let mut cause = std::error::Error::source(&e);
    while let Some(c) = cause {
        message.push_str(&format!(": {c}"));
        cause = c.source();
    }
    fail(ErrorCode::Internal, message)
}

/// Fetches `/cli/<name>`, reading at most `cap` bytes (a longer `Content-Length` is refused before reading, and reading
/// stops at the cap when the length is absent or wrong). Transport only: no rule about any one file lives here.
async fn get(
    http: &reqwest::Client,
    base: &url::Url,
    name: &str,
    cap: usize,
) -> Result<Got, Fetch> {
    let url = format!("{}{CLI_PATH}{name}", shown(base));
    let mut res = http
        .get(&url)
        .send()
        .await
        .map_err(|e| Fetch::Failed(transport(&url, e)))?;
    let status = res.status();
    if status.is_redirection() {
        let to = res
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("?");
        return Err(Fetch::Failed(fail(
            ErrorCode::Validation,
            format!(
                "the server redirected {CLI_PATH}{name} to {to}; reeve update follows no redirects (is the server url {} right?)",
                shown(base)
            ),
        )));
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(Fetch::Failed(fail(
            ErrorCode::RateLimited,
            format!(
                "the server's proxy is rate-limiting {CLI_PATH}; wait a minute and run reeve update again"
            ),
        )));
    }
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(Fetch::NotFound);
    }
    if status.is_server_error() {
        return Err(Fetch::Failed(fail(
            ErrorCode::Internal,
            format!(
                "GET {CLI_PATH}{name}: HTTP {status}; the server may be restarting: wait a minute and run reeve update again"
            ),
        )));
    }
    if !status.is_success() {
        return Err(Fetch::Failed(fail(
            ErrorCode::Internal,
            format!("GET {CLI_PATH}{name}: HTTP {status}"),
        )));
    }
    let content_type = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let build = cli_version::offered_in(res.headers());
    if res.content_length().is_some_and(|n| n > cap as u64) {
        return Err(Fetch::TooLarge);
    }
    let mut bytes = Vec::with_capacity(res.content_length().unwrap_or(0).min(cap as u64) as usize);
    while let Some(chunk) = res
        .chunk()
        .await
        .map_err(|e| Fetch::Failed(transport(&url, e)))?
    {
        if bytes.len() + chunk.len() > cap {
            return Err(Fetch::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Got {
        bytes,
        content_type,
        build,
    })
}

/// The build the server offers, read from `VERSION`: the one place that decides whether this server distributes the
/// CLI. A 404, an answer over the cap, one not served as `text/plain` (an older server answers unknown paths with its
/// web page) or one that is not a CLI version all mean `not_distributed`. Used for the first read and the re-read alike.
async fn offered_build(http: &reqwest::Client, base: &url::Url) -> Result<String, Failure> {
    let got = match get(http, base, CLI_VERSION_FILE, TEXT_CAP).await {
        Ok(g) => g,
        Err(Fetch::NotFound | Fetch::TooLarge) => return Err(not_distributed(base)),
        Err(Fetch::Failed(f)) => return Err(f),
    };
    let offered = String::from_utf8_lossy(&got.bytes).trim().to_string();
    if is_text(&got.content_type) && is_cli_version(&offered) {
        Ok(offered)
    } else {
        Err(not_distributed(base))
    }
}

/// Any other `/cli/` file: missing or over the cap is a plain error.
async fn file(
    http: &reqwest::Client,
    base: &url::Url,
    name: &str,
    cap: usize,
) -> Result<Got, Failure> {
    get(http, base, name, cap).await.map_err(|f| match f {
        Fetch::NotFound => fail(
            ErrorCode::Internal,
            format!("GET {CLI_PATH}{name}: HTTP 404 Not Found"),
        ),
        Fetch::TooLarge => fail(
            ErrorCode::Internal,
            format!(
                "GET {CLI_PATH}{name}: the answer is larger than {cap} bytes; nothing was replaced"
            ),
        ),
        Fetch::Failed(f) => f,
    })
}

/// Whether a `Content-Type` is the bundle's text type, with or without parameters (`text/plain; charset=utf-8`).
fn is_text(content_type: &str) -> bool {
    content_type
        .split(';')
        .next()
        .is_some_and(|t| t.trim().eq_ignore_ascii_case(CLI_TEXT_CONTENT_TYPE))
}

fn not_distributed(base: &url::Url) -> Failure {
    fail_because(
        ErrorCode::NotFound,
        "not_distributed",
        format!(
            "this server ({}) does not distribute the CLI; ask whoever runs it to enable it (deploy/README.md, \"CLI distribution\")",
            shown(base)
        ),
    )
}

/// The install line against `base`, for Windows (PowerShell) or the others (sh).
fn install_line(base: &url::Url, windows: bool) -> String {
    let b = shown(base);
    if windows {
        // Windows PowerShell 5.1 may not offer TLS 1.2 by default; the same prefix the installer prints.
        format!(
            "[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor \
             [Net.SecurityProtocolType]::Tls12; irm {b}{CLI_PATH}{CLI_INSTALL_PS1} | iex"
        )
    } else if base.scheme() == "http" {
        // Plain http is allowed only to loopback, and then never through a proxy (kb/ADR-0030.R5).
        format!("curl -q --noproxy '*' -fsS {b}{CLI_PATH}{CLI_INSTALL_SH} | sh")
    } else {
        format!("curl -q --proto '=https' --tlsv1.2 -fsS {b}{CLI_PATH}{CLI_INSTALL_SH} | sh")
    }
}

/// Why `install` failed: `writing` when the temporary file beside the executable could not be written (so the
/// directory is the obstacle), otherwise the swap itself failed.
struct InstallError {
    writing: bool,
    error: std::io::Error,
}

/// Writes `bin` beside `exe` and swaps it in. The temporary file comes from `atomic_file::create_temporary` (exclusive,
/// mode 0600 on Unix, a fresh name when one exists); from then on it is this run's, and only it is removed on a failure.
/// `0755` is set through the open handle before it is closed, so the file is never writable by others.
fn install(exe: &Path, bin: &[u8]) -> Result<(), InstallError> {
    let writing = |error| InstallError {
        writing: true,
        error,
    };
    let (tmp, mut f) = crate::atomic_file::create_temporary(exe).map_err(writing)?;
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
    let result = written.map_err(writing).and_then(|()| {
        swap(exe, &tmp).map_err(|error| InstallError {
            writing: false,
            error,
        })
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// An error from a rename that Windows refused because something holds the file (access denied or sharing violation):
/// the only place `cannot_replace` says "another program is using it".
#[derive(Debug)]
struct Held(std::io::Error);
impl std::fmt::Display for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Held {}

#[cfg_attr(not(windows), allow(dead_code))]
fn held(e: std::io::Error) -> std::io::Error {
    if matches!(e.raw_os_error(), Some(5 | 32)) {
        std::io::Error::new(e.kind(), Held(e))
    } else {
        e
    }
}

/// What to tell someone who may not replace the executable `exe`. `in_directory`: its directory is not writable (the
/// temporary file could not be created there), rather than the file itself refusing to be replaced. `windows` picks
/// the install location and line the installer for that platform uses.
fn permission_advice(exe: &Path, in_directory: bool, base: &url::Url, windows: bool) -> String {
    let who = if in_directory {
        let dir = exe.parent().unwrap_or(exe);
        format!(
            "its directory ({}) is not writable by you: run reeve update as someone who can write there",
            dir.display()
        )
    } else {
        "the file is not yours to replace: run reeve update as its owner".to_string()
    };
    let place = if windows {
        r"%LOCALAPPDATA%\Programs\reeve"
    } else {
        "~/.local/bin"
    };
    format!(
        "permission denied: {who} (with REEVE_URL={}), update it the way it was installed, or reinstall to {place} with `{}`",
        shown(base),
        install_line(base, windows)
    )
}

/// What to tell someone whose executable could not be replaced. An executable a package manager installed read-only
/// (the nix store) lands here too, as permission denied.
fn cannot_replace(exe: &Path, base: &url::Url, e: &InstallError) -> Failure {
    let next = if e.error.get_ref().is_some_and(|i| i.is::<Held>()) {
        // A swap rename refused with ERROR_ACCESS_DENIED / ERROR_SHARING_VIOLATION: antivirus or another window holds
        // the file. The same codes from creating the temporary file mean permissions, and take the next arm.
        "another program is using it: close other reeve windows (or wait for antivirus to finish scanning it), then run reeve update again".to_string()
    } else if matches!(
        e.error.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
    ) {
        permission_advice(exe, e.writing, base, cfg!(windows))
    } else {
        e.error.to_string()
    };
    fail_because(
        ErrorCode::Internal,
        "cannot_replace",
        format!("cannot replace {}: {next}", exe.display()),
    )
}

/// A line on standard output or error that cannot panic: once the binary is replaced, a closed stream must not turn a
/// successful update into exit status 101.
fn say(to: &mut dyn Write, line: std::fmt::Arguments<'_>) {
    let _ = writeln!(to, "{line}");
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
                say(&mut std::io::stdout(), format_args!("{v}"));
            }
            say(&mut std::io::stderr(), format_args!("error: {}", f.message));
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
    let file_name = platform_file().ok_or_else(|| {
        fail(
            ErrorCode::Internal,
            format!(
                "no CLI is distributed for this platform ({}-{})",
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
        )
    })?;
    // A client of its own: no default headers, no cookie store, no redirects. It is not a `crate::client::Client`, so
    // it never records the offered build: `reeve update` never ends with the update notice.
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        // Not one timeout for the whole request: a slow link may take minutes for the binary, but must keep moving.
        .connect_timeout(std::time::Duration::from_secs(10))
        // And an overall ceiling per request, so a server that trickles bytes forever cannot hold the command.
        .timeout(std::time::Duration::from_secs(30 * 60))
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
    let offered = offered_build(&http, &base).await?;
    if current.as_deref() == Some(offered.as_str()) {
        if json {
            say(
                &mut std::io::stdout(),
                format_args!(
                    "{}",
                    serde_json::json!({"outcome":"ok","value":{"from":current,"to":offered,"updated":false}})
                ),
            );
        } else {
            say(
                &mut std::io::stdout(),
                format_args!("reeve is up to date (build {})", short(&offered)),
            );
        }
        return Ok(0);
    }
    let sums = file(&http, &base, CLI_SUMS_FILE, TEXT_CAP).await?;
    let bin = file(&http, &base, file_name, BINARY_CAP).await?;
    // What is guaranteed: the binary matches the SHA256SUMS line it was verified against; no answer that names a valid
    // build (`X-Reeve-Cli`) names another than the one VERSION offered; and VERSION still offers that build after the
    // download. A deploy that lands and is undone between requests, on a server that stamps no header, is not seen.
    let mismatch = |why: String| {
        fail_because(
            ErrorCode::Internal,
            "checksum_mismatch",
            format!(
                "{why}; nothing was replaced. Try again; if it repeats, tell whoever runs the server"
            ),
        )
    };
    let changed = || mismatch("the build the server offers changed while updating".to_string());
    if [&sums, &bin]
        .iter()
        .any(|got| got.build.as_deref().is_some_and(|b| b != offered))
    {
        return Err(changed());
    }
    let want = expected_sum(&String::from_utf8_lossy(&sums.bytes), file_name).map_err(mismatch)?;
    if hex::encode(Sha256::digest(&bin.bytes)) != want {
        return Err(mismatch(format!(
            "{file_name} does not match its {CLI_SUMS_FILE} line"
        )));
    }
    if offered_build(&http, &base).await? != offered {
        return Err(changed());
    }
    install(&exe, &bin.bytes).map_err(|e| cannot_replace(&exe, &base, &e))?;
    let from = current
        .as_deref()
        .map_or("dev build".to_string(), |c| format!("build {}", short(c)));
    if json {
        say(
            &mut std::io::stdout(),
            format_args!(
                "{}",
                serde_json::json!({"outcome":"ok","value":{"from":current,"to":offered,"updated":true}})
            ),
        );
    } else {
        say(
            &mut std::io::stdout(),
            format_args!(
                "replaced reeve {from} with the server's {}",
                short(&offered)
            ),
        );
    }
    Ok(0)
}

/// This platform's file under `/cli/`, from the protocol's fixed list; `None` where no CLI is distributed.
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
    CLI_BINARIES.iter().copied().find(|n| *n == want)
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
        "http" => Err(
            "refusing to download over http: the server url must be https (plain http only to a loopback address). \
             To keep this server and this build, set REEVE_UPDATE_NOTICE=0 to silence the update notice"
                .to_string(),
        ),
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
            "{CLI_SUMS_FILE} has {} lines for {file}, not one",
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
        Err(format!(
            "{CLI_SUMS_FILE}'s line for {file} holds no SHA-256"
        ))
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

/// `rename`, tried up to 10 times, sleeping `delay`, `2 * delay`, ... between tries (about 4.5 s at 100 ms), while Windows
/// answers access denied (5) or sharing violation (32): antivirus and indexers hold a freshly written file for a
/// moment. A rename still refused afterwards is returned as `Held`.
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
            Err(e) if tries < 10 && matches!(e.raw_os_error(), Some(5 | 32)) => {
                std::thread::sleep(delay * tries);
                tries += 1;
            }
            r => return r.map_err(held),
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
    rename(exe, &old).map_err(held)?;
    if let Err(e) = fault("after_move").and_then(|()| rename_retrying(&rename, new, exe, delay)) {
        // Restored with the same retries: antivirus may hold the backup for a moment too, and giving up here would leave
        // no `reeve.exe` at all.
        return match rename_retrying(&rename, &old, exe, delay) {
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
            std::time::Duration::from_millis(100),
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
    fn plain_http_off_loopback_says_how_to_silence_the_notice() {
        // Commands work over it and print the notice, but reeve update never will: say how to stop asking.
        let e = check_url("http://reeve.lan:7474").unwrap_err();
        assert!(
            e.contains("must be https") && e.contains("REEVE_UPDATE_NOTICE=0"),
            "{e}"
        );
        let e = check_url("ftp://reeve.example.com").unwrap_err();
        assert!(!e.contains("REEVE_UPDATE_NOTICE"), "{e}");
    }

    #[test]
    fn the_text_type_with_or_without_parameters() {
        for ok in [
            "text/plain",
            "text/plain; charset=utf-8",
            "Text/Plain;charset=UTF-8",
        ] {
            assert!(is_text(ok), "{ok}");
        }
        for bad in [
            "",
            "text/html; charset=utf-8",
            "text/plainx",
            "application/octet-stream",
        ] {
            assert!(!is_text(bad), "{bad}");
        }
    }

    #[test]
    fn permission_advice_names_the_obstacle_and_this_platforms_installer() {
        let base = url::Url::parse("https://reeve.example.com").unwrap();
        let exe = Path::new("/opt/tools/reeve");
        let dir = permission_advice(exe, true, &base, false);
        assert!(
            dir.contains("its directory (/opt/tools) is not writable by you")
                && dir.contains("someone who can write there")
                && dir.contains("~/.local/bin")
                && dir.contains("https://reeve.example.com/cli/install.sh | sh"),
            "{dir}"
        );
        let file = permission_advice(exe, false, &base, false);
        assert!(
            file.contains("the file is not yours to replace") && !file.contains("directory"),
            "{file}"
        );
        let win = permission_advice(Path::new(r"C:\tools\reeve.exe"), true, &base, true);
        assert!(
            win.contains(r"%LOCALAPPDATA%\Programs\reeve")
                && win.contains("Tls12; irm https://reeve.example.com/cli/install.ps1 | iex")
                && !win.contains("~/.local/bin"),
            "{win}"
        );
    }

    #[test]
    fn a_loopback_install_hint_bypasses_proxies() {
        let base = url::Url::parse("http://127.0.0.1:7474").unwrap();
        let line = install_line(&base, false);
        assert!(
            line.contains("--noproxy '*'") && !line.contains("=https"),
            "{line}"
        );
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
        assert!(e.get_ref().is_some_and(|i| i.is::<Held>()), "{e}");
        assert_eq!(n.get(), 10, "ten tries, no more");
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old binary");

        // Any other error is not retried.
        let d = tempfile::tempdir().unwrap();
        let (exe, new) = two_files(d.path());
        let (n, r) = flaky(2, 99);
        swap_aside(&exe, &new, |_| Ok(()), r, z).unwrap_err();
        assert_eq!(n.get(), 1);
    }

    /// Installing fails, and antivirus holds the moved-aside original for a moment: the restore is retried like the
    /// install, so the original is back rather than stranded at `reeve.exe.old.<n>`.
    #[test]
    fn a_held_backup_is_retried_when_putting_the_original_back() {
        for code in [5, 32] {
            let d = tempfile::tempdir().unwrap();
            let (exe, new) = two_files(d.path());
            let restores = std::cell::Cell::new(0);
            let rename = |a: &Path, b: &Path| {
                let from = a.file_name().unwrap().to_string_lossy().into_owned();
                if from == "new" {
                    return Err(std::io::Error::from_raw_os_error(2));
                }
                if from.contains(".old.") {
                    restores.set(restores.get() + 1);
                    if restores.get() == 1 {
                        return Err(std::io::Error::from_raw_os_error(code));
                    }
                }
                std::fs::rename(a, b)
            };
            let e =
                swap_aside(&exe, &new, |_| Ok(()), rename, std::time::Duration::ZERO).unwrap_err();
            assert_eq!(
                e.raw_os_error(),
                Some(2),
                "the install's error is reported: {e}"
            );
            assert_eq!(restores.get(), 2, "held once, then restored (code {code})");
            assert_eq!(
                std::fs::read_to_string(&exe).unwrap(),
                "old binary",
                "the original is back in place (code {code})"
            );
            assert!(
                !backup_name(&exe, 0).exists(),
                "nothing left aside (code {code})"
            );
        }
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
