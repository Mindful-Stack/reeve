//! `reeve update` end to end: a copy of the real binary, run as one build against an in-test server that offers another,
//! replaces itself only with a complete, verified file and never sends a credential (kb/ADR-0030 R5, R7).
use axum::{
    Router,
    body::Body,
    extract::{Path as UrlPath, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccccccccccc";

/// The file this host must download, written out per target (not taken from the code under test).
fn this_target() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return "reeve-linux-x86_64";
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return "reeve-linux-aarch64";
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return "reeve-macos-aarch64";
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return "reeve-macos-x86_64";
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return "reeve-windows-x86_64.exe";
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
    compile_error!(
        "reeve update ships for five targets only; this test has nothing to download here"
    );
}

/// The real binary, read once. (The debug binary is ~150 MB, about 59% of `BINARY_CAP`: a much bigger one would make
/// every test here fail the cap, not the behaviour under test.)
fn real() -> &'static [u8] {
    static REAL: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    REAL.get_or_init(|| std::fs::read(env!("CARGO_BIN_EXE_reeve")).unwrap())
}

/// A scratch directory on disk under the cargo target, never `/tmp` (a RAM-backed tmpfs on some machines: every test
/// here copies the binary).
fn tmp() -> tempfile::TempDir {
    tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap()
}
/// What the fake serves as `name`: the real binary made distinct per name, and still runnable. Everywhere but macOS a
/// marker naming the file is appended (Linux and Windows run a binary with trailing bytes).
#[cfg(not(target_os = "macos"))]
fn served(name: &str) -> Vec<u8> {
    let mut b = real().to_vec();
    b.extend_from_slice(format!("\0reeve-update-test:{name}\0").as_bytes());
    b
}

/// On macOS appended bytes invalidate the signature (Apple Silicon kills such a binary at launch), so the copy is
/// re-signed ad hoc instead, with an identifier naming the file: distinct bytes per name, a valid signature, and the
/// same bytes every call (an ad-hoc signature carries no timestamp), so the checksum computed here matches what is served.
#[cfg(target_os = "macos")]
fn served(name: &str) -> Vec<u8> {
    let dir = tmp();
    let p = dir.path().join(name);
    std::fs::write(&p, real()).unwrap();
    let out = std::process::Command::new("codesign")
        .args(["--force", "--timestamp=none", "--sign", "-", "--identifier"])
        .arg(format!("reeve-update-test.{name}"))
        .arg(&p)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "codesign: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::read(&p).unwrap()
}

/// Proxy variables a developer's shell may set: every child here runs without them, so a request meant for the in-test
/// server cannot go to a proxy instead (a test that wants one sets it itself).
const PROXY_VARS: [&str; 8] = [
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
];

/// `c` without any proxy variable, and killed if the test drops it (a timed-out test leaves no child behind).
fn isolated(c: &mut tokio::process::Command) -> &mut tokio::process::Command {
    for v in PROXY_VARS {
        c.env_remove(v);
    }
    c.kill_on_drop(true)
}

