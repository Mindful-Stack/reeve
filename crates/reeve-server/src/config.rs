use serde::Deserialize;

/// A repository of the team's household. `name` is the allowlist a record reference must match and the clone's directory
/// under the household; `url` is what the scheduled check clones (`reeve-server repos`); `adr_dir` is instance
/// configuration kb/ADR-0016 R2 requires, read by nothing yet.
///
/// A record's browse link is `<base>/blob/HEAD/<path>`, GitHub's layout, so a host that lays out its web pages
/// differently gets links that do not resolve. The base is `browse_url` when set: an `https://` URL with a dotted host
/// (anything else fails the configuration load), taken as is apart from userinfo, query, fragment, a trailing `/` and
/// `.git`. Otherwise it is derived from `url` when that is an `https://host/org/repo` URL, an scp-style
/// `git@host:org/repo` or an `ssh://git@host[:port]/org/repo` one (user, port and `.git` dropped). A host without a dot
/// is an ssh host alias, not a web host, so it gives no link: set `browse_url` for such a repository. Any other URL
/// gives no link.
#[derive(Debug, Clone, Deserialize)]
pub struct RepoConfig {
    pub name: String,
    pub url: Option<String>,
    /// The repository's web address, for record links when `url` cannot give one (an ssh host alias).
    #[serde(default)]
    pub browse_url: Option<String>,
    pub adr_dir: String,
}

/// `Debug` is written by hand so `database_url` never prints its password (see `redacted_database_url`).
#[derive(Clone, Deserialize)]
pub struct ServerConfig {
    pub database_url: String,
    #[serde(default = "default_bind")]
    pub bind: String,
    pub key_prefix: String,
    #[serde(default = "default_buckets")]
    pub horizon_buckets: Vec<String>,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default)]
    pub allowed_emails: Vec<String>,
    #[serde(default)]
    pub admins: Vec<String>,
    #[serde(default)]
    pub repos: Vec<RepoConfig>,
    /// The URL users reach the instance at; the OIDC redirect is `{public_url}/auth/callback` and the cookie is
    /// `Secure` when it is https. Required when `[oidc]` is configured. Without sign-in it defaults from the bind so
    /// token-only configs keep loading: `http://127.0.0.1:{port}` when the bind host is unspecified (`0.0.0.0`, `::`),
    /// else `http://{bind}`.
    #[serde(default)]
    pub public_url: Option<String>,
    #[serde(default = "default_session_days")]
    pub session_days: i64,
    /// `None`: no browser sign-in (CLI tokens only); the sign-in page says so.
    #[serde(default)]
    pub oidc: Option<OidcSection>,
    /// `None`: the GitHub webhook answers 404.
    #[serde(default)]
    pub github: Option<GithubConfig>,
    /// The Jira import endpoints; off by default.
    #[serde(default)]
    pub import: ImportConfig,
    #[serde(default)]
    pub board: BoardConfig,
    #[serde(default)]
    pub sprints: SprintsConfig,
    #[serde(default)]
    pub events: EventsConfig,
    /// The MCP endpoint; off by default.
    #[serde(default)]
    pub mcp: McpConfig,
}

/// `[board]`: board-only settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardConfig {
    #[serde(default = "default_done_window_days")]
    pub done_window_days: u32,
}
impl Default for BoardConfig {
    fn default() -> Self {
        Self {
            done_window_days: default_done_window_days(),
        }
    }
}
fn default_done_window_days() -> u32 {
    14
}

/// `[sprints]`: sprint defaults.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SprintsConfig {
    /// A new sprint's default length in days (1 to 90).
    #[serde(default = "default_sprint_length_days")]
    pub length_days: u32,
    /// The reference line for the upkeep share, in percent (0 to 100); not a budget.
    #[serde(default = "default_upkeep_line")]
    pub upkeep_line: u8,
}
impl Default for SprintsConfig {
    fn default() -> Self {
        Self {
            length_days: default_sprint_length_days(),
            upkeep_line: default_upkeep_line(),
        }
    }
}
fn default_sprint_length_days() -> u32 {
    14
}
fn default_upkeep_line() -> u8 {
    20
}

/// `[events]`: the live event stream.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventsConfig {
    /// How often `GET /api/v1/events` reads the activity log, in milliseconds.
    #[serde(default = "default_poll_ms")]
    pub poll_ms: u64,
}
impl Default for EventsConfig {
    fn default() -> Self {
        Self {
            poll_ms: default_poll_ms(),
        }
    }
}
fn default_poll_ms() -> u64 {
    2000
}

impl std::fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Destructured so a new field fails to compile until it is listed here, redacted or not.
        let ServerConfig {
            database_url,
            bind,
            key_prefix,
            horizon_buckets,
            allowed_domains,
            allowed_emails,
            admins,
            repos,
            public_url,
            session_days,
            oidc,
            github,
            import,
            board,
            sprints,
            events,
            mcp,
        } = self;
        f.debug_struct("ServerConfig")
            .field("database_url", &redacted_database_url(database_url))
            .field("bind", bind)
            .field("key_prefix", key_prefix)
            .field("horizon_buckets", horizon_buckets)
            .field("allowed_domains", allowed_domains)
            .field("allowed_emails", allowed_emails)
            .field("admins", admins)
            .field("repos", repos)
            .field("public_url", public_url)
            .field("session_days", session_days)
            .field("oidc", oidc)
            .field("github", github)
            .field("import", import)
            .field("board", board)
            .field("sprints", sprints)
            .field("events", events)
            .field("mcp", mcp)
            .finish()
    }
}

