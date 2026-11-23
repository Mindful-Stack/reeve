//! CLI distribution (kb/ADR-0030): the bundle the image carries in `REEVE_CLI_DIR` (default `/srv/cli`), loaded once
//! at startup, served at `/cli/{name}` (`routes/cli_dist.rs`) without sign-in, and the `X-Reeve-Cli` header on every
//! response. File serving in the composition root, not a context (kb/ADR-0010 R1). The wire contract (the header
//! name, the bundle's file names, the CLI-version format) is `reeve_protocol`'s.
use crate::{config::ServerConfig, state::AppState};
use axum::{
    extract::State,
    http::{HeaderName, HeaderValue, header},
    response::Response,
};
use reeve_protocol::{CLI_BINARIES, CLI_VERSION_HEADER, SUMS_FILE, VERSION_FILE, is_cli_version};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// `reeve_protocol::CLI_VERSION_HEADER` as a header name (lower case, so `from_static` accepts it).
pub const HEADER: HeaderName = HeaderName::from_static(CLI_VERSION_HEADER);
/// What the server replaces, once, in each install script: the quoted URL, quotes included.
pub const URL_PLACEHOLDER: &str = "'@REEVE_URL@'";
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
    /// (`ServerConfig::explicit_public_url`): `None` turns distribution off.
    pub fn load(dir: &Path, public_url: Option<&url::Url>) -> Result<Self, String> {
        let url = public_url.ok_or("no public_url configured")?;
        let version_file = std::fs::read_to_string(dir.join(VERSION_FILE))
            .map_err(|e| format!("no bundle in {} (VERSION: {e})", dir.display()))?;
        let version = version_file.strip_suffix('\n').unwrap_or(&version_file);
        if !is_cli_version(version) {
            return Err("VERSION is not a 40-character lowercase commit id".into());
        }
        let sums = std::fs::read_to_string(dir.join(SUMS_FILE))
            .map_err(|e| format!("SHA256SUMS unreadable ({e})"))?;
        let mut listed: Vec<&str> = Vec::new();
        for line in sums.lines() {
            let (hash, name) = line
                .split_once("  ")
                .ok_or_else(|| format!("SHA256SUMS: malformed line {line:?}"))?;
            if hash.len() != 64 || !hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                return Err(format!("SHA256SUMS: malformed line {line:?}"));
            }
            if !CLI_BINARIES.contains(&name) {
                return Err(format!("SHA256SUMS lists an unknown file {name:?}"));
            }
            if listed.contains(&name) {
                return Err(format!("SHA256SUMS lists {name} twice"));
            }
            listed.push(name);
        }
        if let Some(missing) = CLI_BINARIES.into_iter().find(|n| !listed.contains(n)) {
            return Err(format!("SHA256SUMS does not list {missing}"));
        }
        for name in CLI_BINARIES {
            // symlink_metadata: a symlink is not a regular file, and the image's COPY makes regular files.
            let meta = std::fs::symlink_metadata(dir.join(name))
                .map_err(|e| format!("{name} is missing ({e})"))?;
            if !meta.is_file() {
                return Err(format!("{name} is not a regular file"));
            }
        }
        let origin = url.as_str().trim_end_matches('/');
        Ok(Self {
            header: HeaderValue::from_str(version).expect("hex is a valid header value"),
            version: version.to_owned(),
            version_file: format!("{version}\n"),
            sums,
            dir: dir.to_owned(),
            install_sh: render_install_sh(origin),
            install_ps1: render_install_ps1(origin),
        })
    }
    /// The build the server offers, in full (40 lowercase hex characters).
    pub fn version(&self) -> &str {
        &self.version
    }
    /// The build as people see it: the first seven characters.
    pub fn short(&self) -> &str {
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

/// The install scripts with `origin` (the public URL without a trailing `/`) quoted for each language.
/// POSIX sh: inside single quotes nothing expands; a `'` is closed, escaped and reopened.
pub(crate) fn render_install_sh(origin: &str) -> String {
    INSTALL_SH.replacen(
        URL_PLACEHOLDER,
        &format!("'{}'", origin.replace('\'', r"'\''")),
        1,
    )
}
/// PowerShell: inside single quotes nothing expands; a `'` is doubled.
pub(crate) fn render_install_ps1(origin: &str) -> String {
    INSTALL_PS1.replacen(
        URL_PLACEHOLDER,
        &format!("'{}'", origin.replace('\'', "''")),
        1,
    )
}

/// The one startup line, either way (kb/ADR-0030 R4: the log says why it is off).
pub fn status_line(r: &Result<CliDist, String>) -> String {
    match r {
        Ok(c) => format!("cli distribution: on, offering build {}", c.short()),
        Err(why) => format!("cli distribution: off ({why})"),
    }
}

/// `REEVE_CLI_DIR` when set and non-empty, else `/srv/cli` (where the published image puts the bundle).
pub fn cli_dir(env: Option<std::ffi::OsString>) -> PathBuf {
    env.filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/srv/cli"))
}

/// At startup (`main.rs`'s `serve`): load the bundle in `dir`, log one line, keep it only when distribution is on.
pub fn startup(config: &ServerConfig, dir: &Path) -> Option<Arc<CliDist>> {
    let r = CliDist::load(dir, config.explicit_public_url().as_ref());
    tracing::info!("{}", status_line(&r));
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

/// The response layer: `X-Reeve-Cli` on every response while distribution is on. It sets a header only and never
/// reads or wraps the body, so streamed responses (the event stream, the binaries) pass through untouched.
pub async fn stamp(State(s): State<AppState>, mut res: Response) -> Response {
    if let Some(cli) = &s.cli_dist {
        res.headers_mut().insert(HEADER, cli.header.clone());
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

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

    #[test]
    fn a_valid_bundle_loads() {
        let d = tempfile::tempdir().unwrap();
        bundle(d.path());
        let c = CliDist::load(d.path(), Some(&url())).unwrap();
        assert_eq!(c.version(), VERSION);
        assert_eq!(c.short(), "0123456");
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
        let cases: [(&str, Break, &str); 11] = [
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
                "no SHA256SUMS",
                |d| std::fs::remove_file(d.join(SUMS_FILE)).unwrap(),
                "SHA256SUMS",
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
        ];
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

    /// Each template holds the placeholder exactly once, so rendering replaces the whole quoted value; a URL with a
    /// quote, a dollar and a backtick survives each language's quoting (the sh line runs in a real `sh`); the public
    /// URL's trailing slash is not carried into the scripts.
    #[test]
    fn the_scripts_are_rendered_and_quoted() {
        assert_eq!(INSTALL_SH.matches(URL_PLACEHOLDER).count(), 1);
        assert_eq!(INSTALL_PS1.matches(URL_PLACEHOLDER).count(), 1);
        let tricky = "https://reeve.example.com/it's $HOME `id`";
        let sh = render_install_sh(tricky);
        let line = sh.lines().find(|l| l.starts_with("REEVE_URL=")).unwrap();
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("{line}\nprintf %s \"$REEVE_URL\""))
            .output()
            .unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8(out.stdout).unwrap(), tricky);
        assert!(
            render_install_ps1(tricky)
                .contains("$ReeveUrl = 'https://reeve.example.com/it''s $HOME `id`'")
        );
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