#[derive(Default)]
struct Fake {
    version: String,
    sums: Option<String>,
    redirect: bool,
    distributes: bool,
    /// Answer every name like a server older than this feature: its web page.
    old_server: bool,
    /// What `VERSION` answers from its second request on (a deploy while the update runs).
    later_version: Option<String>,
    /// A body of this many bytes for `VERSION` or `SHA256SUMS`, beyond any cap: the real answer (a valid version, so
    /// only the cap can refuse it) padded with spaces; the flag streams it without a `Content-Length`.
    huge: Option<(&'static str, usize, bool)>,
    /// `X-Reeve-Cli` on the SHA256SUMS and binary answers (PR B stamps every response).
    stamp_files: Option<String>,
    /// An error status for `SHA256SUMS` (a server mid-deploy).
    sums_status: Option<StatusCode>,
    seen: Mutex<Vec<(String, bool)>>,
}

fn sums() -> String {
    static SUMS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SUMS.get_or_init(|| {
        reeve_protocol::CLI_BINARIES
            .iter()
            .map(|n| format!("{}  {n}\n", hex::encode(Sha256::digest(served(n)))))
            .collect()
    })
    .clone()
}

async fn cli(
    State(f): State<Arc<Fake>>,
    UrlPath(name): UrlPath<String>,
    headers: HeaderMap,
) -> Response {
    let creds = headers.contains_key(header::AUTHORIZATION) || headers.contains_key(header::COOKIE);
    f.seen.lock().unwrap().push((name.clone(), creds));
    if f.old_server {
        return (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            "<!doctype html><title>Reeve</title>",
        )
            .into_response();
    }
    if !f.distributes {
        return StatusCode::NOT_FOUND.into_response();
    }
    if f.redirect {
        return (
            StatusCode::FOUND,
            [(header::LOCATION, "https://elsewhere.example/cli/VERSION")],
        )
            .into_response();
    }
    if let Some((n, size, stream)) = f.huge
        && n == name
    {
        let mut body = format!("{}\n", f.version).into_bytes();
        body.resize(size, b' ');
        let ct = [(header::CONTENT_TYPE, "text/plain; charset=utf-8")];
        if !stream {
            return (ct, body).into_response();
        }
        let chunks = body
            .chunks(8192)
            .map(|c| Ok::<_, std::convert::Infallible>(axum::body::Bytes::copy_from_slice(c)))
            .collect::<Vec<_>>();
        return (ct, Body::from_stream(futures_util::stream::iter(chunks))).into_response();
    }
    let asked = f
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|(n, _)| n == "VERSION")
        .count();
    if let Some(st) = f.sums_status
        && name == "SHA256SUMS"
    {
        return st.into_response();
    }
    let (body, ty) = match name.as_str() {
        "VERSION" => {
            let v = match &f.later_version {
                Some(l) if asked > 1 => l,
                _ => &f.version,
            };
            (format!("{v}\n").into_bytes(), "text/plain; charset=utf-8")
        }
        "SHA256SUMS" => (
            f.sums.clone().unwrap_or_else(sums).into_bytes(),
            "text/plain; charset=utf-8",
        ),
        n if reeve_protocol::CLI_BINARIES.contains(&n) => (served(n), "application/octet-stream"),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let mut res = ([(header::CONTENT_TYPE, ty)], body).into_response();
    if let Some(b) = &f.stamp_files
        && name != "VERSION"
    {
        res.headers_mut()
            .insert(reeve_protocol::CLI_VERSION_HEADER, b.parse().unwrap());
    }
    res
}

async fn serve(f: Arc<Fake>) -> String {
    let app = Router::new().route("/cli/{name}", get(cli)).with_state(f);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    url
}

/// A copy of the binary in a fresh directory (never the cargo target's own binary: the update replaces it).
fn installed(dir: &Path) -> PathBuf {
    let p = dir.join(if cfg!(windows) { "reeve.exe" } else { "reeve" });
    std::fs::copy(env!("CARGO_BIN_EXE_reeve"), &p).unwrap();
    p
}

struct Ran {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Runs `<exe> update <args>` as `build` (`"dev"`: unstamped) with poisoned credentials wherever the CLI could read
/// one, so a request that carried one would be seen.
async fn update(exe: &Path, url: &str, build: &str, env: &[(&str, &str)], args: &[&str]) -> Ran {
    let cfg = tmp();
    std::fs::write(
        cfg.path().join("config.toml"),
        "url = \"http://127.0.0.1:1\"\ntoken = \"reeve_stored_poison\"\n",
    )
    .unwrap();
    let mut c = tokio::process::Command::new(exe);
    c.arg("update")
        .args(args)
        .env("REEVE_URL", url)
        .env("REEVE_TOKEN", "reeve_env_poison")
        .env("REEVE_CONFIG_DIR", cfg.path())
        .env("HOME", cfg.path())
        .env("REEVE_CLI_VERSION_OVERRIDE", build)
        .env_remove("REEVE_UPDATE_FAULT")
        .env_remove("REEVE_UPDATE_NOTICE")
        .env_remove("REEVE_ASCII")
        .stdin(std::process::Stdio::null());
    isolated(&mut c);
    for (k, v) in env {
        c.env(k, v);
    }
    let out = tokio::time::timeout(std::time::Duration::from_secs(60), c.output())
        .await
        .unwrap()
        .unwrap();
    Ran {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn fake(version: &str) -> Fake {
    Fake {
        version: version.into(),
        distributes: true,
        ..Default::default()
    }
}

fn names(f: &Fake) -> Vec<String> {
    f.seen
        .lock()
        .unwrap()
        .iter()
        .map(|(n, _)| n.clone())
        .collect()
}

/// No request carried `Authorization` or `Cookie`, whatever the outcome (the poisoned token and config would show).
fn no_credential_sent(f: &Fake) {
    assert!(
        f.seen.lock().unwrap().iter().all(|(_, creds)| !creds),
        "no Authorization or Cookie, ever"
    );
}

/// The parsed `--json` error envelope.
fn envelope(r: &Ran) -> serde_json::Value {
    serde_json::from_str(&r.stdout).unwrap_or_else(|_| panic!("JSON: {}", r.stdout))
}

/// Only the installed binary (and, on Windows, this update's backup) is left: no temporary file.
fn no_temporary_file(dir: &Path) {
    let left: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let stray: Vec<&String> = left
        .iter()
        .filter(|n| *n != "reeve" && *n != "reeve.exe" && !n.starts_with("reeve.exe.old."))
        .collect();
    assert!(stray.is_empty(), "no temporary file left: {left:?}");
}

#[tokio::test]
async fn a_stamped_cli_takes_the_offered_build_without_sending_a_credential() {
    let f = Arc::new(fake(B));
    let dir = tmp();
    let exe = installed(dir.path());
    let r = update(&exe, &serve(f.clone()).await, A, &[], &[]).await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        r.stdout.trim(),
        "replaced reeve build aaaaaaa with the server's bbbbbbb"
    );
    assert_eq!(
        names(&f),
        ["VERSION", "SHA256SUMS", this_target(), "VERSION"],
        "this target's file and nothing else"
    );
    assert_eq!(
        std::fs::read(&exe).unwrap(),
        served(this_target()),
        "replaced byte for byte with this target's file"
    );
    assert!(
        f.seen.lock().unwrap().iter().all(|(_, creds)| !creds),
        "no Authorization or Cookie, ever"
    );
    assert!(
        !r.stderr.contains("run reeve update"),
        "update never ends with its own notice: {}",
        r.stderr
    );
    no_temporary_file(dir.path());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&exe).unwrap().permissions().mode() & 0o777,
            0o755,
            "executable, writable by its owner only"
        );
    }
    let v = std::process::Command::new(&exe)
        .arg("--version")
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&v.stdout).starts_with("reeve-planner"),
        "the new binary runs"
    );
}

