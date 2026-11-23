//! CLI distribution (kb/ADR-0030): the bundle the image carries in `REEVE_CLI_DIR` (default `/srv/cli`), loaded once
//! at startup, served at `/cli/{name}` (`routes/cli_dist.rs`) without sign-in, and the `X-Reeve-Cli` header on every
//! response. File serving in the composition root, not a context (kb/ADR-0010 R1). The wire contract (the header
//! name, the bundle's file names, the CLI-version format) is `reeve_protocol`'s.
use crate::config::ServerConfig;
use axum::{
    http::{HeaderName, HeaderValue, header},
    response::Response,
};
use reeve_protocol::{CLI_BINARIES, CLI_VERSION_HEADER, SUMS_FILE, VERSION_FILE, is_cli_version};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// `reeve_protocol::CLI_VERSION_HEADER` as a header name (lower case, so `from_static` accepts it).
pub const CLI_HEADER: HeaderName = HeaderName::from_static(CLI_VERSION_HEADER);
/// What the server replaces, once, in each install script: the quoted URL, quotes included.
pub const URL_PLACEHOLDER: &str = "'@REEVE_URL@'";
/// The most `VERSION` or `SHA256SUMS` may hold; a bundle's are well under 1 KiB. Checked before either is read.
pub const TEXT_FILE_LIMIT: u64 = 64 * 1024;
const INSTALL_SH: &str = include_str!("../../../deploy/cli/install.sh");
const INSTALL_PS1: &str = include_str!("../../../deploy/cli/install.ps1");

/// A loaded, valid bundle.
pub struct CliDist {
    version: String,
    header: HeaderValue,
    version_file: String,
    sums: String,
    dir: PathBuf,
    install_sh: String,
    install_ps1: String,
}

/// What one name under `/cli/` serves.
#[derive(Debug, PartialEq)]
pub enum Entry<'a> {
    Text(&'a str),
    File(PathBuf),
}

impl CliDist {
    /// The bundle in `dir`, or why distribution is off. `public_url` is the explicitly configured URL
    /// (`ServerConfig::explicit_public_url`): `None` turns distribution off, and so does a host the install scripts
    /// could not carry safely. Reads and hashes every binary: call it off the async runtime (`startup` does).
    pub fn load(dir: &Path, public_url: Option<&url::Url>) -> Result<Self, String> {
        let url = public_url.ok_or("no public_url configured")?;
        check_host(url)?;
        let version_file = read_text(&dir.join(VERSION_FILE))
            .map_err(|e| format!("no bundle in {} ({VERSION_FILE}: {e})", dir.display()))?;
        let version = version_file.strip_suffix('\n').unwrap_or(&version_file);
        if !is_cli_version(version) {
            return Err(format!(
                "{VERSION_FILE} is not a 40-character lowercase commit id"
            ));
        }
        let sums =
            read_text(&dir.join(SUMS_FILE)).map_err(|e| format!("{SUMS_FILE} unreadable ({e})"))?;
        // `lines()` would silently accept CRLF; a bundle CI built has LF only, and the CLI checks the lines as served.
        if sums.contains('\r') {
            return Err(format!(
                "{SUMS_FILE} has a carriage return (CRLF line endings)"
            ));
        }
        let mut listed: Vec<(&str, &str)> = Vec::new();
        for line in sums.lines() {
            let (hash, name) = line
                .split_once("  ")
                .ok_or_else(|| format!("{SUMS_FILE}: malformed line {line:?}"))?;
            if hash.len() != 64 || !hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                return Err(format!("{SUMS_FILE}: malformed line {line:?}"));
            }
            if !CLI_BINARIES.contains(&name) {
                return Err(format!("{SUMS_FILE} lists an unknown file {name:?}"));
            }
            if listed.iter().any(|(_, n)| *n == name) {
                return Err(format!("{SUMS_FILE} lists {name} twice"));
            }
            listed.push((hash, name));
        }
        if let Some(missing) = CLI_BINARIES
            .into_iter()
            .find(|n| !listed.iter().any(|(_, l)| l == n))
        {
            return Err(format!("{SUMS_FILE} does not list {missing}"));
        }
        for (hash, name) in listed {
            let path = dir.join(name);
            // symlink_metadata: a symlink is not a regular file, and the image's COPY makes regular files.
            let meta =
                std::fs::symlink_metadata(&path).map_err(|e| format!("{name} is missing ({e})"))?;
            if !meta.is_file() {
                return Err(format!("{name} is not a regular file"));
            }
            if meta.len() == 0 {
                return Err(format!("{name} is empty"));
            }
            let actual = sha256_of(&path).map_err(|e| format!("{name} unreadable ({e})"))?;
            if actual != hash {
                return Err(format!("{name} does not match its {SUMS_FILE} line"));
            }
        }
        Ok(Self {
            header: HeaderValue::from_str(version).expect("hex is a valid header value"),
            version: version.to_owned(),
            version_file: format!("{version}\n"),
            sums,
            dir: dir.to_owned(),
            install_sh: render_install_sh(url),
            install_ps1: render_install_ps1(url),
        })
    }
    /// The build the server offers, in full (40 lowercase hex characters).
    pub fn version(&self) -> &str {
        &self.version
    }
    /// The build as people see it: the first seven characters of the CLI version.
    pub fn build(&self) -> &str {
        &self.version[..7]
    }
    /// What `name` serves: `VERSION`, `SHA256SUMS`, one of `CLI_BINARIES`, or an install script; `None` otherwise.
    pub fn entry(&self, name: &str) -> Option<Entry<'_>> {
        match name {
            VERSION_FILE => Some(Entry::Text(&self.version_file)),
            SUMS_FILE => Some(Entry::Text(&self.sums)),
            "install.sh" => Some(Entry::Text(&self.install_sh)),
            "install.ps1" => Some(Entry::Text(&self.install_ps1)),
            // Joined from the constant, never from the request: only the bundle's own paths are ever opened.
            _ => CLI_BINARIES
                .into_iter()
                .find(|n| *n == name)
                .map(|n| Entry::File(self.dir.join(n))),
        }
    }
}