/// The database URL with its credentials and query (which may carry `password=`) left out: `postgres://<redacted>@host:port/db`.
/// A value that does not parse as a URL (a key-value connection string) is redacted whole.
fn redacted_database_url(raw: &str) -> String {
    let Ok(u) = url::Url::parse(raw) else {
        return "<redacted>".into();
    };
    let Some(host) = u.host_str() else {
        return "<redacted>".into();
    };
    let creds = if u.username().is_empty() && u.password().is_none() {
        ""
    } else {
        "<redacted>@"
    };
    let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
    let query = if u.query().is_some() {
        "?<redacted>"
    } else {
        ""
    };
    format!("{}://{creds}{host}{port}{}{query}", u.scheme(), u.path())
}

/// `[import]`: the switch for `POST /api/v1/import/*`. Off (the default), every import endpoint answers 404; turn it on
/// for the migration and off again afterwards.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportConfig {
    #[serde(default)]
    pub enabled: bool,
}

/// `[mcp]`: the MCP endpoint at `/mcp` (kb/ADR-0029). Off (the default), `/mcp` answers 404. It can be on only with
/// `[oidc]`: an instance without sign-in answers 404 there (kb/ADR-0029 R2).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpConfig {
    #[serde(default)]
    pub enabled: bool,
    /// The `Origin` header values `/mcp` accepts; a request without the header always passes. Unset: only the
    /// origin of `public_url`. Each entry is an origin as a browser sends it: scheme, host and a non-default port,
    /// no path and no trailing slash.
    #[serde(default)]
    pub allowed_origins: Option<Vec<String>>,
}

/// The GitHub pull-request webhook (`POST /webhooks/github`).
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GithubConfig {
    /// The secret configured on the GitHub webhook; every delivery's `X-Hub-Signature-256` is checked against it.
    pub webhook_secret: String,
    /// `owner/name` of the repositories whose events are accepted (case-insensitive); others are ignored. Pull requests
    /// are stored under the lower-cased name whatever the spelling here.
    pub repos: Vec<String>,
    /// Jira delivery project keys (the project holding stories, bugs and tasks) whose issue keys (`SCRUM-11`) in branch
    /// names resolve through a story's `external_ref`. Not the discovery project: its ideas import as initiatives.
    #[serde(default)]
    pub legacy_prefixes: Vec<String>,
}

impl std::fmt::Debug for GithubConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GithubConfig")
            .field("webhook_secret", &"<redacted>")
            .field("repos", &self.repos)
            .field("legacy_prefixes", &self.legacy_prefixes)
            .finish()
    }
}

/// Which identity provider browser sign-in uses.
#[derive(Clone, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase", deny_unknown_fields)]
pub enum OidcSection {
    Google {
        client_id: String,
        client_secret: String,
    },
    /// Local development and tests only: compiled only with the `static-oidc` feature, and refused unless both
    /// `bind` and the `public_url` host are loopback.
    #[cfg(feature = "static-oidc")]
    Static { static_email: String },
}

impl std::fmt::Debug for OidcSection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OidcSection::Google { client_id, .. } => f
                .debug_struct("Google")
                .field("client_id", client_id)
                .field("client_secret", &"<redacted>")
                .finish(),
            #[cfg(feature = "static-oidc")]
            OidcSection::Static { static_email } => f
                .debug_struct("Static")
                .field("static_email", static_email)
                .finish(),
        }
    }
}

const LOOPBACK: [&str; 4] = ["127.0.0.1", "localhost", "::1", "[::1]"];

/// The host and port parts of a `host:port` bind address; the host without IPv6 brackets.
fn split_bind(bind: &str) -> (&str, Option<&str>) {
    let (host, port) = match bind.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => (h, Some(p)),
        _ => (bind, None),
    };
    (host.trim_start_matches('[').trim_end_matches(']'), port)
}

#[cfg(feature = "static-oidc")]
fn bind_host(bind: &str) -> &str {
    split_bind(bind).0
}

/// The public URL a config without `public_url` gets: always loopback on the bind's port, whatever the bind host. Only
/// token-only instances get here (`[oidc]` requires `public_url`), and the URL then feeds nothing but cookie flags and
/// redirects, which do not exist without sign-in.
fn default_public_url(bind: &str) -> String {
    match split_bind(bind).1 {
        Some(p) => format!("http://127.0.0.1:{p}"),
        None => "http://127.0.0.1".into(),
    }
}

/// `owner/name`: two non-empty parts without whitespace or further slashes.
fn is_owner_name(r: &str) -> bool {
    match r.split_once('/') {
        Some((o, n)) => [o, n]
            .iter()
            .all(|p| !p.is_empty() && !p.contains(|c: char| c == '/' || c.is_whitespace())),
        None => false,
    }
}