#[tokio::test]
async fn an_older_offered_build_is_installed_too() {
    // B running, A offered: no order between builds, only equality (kb/ADR-0030 R7).
    let f = Arc::new(fake(A));
    let dir = tmp();
    let exe = installed(dir.path());
    let r = update(&exe, &serve(f).await, B, &[], &["--json"]).await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        envelope(&r),
        serde_json::json!({"outcome":"ok","value":{"from":B,"to":A,"updated":true}})
    );
    assert_eq!(std::fs::read(&exe).unwrap(), served(this_target()));
}

#[tokio::test]
async fn equal_builds_replace_nothing() {
    let f = Arc::new(fake(A));
    let dir = tmp();
    let exe = installed(dir.path());
    let r = update(&exe, &serve(f.clone()).await, A, &[], &[]).await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout.trim(), "reeve is up to date (build aaaaaaa)");
    assert_eq!(names(&f), ["VERSION"]);
    assert_eq!(std::fs::read(&exe).unwrap(), real());
}

#[tokio::test]
async fn an_unstamped_build_updates_from_nothing() {
    let f = Arc::new(fake(B));
    let dir = tmp();
    let exe = installed(dir.path());
    let r = update(&exe, &serve(f).await, "dev", &[], &[]).await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        r.stdout.trim(),
        "replaced reeve dev build with the server's bbbbbbb"
    );
    assert_eq!(std::fs::read(&exe).unwrap(), served(this_target()));
}