/// A host the scripts can carry: a DNS name of letters, digits, hyphens and dots (the `url` crate has already
/// lower-cased it and turned an international name into its `xn--` form), or an IP literal. Anything else (an
/// underscore, a percent-encoded byte in an opaque host) is refused rather than quoted.
fn check_host(url: &url::Url) -> Result<(), String> {
    match url.host() {
        Some(url::Host::Ipv4(_) | url::Host::Ipv6(_)) => Ok(()),
        Some(url::Host::Domain(d))
            if !d.is_empty()
                && d.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.') =>
        {
            Ok(())
        }
        Some(url::Host::Domain(d)) => Err(format!(
            "the public_url host {d:?} is not letters, digits, hyphens and dots, or an IP address"
        )),
        None => Err("the public_url has no host".into()),
    }
}

/// A bundle text file: a regular file (not a symlink) of at most `TEXT_FILE_LIMIT` bytes, checked before it is read.
fn read_text(path: &Path) -> Result<String, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a regular file".into());
    }
    if meta.len() > TEXT_FILE_LIMIT {
        return Err(format!("larger than {TEXT_FILE_LIMIT} bytes"));
    }
    let mut text = String::new();
    // The limit holds even if the file grew between the check and the read.
    std::fs::File::open(path)
        .and_then(|f| f.take(TEXT_FILE_LIMIT + 1).read_to_string(&mut text))
        .map_err(|e| e.to_string())?;
    if text.len() as u64 > TEXT_FILE_LIMIT {
        return Err(format!("larger than {TEXT_FILE_LIMIT} bytes"));
    }
    Ok(text)
}

/// The file's SHA-256 in lowercase hex, read in chunks (a binary is tens of megabytes).
fn sha256_of(path: &Path) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(path)?, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

/// The configured public URL as the scripts carry it: without a trailing `/`.
fn origin(url: &url::Url) -> &str {
    url.as_str().trim_end_matches('/')
}