/// A Jira project key as it appears in issue keys: `[A-Z][A-Z0-9]*`.
fn is_jira_prefix(p: &str) -> bool {
    let mut chars = p.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

fn is_loopback(host: &str) -> bool {
    LOOPBACK.contains(&host)
}

/// serde's `invalid type: string "x", expected …`, `invalid value …` and `unknown variant `x`, expected …` messages
/// quote the offending scalar, which can be a secret (an unquoted numeric `webhook_secret`): keep only the kind and
/// the "expected …" part. The last ", expected " is the real one: the quoted value can contain the text itself.
fn scrubbed_message(message: &str) -> String {
    let message = message.trim_end();
    if ["invalid type", "invalid value", "unknown variant"]
        .iter()
        .any(|k| message.starts_with(k))
    {
        let kind = message.split(' ').take(2).collect::<Vec<_>>().join(" ");
        let kind = kind.trim_end_matches([':', ',']);
        return match message.rfind(", expected ") {
            Some(i) => format!("{kind}, expected {}", &message[i + ", expected ".len()..]),
            None => kind.to_string(),
        };
    }
    message.to_string()
}
fn default_bind() -> String {
    "127.0.0.1:7474".into()
}
fn default_session_days() -> i64 {
    30
}
fn default_buckets() -> Vec<String> {
    ["Now", "Next", "Upcoming", "Later", "Won't do"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

impl ServerConfig {
    /// What `reeve-server repos` prints: `name<TAB>url` for each `[[repos]]` entry with a non-blank url, in configuration
    /// order, and one warning for each entry without one. The listing is read line by line and split at the tab, so a
    /// name or url holding a tab or a line break is an error rather than a listing that names another repository.
    pub fn repo_listing(&self) -> anyhow::Result<(String, Vec<String>)> {
        let mut listing = String::new();
        let mut warnings = Vec::new();
        for r in &self.repos {
            let url = r.url.as_deref().map(str::trim).filter(|u| !u.is_empty());
            let bad = |s: &str| s.contains(['\t', '\n', '\r']);
            if bad(&r.name) || url.is_some_and(bad) {
                anyhow::bail!(
                    "[[repos]] entry {:?}: name and url must not contain tabs or line breaks",
                    r.name
                );
            }
            match url {
                Some(u) => {
                    listing.push_str(&r.name);
                    listing.push('\t');
                    listing.push_str(u);
                    listing.push('\n');
                }
                None => warnings.push(format!(
                    "[[repos]] entry {:?} has no url; the scheduled check cannot clone it",
                    r.name
                )),
            }
        }
        Ok((listing, warnings))
    }
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        Self::parse(&std::fs::read_to_string(path)?)
    }
    /// Parses and validates configuration text; `load` reads the file and calls this.
    /// A TOML error names its line and column but never quotes the line: it can hold the database password.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let cfg: Self = toml::from_str(text).map_err(|e| {
            let at = e.span().map(|span| {
                let before = text.get(..span.start).unwrap_or("");
                let line = before.matches('\n').count() + 1;
                let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
                format!(" at line {line}, column {column}")
            });
            anyhow::anyhow!(
                "configuration is not valid TOML{}: {}",
                at.unwrap_or_default(),
                scrubbed_message(e.message())
            )
        })?;
        cfg.validate()?;
        Ok(cfg)
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.key_prefix.is_empty() || !self.key_prefix.chars().all(|c| c.is_ascii_uppercase()) {
            anyhow::bail!("key_prefix must be uppercase letters");
        }
        if !(1..=365).contains(&self.session_days) {
            anyhow::bail!("session_days must be between 1 and 365");
        }
        if !(1..=3650).contains(&self.board.done_window_days) {
            anyhow::bail!("[board] done_window_days must be between 1 and 3650");
        }
        if !(1..=90).contains(&self.sprints.length_days) {
            anyhow::bail!("[sprints] length_days must be between 1 and 90");
        }
        // A value above 255 never gets here: serde rejects it while parsing the u8.
        if self.sprints.upkeep_line > 100 {
            anyhow::bail!("[sprints] upkeep_line must be between 0 and 100");
        }
        if !(10..=10_000).contains(&self.events.poll_ms) {
            anyhow::bail!("[events] poll_ms must be between 10 and 10000");
        }
        for r in &self.repos {
            let n = r.name.as_str();
            if n.is_empty() || n.starts_with(['.', '-']) || n.contains(['/', '\\']) {
                anyhow::bail!(
                    "[[repos]] name {n:?} must be a single directory name: not empty, not starting with '.' or '-', without '/' or '\\'"
                );
            }
            if let Some(b) = &r.browse_url
                && crate::state::explicit_browse_base(b).is_none()
            {
                anyhow::bail!(
                    "[[repos]] {n:?}: browse_url {b:?} must be an https:// URL with a host name (a dot in it), like https://github.com/org/repo"
                );
            }
        }
        if self.oidc.is_some() && self.public_url.is_none() {
            anyhow::bail!(
                "public_url is required when [oidc] is configured: browser sign-in redirects to <public_url>/auth/callback"
            );
        }
        if self.mcp.enabled && self.oidc.is_none() {
            anyhow::bail!(
                "[mcp] enabled = true requires [oidc]: an instance without sign-in answers 404 on /mcp (kb/ADR-0029 R2)"
            );
        }
        for o in self.mcp.allowed_origins.iter().flatten() {
            let is_origin =
                url::Url::parse(o).is_ok_and(|u| u.origin().ascii_serialization() == *o);
            if !is_origin {
                anyhow::bail!(
                    "[mcp] allowed_origins entry {o:?} must be an origin such as https://claude.ai: scheme, host and optional port, no path and no trailing slash"
                );
            }
        }
        if let Some(g) = &self.github {
            if g.webhook_secret.len() < 16 {
                anyhow::bail!("[github] webhook_secret must be at least 16 characters");
            }
            if g.repos.is_empty() {
                anyhow::bail!("[github] repos must name at least one repository (owner/name)");
            }
            if let Some(r) = g.repos.iter().find(|r| !is_owner_name(r)) {
                anyhow::bail!("[github] repos entry {r:?} must look like owner/name");
            }
            for p in &g.legacy_prefixes {
                if !is_jira_prefix(p) {
                    anyhow::bail!(
                        "[github] legacy_prefixes entry {p:?} must be uppercase letters and digits, starting with a letter"
                    );
                }
                if *p == self.key_prefix {
                    anyhow::bail!(
                        "[github] legacy_prefixes entry {p:?} equals key_prefix: its branches would link to the wrong story"
                    );
                }
            }
        }
        #[cfg_attr(not(feature = "static-oidc"), allow(unused_variables))]
        let url = self.try_public_url()?;
        #[cfg(feature = "static-oidc")]
        if matches!(self.oidc, Some(OidcSection::Static { .. })) {
            let public_host = url.host_str().unwrap_or("");
            if !is_loopback(bind_host(&self.bind)) || !is_loopback(public_host) {
                anyhow::bail!(
                    "the static oidc provider requires a loopback bind and a loopback public_url"
                );
            }
        }
        // kb/ADR-0015 R2: the floor guarantees a usable admin, so break-glass, "cannot revoke your own grant" and
        // "cannot disable a configured admin" always rest on one.
        if self.admins.iter().all(|a| a.trim().is_empty()) {
            anyhow::bail!("admins must name at least one admin");
        }
        let floor = self.identity();
        for a in self.admins.iter().filter(|a| !a.trim().is_empty()) {
            if !floor.is_allowed(a) {
                anyhow::bail!(
                    "admin {} is not allowed by allowed_domains or allowed_emails",
                    reeve_identity::normalise_email(a)
                );
            }
        }
        Ok(())
    }
    /// The parsed public URL: the configured value, or the default from the bind. `load` has validated it, so this never fails
    /// on a loaded configuration.
    pub fn public_url(&self) -> url::Url {
        self.try_public_url()
            .expect("public_url is validated when the configuration loads")
    }
    /// Whether cookies are `Secure` (and the session cookie carries the `__Host-` prefix): the parsed scheme, never a
    /// string prefix.
    pub fn https(&self) -> bool {
        self.public_url().scheme() == "https"
    }
    /// Whether `/mcp` accepts this `Origin` header value: one of `[mcp] allowed_origins`, or the origin of
    /// `public_url` when the list is unset. The comparison is exact, as origins are.
    pub fn mcp_origin_allowed(&self, origin: &str) -> bool {
        match &self.mcp.allowed_origins {
            Some(list) => list.iter().any(|allowed| allowed == origin),
            None => self.public_url().origin().ascii_serialization() == origin,
        }
    }
    fn try_public_url(&self) -> anyhow::Result<url::Url> {
        let raw = match &self.public_url {
            Some(u) => u.clone(),
            None => default_public_url(&self.bind),
        };
        let u =
            url::Url::parse(&raw).map_err(|e| anyhow::anyhow!("public_url is not a URL: {e}"))?;
        if !u.username().is_empty() || u.password().is_some() {
            anyhow::bail!("public_url must not carry credentials");
        }
        if u.query().is_some() || u.fragment().is_some() {
            anyhow::bail!("public_url must not have a query or fragment");
        }
        if u.path() != "/" {
            anyhow::bail!("public_url must not have a path");
        }
        let host = u.host_str().unwrap_or("");
        let ok = u.scheme() == "https" || (u.scheme() == "http" && is_loopback(host));
        if !ok {
            anyhow::bail!("public_url must be https, or http on a loopback host");
        }
        Ok(u)
    }
    pub fn identity(&self) -> reeve_identity::IdentityConfig {
        reeve_identity::IdentityConfig {
            allowed_domains: self.allowed_domains.clone(),
            allowed_emails: self.allowed_emails.clone(),
            admins: self.admins.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(extra: &str) -> anyhow::Result<ServerConfig> {
        ServerConfig::parse(&format!(
            "database_url = \"postgres://x\"\nkey_prefix = \"TST\"\nallowed_domains = [\"example.com\"]\nadmins = [\"admin@example.com\"]\n{extra}"
        ))
    }

    /// A configuration without the helper's floor: `extra` supplies the identity lists itself.
    fn bare(extra: &str) -> anyhow::Result<ServerConfig> {
        ServerConfig::parse(&format!(
            "database_url = \"postgres://x\"\nkey_prefix = \"TST\"\n{extra}"
        ))
    }

    #[test]
    fn a_configuration_must_name_an_admin() {
        let e = bare("allowed_domains = [\"example.com\"]").unwrap_err();
        assert!(
            e.to_string()
                .contains("admins must name at least one admin"),
            "{e}"
        );
    }

    #[test]
    fn blank_admin_entries_are_ignored_like_at_sign_in() {
        let e = bare("allowed_domains = [\"example.com\"]\nadmins = [\"  \"]").unwrap_err();
        assert!(
            e.to_string()
                .contains("admins must name at least one admin"),
            "{e}"
        );
        assert!(
            bare("allowed_domains = [\"example.com\"]\nadmins = [\"\", \"admin@example.com\"]")
                .is_ok()
        );
        assert!(
            bare("allowed_domains = [\"example.com\"]\nadmins = [\" Admin@Example.com \"]").is_ok()
        );
    }

    #[test]
    fn every_configured_admin_must_be_allowed_by_the_configuration() {
        let e = bare("allowed_domains = [\"example.com\"]\nadmins = [\"admin@example.com\", \"Boss@Other.org\"]")
            .unwrap_err();
        assert!(
            e.to_string().contains(
                "admin boss@other.org is not allowed by allowed_domains or allowed_emails"
            ),
            "{e}"
        );
        // allowed by the domain, by the email list, and matched case-insensitively
        assert!(
            bare("allowed_domains = [\"Example.com\"]\nadmins = [\"Admin@example.com\"]").is_ok()
        );
        assert!(
            bare("allowed_emails = [\"boss@other.org\"]\nadmins = [\"boss@other.org\"]").is_ok()
        );
    }

    #[test]
    fn repo_listing_has_each_repository_with_a_url_in_order_and_warns_about_the_rest() {
        let c = cfg("[[repos]]\nname = \"kb\"\nurl = \" git@github.com:G/kb.git \"\nadr_dir = \"a\"\n[[repos]]\nname = \"portal\"\nadr_dir = \"b\"\n[[repos]]\nname = \"empty\"\nurl = \"\"\nadr_dir = \"c\"\n[[repos]]\nname = \"runner\"\nurl = \"https://github.com/G/runner.git\"\nadr_dir = \"d\"").unwrap();
        let (listing, warnings) = c.repo_listing().unwrap();
        assert_eq!(
            listing,
            "kb\tgit@github.com:G/kb.git\nrunner\thttps://github.com/G/runner.git\n"
        );
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(
            warnings[0].contains("\"portal\"") && warnings[1].contains("\"empty\""),
            "{warnings:?}"
        );
    }

    #[test]
    fn repo_listing_refuses_a_tab_or_line_break_in_a_name_or_url() {
        for (name, url) in [
            ("a\\tb", "u"),
            ("a\\nb", "u"),
            ("a", "u\\tv"),
            ("a", "u\\rv"),
        ] {
            let c = cfg(&format!(
                "[[repos]]\nname = \"{name}\"\nurl = \"{url}\"\nadr_dir = \"x\""
            ))
            .unwrap();
            let err = c.repo_listing().unwrap_err();
            assert!(
                err.to_string().contains("tabs or line breaks"),
                "{name:?} {url:?}: {err}"
            );
        }
    }

    #[test]
    fn a_repository_name_must_be_a_single_directory_name() {
        for name in ["", ".", "..", ".hidden", "a/b", "a\\\\b", "-c", "../x"] {
            let err = cfg(&format!(
                "[[repos]]\nname = \"{name}\"\nurl = \"u\"\nadr_dir = \"x\""
            ))
            .unwrap_err();
            assert!(
                err.to_string().contains("single directory name"),
                "{name:?}: {err}"
            );
        }
        assert!(cfg("[[repos]]\nname = \"kb-2.x\"\nurl = \"u\"\nadr_dir = \"x\"").is_ok());
    }

    /// An explicit `browse_url` must be an https URL with a dotted host; anything else stops the load, like a bad
    /// public_url does. Catches a quietly ignored typo that would leave records without links.
    #[test]
    fn a_repository_browse_url_must_be_https_with_a_dotted_host() {
        for bad in [
            "",
            "http://github.com/o/r",
            "git@github.com:o/r.git",
            "ssh://git@github.com/o/r",
            "https://localhost/o/r",
            "https://github-kb/o/r",
        ] {
            let err = cfg(&format!(
                "[[repos]]\nname = \"kb\"\nurl = \"u\"\nbrowse_url = \"{bad}\"\nadr_dir = \"x\""
            ))
            .unwrap_err();
            assert!(err.to_string().contains("browse_url"), "{bad:?}: {err}");
        }
        let c = cfg(
            "[[repos]]\nname = \"kb\"\nurl = \"u\"\nbrowse_url = \"https://github.com/o/r\"\nadr_dir = \"x\"\n[[repos]]\nname = \"other\"\nadr_dir = \"x\"",
        )
        .unwrap();
        assert_eq!(
            c.repos[0].browse_url.as_deref(),
            Some("https://github.com/o/r")
        );
        assert_eq!(c.repos[1].browse_url, None);
    }

    /// The browse base: an explicit `browse_url` wins (trimmed of `.git`, `/` and credentials); an ssh host alias gives none
    /// on its own; an alias url with an explicit `browse_url` links from the explicit one.
    #[test]
    fn browse_urls_prefer_the_explicit_address_and_refuse_an_ssh_alias() {
        let c = cfg(concat!(
            "[[repos]]\nname = \"alias\"\nurl = \"git@github-knowledge-base:Acme/kb.git\"\nadr_dir = \"x\"\n",
            "[[repos]]\nname = \"aliasset\"\nurl = \"git@github-knowledge-base:Acme/kb.git\"\n",
            "browse_url = \"https://bot:SECRET@github.com/Acme/kb.git/?x=1#y\"\nadr_dir = \"x\"\n",
            "[[repos]]\nname = \"over\"\nurl = \"git@github.com:Acme/derived.git\"\nbrowse_url = \"https://git.example.com/Acme/explicit/\"\nadr_dir = \"x\"\n",
            "[[repos]]\nname = \"real\"\nurl = \"git@github.com:Acme/real.git\"\nadr_dir = \"x\"\n",
            "[[repos]]\nname = \"urlalias\"\nurl = \"ssh://git@github-kb/Acme/kb.git\"\nadr_dir = \"x\"",
        ))
        .unwrap();
        let urls = crate::state::browsable_repo_urls(&c);
        assert_eq!(urls.get("alias"), None);
        assert_eq!(urls.get("urlalias"), None);
        assert_eq!(urls["aliasset"], "https://github.com/Acme/kb");
        assert_eq!(urls["over"], "https://git.example.com/Acme/explicit");
        assert_eq!(urls["real"], "https://github.com/Acme/real");
    }

    #[test]
    fn the_example_configuration_lists_its_repositories() {
        let c = ServerConfig::parse(include_str!("../../../reeve.example.toml")).unwrap();
        let (listing, warnings) = c.repo_listing().unwrap();
        let names: Vec<&str> = listing
            .lines()
            .map(|l| l.split('\t').next().unwrap())
            .collect();
        assert_eq!(names, ["knowledge-base", "portal"]);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn public_url_defaults_to_the_bind_address_over_http() {
        let c = cfg("").unwrap();
        assert_eq!(c.public_url().as_str(), "http://127.0.0.1:7474/");
        assert!(!c.https());
        assert_eq!(c.session_days, 30);
        assert!(c.oidc.is_none());
    }

    #[test]
    fn a_legacy_wildcard_bind_without_public_url_or_sign_in_loads_on_loopback() {
        let c = cfg("bind = \"0.0.0.0:7474\"").unwrap();
        assert_eq!(c.public_url().as_str(), "http://127.0.0.1:7474/");
        let c = cfg("bind = \"[::]:7474\"").unwrap();
        assert_eq!(c.public_url().as_str(), "http://127.0.0.1:7474/");
        let c = cfg("bind = \"localhost:7475\"").unwrap();
        assert_eq!(c.public_url().as_str(), "http://127.0.0.1:7475/");
    }

    #[test]
    fn a_token_only_config_on_a_concrete_network_bind_loads() {
        let c = cfg("bind = \"192.168.1.10:7474\"").unwrap();
        assert_eq!(c.public_url().as_str(), "http://127.0.0.1:7474/");
        let err = cfg("bind = \"192.168.1.10:7474\"\n[oidc]\nprovider = \"google\"\nclient_id = \"id\"\nclient_secret = \"s\"")
            .unwrap_err();
        assert!(err.to_string().contains("public_url"), "{err}");
    }

    #[test]
    fn sign_in_without_public_url_is_refused_naming_it() {
        let err = cfg("bind = \"0.0.0.0:7474\"\n[oidc]\nprovider = \"google\"\nclient_id = \"id\"\nclient_secret = \"s\"")
            .unwrap_err();
        assert!(err.to_string().contains("public_url"), "{err}");
        let err = cfg("[oidc]\nprovider = \"google\"\nclient_id = \"id\"\nclient_secret = \"s\"")
            .unwrap_err();
        assert!(err.to_string().contains("public_url is required"), "{err}");
    }

    #[test]
    fn https_comes_from_the_parsed_scheme() {
        let c = cfg("public_url = \"HTTPS://Reeve.Example.com\"").unwrap();
        assert!(c.https());
        assert_eq!(c.public_url().host_str(), Some("reeve.example.com"));
    }

    #[test]
    fn public_url_rejects_paths_credentials_queries_and_plain_http_off_loopback() {
        for bad in [
            "https://reeve.example.com/sub",
            "https://u:p@reeve.example.com",
            "https://reeve.example.com/?x=1",
            "https://reeve.example.com/#f",
            "http://reeve.example.com",
            "ftp://127.0.0.1",
            "not a url",
        ] {
            assert!(cfg(&format!("public_url = \"{bad}\"")).is_err(), "{bad}");
        }
        assert!(cfg("public_url = \"https://reeve.example.com/\"").is_ok());
        assert!(cfg("public_url = \"http://localhost:5173\"").is_ok());
        assert!(cfg("public_url = \"http://[::1]:7474\"").is_ok());
    }

    #[test]
    fn a_syntax_error_does_not_echo_the_line_and_its_password() {
        let text = "key_prefix = \"TST\"\ndatabase_url = \"postgres://reeve:hunter2-secret@db/reeve\" oops\n";
        let err = format!("{:#}", ServerConfig::parse(text).unwrap_err());
        assert!(!err.contains("hunter2"), "{err}");
        assert!(!err.contains("postgres://"), "{err}");
        assert!(
            err.contains("line 2, column"),
            "the position is reported: {err}"
        );
        // the same for a type error on a line holding the password
        let err = format!(
            "{:#}",
            ServerConfig::parse("database_url = [\"postgres://reeve:hunter2-secret@db/reeve\"]\n")
                .unwrap_err()
        );
        assert!(!err.contains("hunter2"), "{err}");
        assert!(err.contains("line 1, column"), "{err}");
    }

    #[test]
    fn a_type_error_does_not_echo_the_offending_value() {
        let err = format!(
            "{:#}",
            ServerConfig::parse("key_prefix = 123456789\n").unwrap_err()
        );
        assert!(!err.contains("123456789"), "{err}");
        assert!(err.contains("expected"), "{err}");
        assert!(err.contains("line 1, column"), "{err}");
        let err = format!(
            "{:#}",
            ServerConfig::parse("bind = 987654321\n").unwrap_err()
        );
        assert!(!err.contains("987654321"), "{err}");
        // a quoted value can contain the separator itself; an unknown variant quotes its input too
        let err = format!(
            "{:#}",
            ServerConfig::parse("session_days = \"pw, expected SECRETTAIL\"\n").unwrap_err()
        );
        assert!(!err.contains("SECRETTAIL") && !err.contains("pw"), "{err}");
        let err = format!(
            "{:#}",
            ServerConfig::parse("[oidc]\nprovider = \"hunter2-provider\"\n").unwrap_err()
        );
        assert!(!err.contains("hunter2"), "{err}");
        assert!(err.contains("expected"), "{err}");
    }

    #[test]
    fn the_example_configuration_loads() {
        let c = ServerConfig::parse(include_str!("../../../reeve.example.toml")).unwrap();
        assert!(c.https());
        // its commented [github] block loads once uncommented
        let text = include_str!("../../../reeve.example.toml")
            .replace("# [github]", "[github]")
            .replace("# webhook_secret", "webhook_secret")
            .replace("# repos", "repos")
            .replace("# legacy_prefixes", "legacy_prefixes");
        let g = ServerConfig::parse(&text).unwrap().github.unwrap();
        assert_eq!(g.legacy_prefixes, ["SCRUM"]);
    }

    #[test]
    fn board_done_window_days_is_bounded_and_defaults_to_14() {
        for bad in [0, 3651] {
            let e = cfg(&format!("[board]\ndone_window_days = {bad}")).unwrap_err();
            assert!(e.to_string().contains("[board] done_window_days"), "{e}");
        }
        assert!(cfg("[board]\ndone_window_days = 1").is_ok());
        assert!(cfg("[board]\ndone_window_days = 3650").is_ok());
        assert_eq!(cfg("").unwrap().board.done_window_days, 14);
        assert!(cfg("[board]\nwindow = 3").is_err());
    }

    #[test]
    fn sprints_are_bounded_default_to_14_and_20_and_reject_unknown_keys() {
        let e = cfg("[sprints]\nlength_days = 0").unwrap_err();
        assert!(
            e.to_string()
                .contains("[sprints] length_days must be between 1 and 90"),
            "{e}"
        );
        assert!(cfg("[sprints]\nlength_days = 91").is_err());
        let e = cfg("[sprints]\nupkeep_line = 101").unwrap_err();
        assert!(
            e.to_string()
                .contains("[sprints] upkeep_line must be between 0 and 100"),
            "{e}"
        );
        assert!(cfg("[sprints]\nlength_days = 90\nupkeep_line = 100").is_ok());
        assert!(cfg("[sprints]\nupkeep_line = 0").is_ok());
        let d = cfg("").unwrap();
        assert_eq!((d.sprints.length_days, d.sprints.upkeep_line), (14, 20));
        assert!(cfg("[sprints]\nlength = 7").is_err());
    }

    #[test]
    fn events_poll_ms_is_bounded_and_defaults_to_2000() {
        for bad in [0, 9, 10_001] {
            let e = cfg(&format!("[events]\npoll_ms = {bad}")).unwrap_err();
            assert!(e.to_string().contains("[events] poll_ms"), "{e}");
        }
        assert_eq!(cfg("[events]\npoll_ms = 10").unwrap().events.poll_ms, 10);
        assert_eq!(
            cfg("[events]\npoll_ms = 10000").unwrap().events.poll_ms,
            10_000
        );
        assert_eq!(cfg("").unwrap().events.poll_ms, 2000);
        assert!(cfg("[events]\ninterval = 3").is_err());
    }

    #[test]
    fn session_days_must_be_between_1_and_365() {
        assert!(cfg("session_days = 0").is_err());
        assert!(cfg("session_days = 366").is_err());
        assert!(cfg("session_days = 1").is_ok());
        assert!(cfg("session_days = 365").is_ok());
    }

    #[test]
    fn debug_redacts_the_database_password() {
        for (url, shown_url) in [
            (
                "postgres://reeve:hunter2-pw@db.internal:5432/reeve",
                "postgres://<redacted>@db.internal:5432/reeve",
            ),
            (
                "postgres://db.internal/reeve?user=reeve&password=hunter2-pw",
                "postgres://db.internal/reeve?<redacted>",
            ),
            ("host=db user=reeve password=hunter2-pw", "<redacted>"),
        ] {
            let c = ServerConfig::parse(&format!(
                "database_url = \"{url}\"\nkey_prefix = \"TST\"\nallowed_domains = [\"example.com\"]\nadmins = [\"admin@example.com\"]\n"
            ))
            .unwrap();
            let shown = format!("{c:?}");
            assert!(!shown.contains("hunter2-pw"), "{shown}");
            assert!(shown.contains(shown_url), "{shown}");
            assert!(shown.contains("TST"), "{shown}");
            assert_eq!(c.database_url, url, "the value itself is untouched");
        }
    }

    #[test]
    fn google_section_parses_and_debug_redacts_the_secret() {
        let c = cfg("public_url = \"https://reeve.example.com\"\n[oidc]\nprovider = \"google\"\nclient_id = \"id\"\nclient_secret = \"s3cr3t\"")
            .unwrap();
        let shown = format!("{:?}", c.oidc.unwrap());
        assert!(!shown.contains("s3cr3t") && shown.contains("id"), "{shown}");
    }

    #[test]
    fn github_section_validates_and_debug_redacts_the_secret() {
        let c =
            cfg("[github]\nwebhook_secret = \"sentinel-secret-0123\"\nrepos = [\"Acme/portal\"]")
                .unwrap();
        let g = c.github.as_ref().unwrap();
        assert!(g.legacy_prefixes.is_empty());
        for shown in [format!("{g:?}"), format!("{c:?}")] {
            assert!(
                !shown.contains("sentinel-secret-0123") && shown.contains("Acme/portal"),
                "{shown}"
            );
        }
        let err = cfg("[github]\nwebhook_secret = \"short\"\nrepos = [\"a/b\"]").unwrap_err();
        assert!(err.to_string().contains("webhook_secret"), "{err}");
        let err = cfg("[github]\nwebhook_secret = \"0123456789abcdef\"\nrepos = []").unwrap_err();
        assert!(err.to_string().contains("repos"), "{err}");
        assert!(cfg("").unwrap().github.is_none());
    }

    #[test]
    fn github_repos_must_look_like_owner_name() {
        let github = |repos: &str| {
            cfg(&format!(
                "[github]\nwebhook_secret = \"0123456789abcdef\"\nrepos = {repos}"
            ))
        };
        assert!(github("[\"Acme/portal\", \"a/b.c\"]").is_ok());
        for bad in [
            "[\"portal\"]",
            "[\"Acme/portal \"]",
            "[\"/portal\"]",
            "[\"Acme/\"]",
            "[\"a/b/c\"]",
            "[\"a b/c\"]",
            "[\"\"]",
        ] {
            let err = github(bad).unwrap_err();
            assert!(err.to_string().contains("owner/name"), "{bad}: {err}");
        }
    }

    #[test]
    fn github_legacy_prefixes_must_look_like_jira_keys_and_differ_from_key_prefix() {
        let github = |prefixes: &str| {
            cfg(&format!(
                "[github]\nwebhook_secret = \"0123456789abcdef\"\nrepos = [\"a/b\"]\nlegacy_prefixes = {prefixes}"
            ))
        };
        assert!(github("[\"JPD\", \"AB2\"]").is_ok());
        for bad in [
            "[\"\"]",
            "[\"jpd\"]",
            "[\"Jpd\"]",
            "[\"JP/D\"]",
            "[\"2JPD\"]",
            "[\"P-B\"]",
            "[\"P B\"]",
        ] {
            let err = github(bad).unwrap_err();
            assert!(err.to_string().contains("legacy_prefixes"), "{bad}: {err}");
        }
        let err = github("[\"TST\"]").unwrap_err();
        assert!(err.to_string().contains("equals key_prefix"), "{err}");
    }

    #[cfg(feature = "static-oidc")]
    #[test]
    fn static_is_refused_behind_a_reverse_proxy() {
        let err = cfg("bind = \"127.0.0.1:7474\"\npublic_url = \"https://reeve.example.com\"\n[oidc]\nprovider = \"static\"\nstatic_email = \"dan@example.com\"")
            .unwrap_err();
        assert!(err.to_string().contains("loopback"), "{err}");
    }

    #[cfg(feature = "static-oidc")]
    #[test]
    fn static_is_refused_on_a_public_bind() {
        assert!(cfg("bind = \"0.0.0.0:7474\"\npublic_url = \"http://127.0.0.1:7474\"\n[oidc]\nprovider = \"static\"\nstatic_email = \"dan@example.com\"").is_err());
    }

    #[cfg(feature = "static-oidc")]
    #[test]
    fn static_loads_when_bind_and_public_url_are_loopback() {
        for (bind, public) in [
            ("127.0.0.1:7474", "http://127.0.0.1:7474"),
            ("localhost:7474", "http://localhost:7474"),
            ("[::1]:7474", "http://[::1]:7474"),
        ] {
            let c = cfg(&format!(
                "bind = \"{bind}\"\npublic_url = \"{public}\"\n[oidc]\nprovider = \"static\"\nstatic_email = \"dan@example.com\""
            ));
            assert!(c.is_ok(), "{bind} {public}: {:?}", c.err());
        }
    }

    const GOOGLE: &str = "public_url = \"https://reeve.example.com\"\n[oidc]\nprovider = \"google\"\nclient_id = \"i\"\nclient_secret = \"s\"\n";

    #[test]
    fn mcp_is_off_by_default_and_needs_sign_in_to_be_turned_on() {
        assert!(!cfg("").unwrap().mcp.enabled);
        assert!(!cfg("[mcp]\nenabled = false").unwrap().mcp.enabled);
        let e = cfg("[mcp]\nenabled = true").unwrap_err();
        assert!(
            e.to_string()
                .contains("[mcp] enabled = true requires [oidc]"),
            "{e}"
        );
        assert!(
            cfg(&format!("{GOOGLE}[mcp]\nenabled = true"))
                .unwrap()
                .mcp
                .enabled
        );
    }

    #[test]
    fn mcp_refuses_unknown_keys() {
        let e = cfg("[mcp]\nsurprise = 1").unwrap_err();
        assert!(e.to_string().contains("unknown field"), "{e}");
    }

    #[test]
    fn mcp_origins_default_to_the_public_origin_and_entries_must_be_origins() {
        let c = cfg(&format!("{GOOGLE}[mcp]\nenabled = true")).unwrap();
        assert!(c.mcp_origin_allowed("https://reeve.example.com"));
        assert!(!c.mcp_origin_allowed("https://claude.ai"));
        assert!(!c.mcp_origin_allowed("https://reeve.example.com/"));
        // An explicit list replaces the default.
        let c = cfg(&format!(
            "{GOOGLE}[mcp]\nallowed_origins = [\"https://claude.ai\", \"http://localhost:6274\"]"
        ))
        .unwrap();
        assert!(c.mcp_origin_allowed("https://claude.ai"));
        assert!(c.mcp_origin_allowed("http://localhost:6274"));
        assert!(!c.mcp_origin_allowed("https://reeve.example.com"));
        for bad in [
            "https://claude.ai/",
            "claude.ai",
            "https://claude.ai/path",
            "*",
        ] {
            let e = cfg(&format!("{GOOGLE}[mcp]\nallowed_origins = [\"{bad}\"]")).unwrap_err();
            assert!(
                e.to_string().contains("[mcp] allowed_origins"),
                "{bad}: {e}"
            );
        }
    }
}