#[tokio::test]
async fn a_checksum_mismatch_changes_nothing() {
    let mut fk = fake(B);
    // Every line well formed, but this target's names other bytes: also what a deploy between the requests looks like.
    fk.sums = Some(sums().replace(
        &hex::encode(Sha256::digest(served(this_target()))),
        &hex::encode(Sha256::digest(b"something else")),
    ));
    let f = Arc::new(fk);
    let dir = tmp();
    let exe = installed(dir.path());
    let r = update(&exe, &serve(f.clone()).await, A, &[], &["--json"]).await;
    assert_eq!(r.code, 1);
    assert_eq!(envelope(&r)["reason"], "checksum_mismatch");
    no_credential_sent(&f);
    assert!(
        r.stderr
            .contains("Try again; if it repeats, tell whoever runs the server"),
        "{}",
        r.stderr
    );
    assert_eq!(std::fs::read(&exe).unwrap(), real());
    no_temporary_file(dir.path());
}

/// On Unix nothing is moved before the fault (the swap is one `rename(2)`), so this pins that a failed swap leaves the
/// original and no temporary file. The rollback itself is covered by `swap_aside`'s unit tests and end to end on the
/// Windows leg of `.github/workflows/cli.yml`, which arrives with PR B.
#[tokio::test]
async fn a_failure_after_the_original_moved_puts_it_back() {
    let f = Arc::new(fake(B));
    let dir = tmp();
    let exe = installed(dir.path());
    let r = update(
        &exe,
        &serve(f).await,
        A,
        &[("REEVE_UPDATE_FAULT", "after_move")],
        &["--json"],
    )
    .await;
    assert_eq!(r.code, 1, "{}", r.stdout);
    assert_eq!(envelope(&r)["reason"], "cannot_replace");
    assert!(
        r.stderr.contains("injected fault at after_move"),
        "{}",
        r.stderr
    );
    assert_eq!(
        std::fs::read(&exe).unwrap(),
        real(),
        "the original, at its own path"
    );
    let left: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        left.len(),
        1,
        "nothing aside and no temporary file: {left:?}"
    );
}

#[tokio::test]
async fn a_redirect_is_refused() {
    let mut fk = fake(B);
    fk.redirect = true;
    let f = Arc::new(fk);
    let dir = tmp();
    let exe = installed(dir.path());
    let r = update(&exe, &serve(f.clone()).await, A, &[], &["--json"]).await;
    assert_eq!(r.code, 1);
    let v = envelope(&r);
    assert_eq!(v["code"], "validation");
    assert!(
        v.get("reason").is_none(),
        "no reason: an agent does nothing different here"
    );
    assert!(
        r.stderr
            .contains("redirected /cli/VERSION to https://elsewhere.example"),
        "{}",
        r.stderr
    );
    assert_eq!(names(&f), ["VERSION"], "the redirect was not followed");
    no_credential_sent(&f);
}

#[tokio::test]
async fn a_url_with_credentials_is_refused_before_any_request() {
    let f = Arc::new(fake(B));
    let url = serve(f.clone())
        .await
        .replace("http://", "http://user:secret@");
    let dir = tmp();
    let exe = installed(dir.path());
    let r = update(&exe, &url, A, &[], &["--json"]).await;
    assert_eq!(r.code, 1);
    assert_eq!(envelope(&r)["code"], "validation");
    assert!(
        f.seen.lock().unwrap().is_empty(),
        "no request, so no Basic auth: {:?}",
        names(&f)
    );
    assert!(
        !r.stderr.contains("secret") && !r.stdout.contains("secret"),
        "the password is never echoed"
    );
}

#[tokio::test]
async fn a_server_without_distribution_says_whom_to_ask() {
    for (label, fk) in [
        (
            "404",
            Fake {
                distributes: false,
                ..fake(B)
            },
        ),
        (
            "an older server's page",
            Fake {
                old_server: true,
                ..fake(B)
            },
        ),
    ] {
        let f = Arc::new(fk);
        let url = serve(f.clone()).await;
        let dir = tmp();
        let exe = installed(dir.path());
        let r = update(&exe, &url, A, &[], &["--json"]).await;
        assert_eq!(r.code, 1, "{label}");
        assert_eq!(envelope(&r)["reason"], "not_distributed", "{label}");
        assert!(
            r.stderr.contains(&format!(
                "this server ({url}) does not distribute the CLI; ask whoever runs it to enable it"
            )),
            "{label}: {}",
            r.stderr
        );
        assert_eq!(names(&f), ["VERSION"], "{label}: nothing else fetched");
        no_credential_sent(&f);
        assert_eq!(std::fs::read(&exe).unwrap(), real());
    }
}