/// POSIX sh: inside single quotes nothing expands; a `'` is closed, escaped and reopened.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// PowerShell: inside single quotes nothing expands; a quote is doubled. PowerShell takes U+2018–U+201B as single
/// quotes too, so each of those is doubled as well.
fn ps_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if matches!(c, '\'' | '\u{2018}'..='\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// `install.sh` with the public URL quoted for POSIX sh.
pub(crate) fn render_install_sh(url: &url::Url) -> String {
    INSTALL_SH.replacen(URL_PLACEHOLDER, &sh_quote(origin(url)), 1)
}
/// `install.ps1` with the public URL quoted for PowerShell.
pub(crate) fn render_install_ps1(url: &url::Url) -> String {
    INSTALL_PS1.replacen(URL_PLACEHOLDER, &ps_quote(origin(url)), 1)
}

/// The one startup line, either way (kb/ADR-0030 R4: the log says why it is off).
pub fn status_line(r: &Result<CliDist, String>) -> String {
    match r {
        Ok(c) => format!("cli distribution: on, offering build {}", c.build()),
        Err(why) => format!("cli distribution: off ({why})"),
    }
}

/// Whether the startup line is a warning: distribution is off although `dir` holds a `VERSION` and a public URL is
/// configured, so a bundle was meant to be served and is broken. Off for want of a bundle or of a URL is `info`.
pub fn is_fault(r: &Result<CliDist, String>, dir: &Path, public_url: Option<&url::Url>) -> bool {
    r.is_err() && public_url.is_some() && std::fs::symlink_metadata(dir.join(VERSION_FILE)).is_ok()
}

/// `REEVE_CLI_DIR` when set and non-empty, else `/srv/cli` (where the published image puts the bundle).
pub fn cli_dir(env: Option<std::ffi::OsString>) -> PathBuf {
    env.filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/srv/cli"))
}

/// At startup (`main.rs`'s `serve`, before it listens): load and hash the bundle in `dir` on a blocking thread, log one
/// line, keep it only when distribution is on.
pub async fn startup(config: &ServerConfig, dir: PathBuf) -> Option<Arc<CliDist>> {
    let url = config.explicit_public_url();
    let (r, fault) = tokio::task::spawn_blocking(move || {
        let r = CliDist::load(&dir, url.as_ref());
        let fault = is_fault(&r, &dir, url.as_ref());
        (r, fault)
    })
    .await
    .unwrap_or_else(|e| (Err(format!("loading the bundle failed ({e})")), true));
    if fault {
        tracing::warn!("{}", status_line(&r));
    } else {
        tracing::info!("{}", status_line(&r));
    }
    r.ok().map(Arc::new)
}

/// The headers every `/cli/` answer carries, success or failure (kb/ADR-0030 R3).
pub fn harden(mut res: Response) -> Response {
    let h = res.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    res
}

/// `router` with `X-Reeve-Cli` on every response while distribution is on (`cli` is `Some`), and unchanged while it is
/// off. The header value is built once here; the layer sets a header only and never reads or wraps the body, so
/// streamed responses (the event stream, the binaries) pass through untouched.
pub fn stamp(router: axum::Router, cli: Option<&CliDist>) -> axum::Router {
    let Some(cli) = cli else { return router };
    let value = cli.header.clone();
    router.layer(axum::middleware::map_response(move |mut res: Response| {
        let value = value.clone();
        async move {
            res.headers_mut().insert(CLI_HEADER, value);
            res
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION: &str = "0123456789abcdef0123456789abcdef01234567";

    /// A valid bundle: every file of `CLI_BINARIES` with distinct contents, their real checksums, the version.
    fn bundle(dir: &Path) {
        let mut sums = String::new();
        for name in CLI_BINARIES {
            let bytes = format!("binary {name}");
            std::fs::write(dir.join(name), &bytes).unwrap();
            sums.push_str(&format!(
                "{}  {name}\n",
                hex::encode(Sha256::digest(&bytes))
            ));
        }
        std::fs::write(dir.join(SUMS_FILE), sums).unwrap();
        std::fs::write(dir.join(VERSION_FILE), format!("{VERSION}\n")).unwrap();
    }

    fn url() -> url::Url {
        url::Url::parse("https://reeve.example.com").unwrap()
    }

    fn edit_sums(d: &Path, f: impl Fn(&str) -> String) {
        let s = std::fs::read_to_string(d.join(SUMS_FILE)).unwrap();
        std::fs::write(d.join(SUMS_FILE), f(&s)).unwrap();
    }

    /// `name` in `d` replaced by a symlink to a copy of itself elsewhere in `d`: same contents, not a regular file.
    #[cfg(unix)]
    fn symlink_in_place(d: &Path, name: &str) {
        let real = d.join(format!("{name}.real"));
        std::fs::rename(d.join(name), &real).unwrap();
        std::os::unix::fs::symlink(&real, d.join(name)).unwrap();
    }

    #[test]
    fn a_valid_bundle_loads() {
        let d = tempfile::tempdir().unwrap();
        bundle(d.path());
        let c = CliDist::load(d.path(), Some(&url())).unwrap();
        assert_eq!(c.version(), VERSION);
        assert_eq!(c.build(), "0123456");
        assert_eq!(
            status_line(&Ok(c)),
            "cli distribution: on, offering build 0123456"
        );
    }

    /// Every reason distribution is off, each from a valid bundle with exactly one thing broken, and the startup line
    /// that names it (kb/ADR-0030 R4). Distinguishes a lenient loader, and a line that drops the reason.
    #[test]
    fn each_reason_for_off_is_named() {
        type Break = fn(&Path);
        let mut cases: Vec<(&str, Break, &str)> = vec![
            (
                "no VERSION",
                |d| std::fs::remove_file(d.join(VERSION_FILE)).unwrap(),
                "no bundle in",
            ),
            (
                "upper-case VERSION",
                |d| std::fs::write(d.join(VERSION_FILE), VERSION.to_uppercase()).unwrap(),
                "40-character",
            ),
            (
                "short VERSION",
                |d| std::fs::write(d.join(VERSION_FILE), &VERSION[..39]).unwrap(),
                "40-character",
            ),
            (
                "two newlines",
                |d| std::fs::write(d.join(VERSION_FILE), format!("{VERSION}\n\n")).unwrap(),
                "40-character",
            ),
            (
                "VERSION is a directory",
                |d| {
                    std::fs::remove_file(d.join(VERSION_FILE)).unwrap();
                    std::fs::create_dir(d.join(VERSION_FILE)).unwrap()
                },
                "not a regular file",
            ),
            (
                "VERSION over the limit",
                |d| {
                    let big = "0".repeat(TEXT_FILE_LIMIT as usize + 1);
                    std::fs::write(d.join(VERSION_FILE), big).unwrap()
                },
                "larger than 65536 bytes",
            ),
            (
                "no SHA256SUMS",
                |d| std::fs::remove_file(d.join(SUMS_FILE)).unwrap(),
                "SHA256SUMS unreadable",
            ),
            (
                "SHA256SUMS over the limit",
                |d| {
                    edit_sums(d, |s| {
                        format!("{s}{}", "\n".repeat(TEXT_FILE_LIMIT as usize))
                    })
                },
                "SHA256SUMS unreadable (larger than 65536 bytes)",
            ),
            (
                "CRLF line endings",
                |d| edit_sums(d, |s| s.replace('\n', "\r\n")),
                "carriage return",
            ),
            (
                "a name missing",
                |d| {
                    edit_sums(d, |s| {
                        let n = s.lines().count();
                        s.lines().take(n - 1).map(|l| format!("{l}\n")).collect()
                    })
                },
                "does not list",
            ),
            (
                "an unknown name",
                |d| edit_sums(d, |s| format!("{s}{}  extra\n", "0".repeat(64))),
                "unknown file \"extra\"",
            ),
            (
                "a name twice",
                |d| {
                    edit_sums(d, |s| {
                        let first = s.lines().next().unwrap().to_string();
                        format!("{s}{first}\n")
                    })
                },
                "twice",
            ),
            (
                "one space",
                |d| edit_sums(d, |s| s.replacen("  ", " ", 1)),
                "malformed",
            ),
            (
                "a listed file missing",
                |d| std::fs::remove_file(d.join(CLI_BINARIES[0])).unwrap(),
                "is missing",
            ),
            (
                "a listed name is a directory",
                |d| {
                    std::fs::remove_file(d.join(CLI_BINARIES[0])).unwrap();
                    std::fs::create_dir(d.join(CLI_BINARIES[0])).unwrap()
                },
                "is not a regular file",
            ),
            (
                "a binary changed after its checksum",
                |d| std::fs::write(d.join(CLI_BINARIES[2]), "binary tampered").unwrap(),
                "reeve-macos-aarch64 does not match its SHA256SUMS line",
            ),
            (
                "an empty binary, listed with its real checksum",
                |d| {
                    std::fs::write(d.join(CLI_BINARIES[1]), "").unwrap();
                    let empty = hex::encode(Sha256::digest(b""));
                    edit_sums(d, |s| {
                        s.lines()
                            .map(|l| match l.split_once("  ") {
                                Some((_, n)) if n == CLI_BINARIES[1] => format!("{empty}  {n}\n"),
                                _ => format!("{l}\n"),
                            })
                            .collect()
                    })
                },
                "reeve-linux-aarch64 is empty",
            ),
        ];
        #[cfg(unix)]
        cases.extend([
            (
                "a symlinked binary",
                (|d| symlink_in_place(d, CLI_BINARIES[0])) as Break,
                "reeve-linux-x86_64 is not a regular file",
            ),
            (
                "a symlinked VERSION",
                |d| symlink_in_place(d, VERSION_FILE),
                "VERSION: not a regular file",
            ),
            (
                "a symlinked SHA256SUMS",
                |d| symlink_in_place(d, SUMS_FILE),
                "SHA256SUMS unreadable (not a regular file)",
            ),
        ]);
        for (what, brk, want) in cases {
            let d = tempfile::tempdir().unwrap();
            bundle(d.path());
            brk(d.path());
            let r = CliDist::load(d.path(), Some(&url()));
            let why = r.as_ref().err().unwrap_or_else(|| panic!("{what}: loaded"));
            assert!(why.contains(want), "{what}: {why}");
            assert_eq!(
                status_line(&r),
                format!("cli distribution: off ({why})"),
                "{what}"
            );
        }
        let d = tempfile::tempdir().unwrap();
        bundle(d.path());
        let r = CliDist::load(d.path(), None);
        assert_eq!(
            r.as_ref().err().map(String::as_str),
            Some("no public_url configured")
        );
        assert_eq!(
            status_line(&r),
            "cli distribution: off (no public_url configured)"
        );
    }

    /// Only a host of letters, digits, hyphens and dots, or an IP literal, turns distribution on; the `url` crate's
    /// normalisation (lower case, `xn--`) comes first, so an international name is accepted in its ASCII form.
    #[test]
    fn the_public_url_host_must_be_ldh_or_an_ip() {
        let d = tempfile::tempdir().unwrap();
        bundle(d.path());
        for ok in [
            "https://reeve.example.com",
            "https://Reeve-1.Example.COM:8443/base/",
            "https://bücher.example/",
            "http://127.0.0.1:7474",
            "http://[::1]:7474",
        ] {
            let u = url::Url::parse(ok).unwrap();
            assert!(CliDist::load(d.path(), Some(&u)).is_ok(), "{ok}");
        }
        for bad in [
            "https://under_score.example.com",
            "https://a%27b.example.com",
        ] {
            let u = url::Url::parse(bad).unwrap();
            let why = CliDist::load(d.path(), Some(&u))
                .err()
                .unwrap_or_else(|| panic!("{bad}: loaded"));
            assert!(
                why.contains("is not letters, digits, hyphens and dots"),
                "{bad}: {why}"
            );
        }
    }

    /// `warn` only when a bundle was meant to be served (a `VERSION` exists, a public URL is configured) and failed;
    /// `info` when distribution is on, when there is no bundle and when there is no URL.
    #[test]
    fn a_broken_bundle_is_a_warning() {
        let d = tempfile::tempdir().unwrap();
        bundle(d.path());
        let u = url();
        let ok = CliDist::load(d.path(), Some(&u));
        assert!(!is_fault(&ok, d.path(), Some(&u)));
        let no_url = CliDist::load(d.path(), None);
        assert!(!is_fault(&no_url, d.path(), None));
        std::fs::write(d.path().join(CLI_BINARIES[0]), "tampered").unwrap();
        let broken = CliDist::load(d.path(), Some(&u));
        assert!(is_fault(&broken, d.path(), Some(&u)));
        let empty = tempfile::tempdir().unwrap();
        let none = CliDist::load(empty.path(), Some(&u));
        assert!(!is_fault(&none, empty.path(), Some(&u)));
    }

    /// The bundle's names and the two scripts, nothing else; a binary resolves to its own constant name in the bundle
    /// directory, never to anything built from the request.
    #[test]
    fn only_the_bundle_names_and_scripts_resolve() {
        let d = tempfile::tempdir().unwrap();
        bundle(d.path());
        std::fs::write(d.path().join("extra"), "x").unwrap();
        let c = CliDist::load(d.path(), Some(&url())).unwrap();
        assert_eq!(
            c.entry("VERSION"),
            Some(Entry::Text(&format!("{VERSION}\n")))
        );
        assert_eq!(
            c.entry("SHA256SUMS"),
            Some(Entry::Text(
                &std::fs::read_to_string(d.path().join(SUMS_FILE)).unwrap()
            ))
        );
        for name in CLI_BINARIES {
            assert_eq!(
                c.entry(name),
                Some(Entry::File(d.path().join(name))),
                "{name}"
            );
        }
        assert!(
            matches!(c.entry("install.sh"), Some(Entry::Text(t)) if t.contains("REEVE_URL='https://reeve.example.com'"))
        );
        assert!(
            matches!(c.entry("install.ps1"), Some(Entry::Text(t)) if t.contains("$ReeveUrl = 'https://reeve.example.com'"))
        );
        for bad in [
            "",
            "extra",
            "reeve",
            "../VERSION",
            "./VERSION",
            "VERSION/",
            "VERSION.bak",
            "install.SH",
            "Version",
        ] {
            assert_eq!(c.entry(bad), None, "{bad:?}");
        }
    }

    /// Each template holds the placeholder exactly once, so rendering replaces the whole quoted value; a URL whose path
    /// keeps a quote and a dollar survives each language's quoting (the sh line runs in a real `sh`); PowerShell's four
    /// typographic single quotes are doubled like `'`; the public URL's trailing slash is not carried into the scripts.
    #[test]
    fn the_scripts_are_rendered_and_quoted() {
        assert_eq!(INSTALL_SH.matches(URL_PLACEHOLDER).count(), 1);
        assert_eq!(INSTALL_PS1.matches(URL_PLACEHOLDER).count(), 1);
        let tricky = url::Url::parse("https://reeve.example.com/it's/$HOME/").unwrap();
        assert_eq!(origin(&tricky), "https://reeve.example.com/it's/$HOME");
        let sh = render_install_sh(&tricky);
        let line = sh.lines().find(|l| l.starts_with("REEVE_URL=")).unwrap();
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("{line}\nprintf %s \"$REEVE_URL\""))
            .output()
            .unwrap();
        assert!(out.status.success());
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "https://reeve.example.com/it's/$HOME"
        );
        assert!(
            render_install_ps1(&tricky)
                .contains("$ReeveUrl = 'https://reeve.example.com/it''s/$HOME'\n")
        );
        assert_eq!(
            ps_quote("a'b\u{2018}c\u{2019}d\u{201A}e\u{201B}f\u{201C}g"),
            "'a''b\u{2018}\u{2018}c\u{2019}\u{2019}d\u{201A}\u{201A}e\u{201B}\u{201B}f\u{201C}g'"
        );
        assert_eq!(sh_quote("a'b"), r"'a'\''b'");
        let d = tempfile::tempdir().unwrap();
        bundle(d.path());
        let c = CliDist::load(
            d.path(),
            Some(&url::Url::parse("http://127.0.0.1:7474/").unwrap()),
        )
        .unwrap();
        assert!(
            matches!(c.entry("install.sh"), Some(Entry::Text(t)) if t.contains("REEVE_URL='http://127.0.0.1:7474'\n"))
        );
    }

    #[test]
    fn the_directory_defaults_to_srv_cli() {
        assert_eq!(cli_dir(None), PathBuf::from("/srv/cli"));
        assert_eq!(cli_dir(Some("".into())), PathBuf::from("/srv/cli"));
        assert_eq!(
            cli_dir(Some("/opt/reeve-cli".into())),
            PathBuf::from("/opt/reeve-cli")
        );
    }
}