#[tokio::test]
async fn plain_http_off_loopback_is_refused_before_any_request() {
    let dir = tmp();
    let exe = installed(dir.path());
    let r = update(&exe, "http://reeve.example.invalid", A, &[], &["--json"]).await;
    assert_eq!(r.code, 1);
    assert_eq!(envelope(&r)["code"], "validation");
    assert!(r.stderr.contains("must be https"), "{}", r.stderr);
}

#[tokio::test]
async fn an_unreachable_server_fails_and_changes_nothing() {
    // Port 1 (tcpmux): nothing listens there, and unlike a freed ephemeral port no parallel test's server can take it.
    let port = 1;
    let dir = tmp();
    let exe = installed(dir.path());
    let r = update(
        &exe,
        &format!("http://127.0.0.1:{port}"),
        A,
        &[],
        &["--json"],
    )
    .await;
    assert_eq!(r.code, 1);
    assert_eq!(envelope(&r)["code"], "internal");
    // The cause, not only the url: reqwest's own message says just "error sending request".
    assert!(
        r.stderr.to_lowercase().contains("refused"),
        "the cause is named: {}",
        r.stderr
    );
    assert_eq!(
        r.stderr.matches("127.0.0.1:1").count(),
        1,
        "the url once: {}",
        r.stderr
    );
    assert_eq!(std::fs::read(&exe).unwrap(), real());
}

#[tokio::test]
async fn a_server_error_says_to_try_again_and_changes_nothing() {
    let f = Arc::new(Fake {
        sums_status: Some(StatusCode::BAD_GATEWAY),
        ..fake(B)
    });
    let dir = tmp();
    let exe = installed(dir.path());
    let r = update(&exe, &serve(f).await, A, &[], &["--json"]).await;
    assert_eq!(r.code, 1, "{}", r.stdout);
    let v = envelope(&r);
    assert_eq!(v["code"], "internal");
    assert!(v.get("reason").is_none(), "no reason: {v}");
    assert!(
        r.stderr.contains("HTTP 502") && r.stderr.contains("run reeve update again"),
        "{}",
        r.stderr
    );
    assert_eq!(std::fs::read(&exe).unwrap(), real());
}

#[cfg(unix)]
#[tokio::test]
async fn an_unwritable_directory_changes_nothing_and_says_what_to_do() {
    use std::os::unix::fs::PermissionsExt;
    let uid = std::process::Command::new("id").arg("-u").output().unwrap();
    assert_ne!(
        String::from_utf8_lossy(&uid.stdout).trim(),
        "0",
        "run as a non-root user: root can write anywhere, so this test would show nothing"
    );
    let f = Arc::new(fake(B));
    let dir = tmp();
    let exe = installed(dir.path());
    let before = std::fs::metadata(dir.path()).unwrap().permissions().mode();
    /// Gives the directory its mode back however the test ends, so a failure leaves nothing `TempDir` cannot delete.
    struct Writable<'a>(&'a Path);
    impl Drop for Writable<'_> {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(self.0, std::fs::Permissions::from_mode(0o755));
        }
    }
    let _restore = Writable(dir.path());
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    assert_ne!(
        std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
        before & 0o777,
        "the mode really changed"
    );
    let r = update(&exe, &serve(f).await, A, &[], &["--json"]).await;
    assert_eq!(r.code, 1);
    assert_eq!(envelope(&r)["reason"], "cannot_replace");
    assert!(
        r.stderr.contains(&format!(
            "cannot replace {}",
            exe.canonicalize().unwrap().display()
        )),
        "{}",
        r.stderr
    );
    let canonical_dir = dir.path().canonicalize().unwrap();
    assert!(
        r.stderr.contains(&format!(
            "its directory ({}) is not writable by you",
            canonical_dir.display()
        )) && r.stderr.contains("/cli/install.sh"),
        "the directory, not the file, is named as the obstacle: {}",
        r.stderr
    );
    assert_eq!(std::fs::read(&exe).unwrap(), real());
}

#[tokio::test]
async fn a_stale_temporary_name_is_neither_deleted_nor_a_failure() {
    // A file already sits at the first temporary name (`<exe>.tmp`): the update takes the next name and leaves it alone.
    let f = Arc::new(fake(B));
    let dir = tmp();
    let exe = installed(dir.path());
    let mut stale = exe.as_os_str().to_owned();
    stale.push(".tmp");
    std::fs::write(&stale, "someone else's").unwrap();
    let r = update(&exe, &serve(f).await, A, &[], &[]).await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(std::fs::read_to_string(&stale).unwrap(), "someone else's");
}

/// A process still running build A keeps its image; two updates in a row must both succeed (Windows can neither delete
/// nor reuse a running image's backup name). The same sequence must work on Unix too.
#[tokio::test]
async fn a_running_old_build_does_not_block_the_next_update() {
    let dir = tmp();
    let exe = installed(dir.path());
    let cfg = tmp();
    // `reeve auth login --no-browser` waits on stdin: a long-lived process running this exact file.
    let mut held = tokio::process::Command::new(&exe);
    isolated(&mut held)
        .args([
            "auth",
            "login",
            "--no-browser",
            "--url",
            "http://127.0.0.1:1",
        ])
        .env("REEVE_CONFIG_DIR", cfg.path())
        .env_remove("REEVE_TOKEN")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    let mut held = held.spawn().unwrap();
    {
        // Running once it has printed its prompt.
        use tokio::io::AsyncReadExt;
        let mut err = held.stderr.take().unwrap();
        let mut buf = [0u8; 64];
        let n = tokio::time::timeout(std::time::Duration::from_secs(10), err.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert!(n > 0, "the held process started");
    }
    let r = update(&exe, &serve(Arc::new(fake(B))).await, A, &[], &[]).await;
    assert_eq!(r.code, 0, "first update: {}", r.stderr);
    let r = update(&exe, &serve(Arc::new(fake(C))).await, B, &[], &[]).await;
    assert_eq!(
        r.code, 0,
        "second update while build A still runs: {}",
        r.stderr
    );
    held.kill().await.unwrap();
}

#[tokio::test]
async fn no_server_url_says_how_to_set_one() {
    let dir = tmp();
    let exe = installed(dir.path());
    let cfg = tmp();
    let out = isolated(&mut tokio::process::Command::new(&exe))
        .args(["update", "--json"])
        .env_remove("REEVE_URL")
        .env("REEVE_CONFIG_DIR", cfg.path())
        .output()
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["code"], "validation");
    assert!(String::from_utf8_lossy(&out.stderr)
        .contains("no server url: run reeve auth login --url https://reeve.example.com first, or set REEVE_URL"));
}

#[tokio::test]
async fn a_build_that_changes_while_updating_is_a_checksum_mismatch() {
    let mut fk = fake(B);
    fk.later_version = Some(C.into());
    let f = Arc::new(fk);
    let dir = tmp();
    let exe = installed(dir.path());
    let r = update(&exe, &serve(f.clone()).await, A, &[], &["--json"]).await;
    assert_eq!(r.code, 1, "{}", r.stdout);
    assert_eq!(envelope(&r)["reason"], "checksum_mismatch");
    assert!(
        r.stderr.contains("changed while updating") && r.stderr.contains("Try again"),
        "{}",
        r.stderr
    );
    assert_eq!(
        names(&f),
        ["VERSION", "SHA256SUMS", this_target(), "VERSION"]
    );
    assert_eq!(std::fs::read(&exe).unwrap(), real());
    no_temporary_file(dir.path());
}

#[tokio::test]
async fn an_over_long_text_answer_is_refused() {
    // 64 KiB is the cap. The body starts with a real answer, so only the cap can refuse it. With a `Content-Length`
    // the precheck refuses it; streamed without one, the read loop does (delete either check and its case fails:
    // VERSION would be taken, SHA256SUMS would parse as a sums file with no line, a `checksum_mismatch`).
    for stream in [false, true] {
        let f = Arc::new(Fake {
            huge: Some(("VERSION", 70_000, stream)),
            ..fake(B)
        });
        let dir = tmp();
        let exe = installed(dir.path());
        let r = update(&exe, &serve(f.clone()).await, A, &[], &["--json"]).await;
        assert_eq!(r.code, 1, "stream={stream}");
        assert_eq!(envelope(&r)["reason"], "not_distributed", "stream={stream}");
        assert_eq!(names(&f), ["VERSION"], "stream={stream}");
        assert_eq!(std::fs::read(&exe).unwrap(), real());

        let f = Arc::new(Fake {
            huge: Some(("SHA256SUMS", 70_000, stream)),
            ..fake(B)
        });
        let r = update(&exe, &serve(f.clone()).await, A, &[], &["--json"]).await;
        assert_eq!(r.code, 1, "stream={stream}");
        let v = envelope(&r);
        assert_eq!(v["code"], "internal", "stream={stream}");
        assert!(v.get("reason").is_none(), "stream={stream}");
        assert_eq!(
            names(&f),
            ["VERSION", "SHA256SUMS"],
            "the binary is never fetched (stream={stream})"
        );
        assert_eq!(std::fs::read(&exe).unwrap(), real());
    }
}

#[tokio::test]
async fn an_answer_naming_another_build_is_a_checksum_mismatch() {
    let f = Arc::new(Fake {
        stamp_files: Some(C.into()),
        ..fake(B)
    });
    let dir = tmp();
    let exe = installed(dir.path());
    let r = update(&exe, &serve(f).await, A, &[], &["--json"]).await;
    assert_eq!(r.code, 1, "{}", r.stdout);
    assert_eq!(envelope(&r)["reason"], "checksum_mismatch");
    assert!(
        r.stderr.contains("changed while updating") && r.stderr.contains("Try again"),
        "{}",
        r.stderr
    );
    assert_eq!(std::fs::read(&exe).unwrap(), real());
    no_temporary_file(dir.path());
}

#[tokio::test]
async fn a_garbled_build_on_an_answer_is_ignored() {
    // As `Client` ignores it: only a valid build that differs means the offered build changed.
    let f = Arc::new(Fake {
        stamp_files: Some("latest".into()),
        ..fake(B)
    });
    let dir = tmp();
    let exe = installed(dir.path());
    let r = update(&exe, &serve(f).await, A, &[], &[]).await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(std::fs::read(&exe).unwrap(), served(this_target()));
}

#[tokio::test]
async fn a_proxy_in_the_environment_is_never_used_for_loopback_http() {
    let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let purl = format!("http://{}", proxy.local_addr().unwrap());
    let hit = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let h = hit.clone();
    tokio::spawn(async move {
        if proxy.accept().await.is_ok() {
            h.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    });
    let f = Arc::new(fake(B));
    let dir = tmp();
    let exe = installed(dir.path());
    let env = [
        ("HTTP_PROXY", purl.as_str()),
        ("http_proxy", purl.as_str()),
        ("ALL_PROXY", purl.as_str()),
        ("all_proxy", purl.as_str()),
    ];
    let r = update(&exe, &serve(f).await, A, &env, &[]).await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(std::fs::read(&exe).unwrap(), served(this_target()));
    assert!(
        !hit.load(std::sync::atomic::Ordering::SeqCst),
        "the proxy was contacted"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlinked_executable_has_its_target_replaced_and_the_link_kept() {
    let dir = tmp();
    let real_dir = dir.path().join("store");
    std::fs::create_dir(&real_dir).unwrap();
    let target = installed(&real_dir);
    // On Linux `/proc/self/exe` already resolves the link, so this passes even without `canonicalize()`; that call is
    // pinned only where `current_exe` returns the link itself (macOS).
    let link = dir.path().join("reeve");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let r = update(&link, &serve(Arc::new(fake(B))).await, A, &[], &[]).await;
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link is still a link"
    );
    assert_eq!(
        std::fs::read(&target).unwrap(),
        served(this_target()),
        "its target was replaced"
    );
    no_temporary_file(&real_dir);
}
