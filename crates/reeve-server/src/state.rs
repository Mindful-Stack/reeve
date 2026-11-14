use crate::config::{OidcSection, ServerConfig};
use chrono::{DateTime, Utc};
use reeve_identity::{IdentityConfig, OidcClient, TokenStore};
use reeve_planning::Handlers;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;

/// A browser sign-in between the redirect to the provider and the callback, keyed by its `state`.
pub struct PendingLogin {
    pub nonce: String,
    pub pkce_verifier: String,
    pub next: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct AppState {
    pub handlers: Handlers,
    pub tokens: Arc<dyn TokenStore>,
    pub identity: IdentityConfig,
    pub access: Arc<dyn reeve_identity::AccessStore>,
    pub activity: Arc<dyn reeve_activity::ActivityStore>,
    pub config: Arc<ServerConfig>,
    pub pool: PgPool,
    /// `None` when no `[oidc]` section is configured: browser sign-in is off, CLI tokens still work.
    pub oidc: Option<Arc<dyn OidcClient>>,
    /// Pending browser sign-ins (state -> login), ten-minute TTL, capped at 1 000 entries (kb/ADR-0016: one instance).
    pub logins: Arc<tokio::sync::Mutex<HashMap<String, PendingLogin>>>,
    /// Where the client files come from: embedded in the binary, except in debug builds with `REEVE_ASSETS_DIR`.
    pub assets: crate::assets::Assets,
    /// Browsable `https://` base URLs of the configured repositories, by name (no trailing `.git` or `/`), for a
    /// record's `browse_url`. Planning knows names only; a repository without such a URL is absent.
    pub repo_urls: Arc<HashMap<String, String>>,
    /// Fetches an MCP client's metadata document (kb/ADR-0029 R3). The authorize route resolves a client with
    /// `reeve_identity::resolve_client(&*s.client_fetcher, &s.config.client_policy(), …)`.
    pub client_fetcher: Arc<dyn reeve_identity::ClientDocumentFetcher>,
}

/// The `https://` base URL of each `[[repos]]` entry that has one, without credentials, port, query or fragment and
/// without a trailing `.git` or `/`. Three shapes are browsable: `https://host/org/repo(.git)`, the scp-style
/// `git@host:org/repo(.git)` and `ssh://git@host[:port]/org/repo(.git)` (production clones private repositories over
/// ssh with deploy keys); the last two map to `https://host/org/repo`. A repository's own `browse_url`, when set, takes
/// precedence; a host without a dot (an ssh host alias) gives none. A `http://` or other-scheme URL cannot be browsed
/// safely and is left out. Userinfo (`https://user:token@host/...`, a clone URL) is dropped so a credential never reaches
/// an API caller.
pub(crate) fn browsable_repo_urls(config: &ServerConfig) -> HashMap<String, String> {
    config
        .repos
        .iter()
        .filter_map(|r| {
            // An explicit address wins over one derived from the clone url (validated at load, so it is Some here).
            let base = match r.browse_url.as_deref() {
                Some(b) => explicit_browse_base(b),
                None => browse_base(r.url.as_deref()?),
            };
            Some((r.name.clone(), base?))
        })
        .collect()
}

/// The base of an explicit `browse_url`: https only (the ssh forms that `browse_base` derives from are refused).
pub(crate) fn explicit_browse_base(raw: &str) -> Option<String> {
    let u = url::Url::parse(raw.trim()).ok()?;
    if u.scheme() != "https" {
        return None;
    }
    browse_base(raw)
}

fn browse_base(raw: &str) -> Option<String> {
    let raw = raw.trim();
    // scp-style `git@host:org/repo`: no scheme, a colon after the host. A scheme URL has `://` (before any `@`).
    let https = match raw.strip_prefix("git@") {
        Some(rest) if !rest.contains("://") => {
            let (host, path) = rest.split_once(':').filter(|(h, _)| !h.is_empty())?;
            format!("https://{host}/{}", path.trim_start_matches('/'))
        }
        _ => raw.to_string(),
    };
    let mut u = url::Url::parse(&https).ok()?;
    match u.scheme() {
        "https" => {}
        // The ssh scheme's user and port say how to clone, not where to browse. A url cannot change between a special
        // scheme and another, so the https form is built from the parts.
        "ssh" => {
            let host = u.host_str().filter(|h| !h.is_empty())?;
            u = url::Url::parse(&format!("https://{host}{}", u.path())).ok()?;
        }
        _ => return None,
    }
    // A host without a dot is no DNS name (an ssh host alias such as `github-kb`, or `localhost`): a link to it cannot resolve.
    if u.host_str().is_none_or(|h| !h.contains('.')) {
        return None;
    }
    u.set_username("").ok()?;
    u.set_password(None).ok()?;
    u.set_query(None);
    u.set_fragment(None);
    let base = u.as_str().trim_end_matches('/');
    let base = base
        .strip_suffix(".git")
        .unwrap_or(base)
        .trim_end_matches('/');
    Some(base.to_string())
}

/// One strict migrator composed from the three crates' sets, so a migration deleted from any crate is detected
/// (sqlx's `VersionMissing` check) instead of being hidden by `ignore_missing`. Versions are disjoint by crate (1xxx, 2xxx, 3xxx).
pub fn composed_migrator() -> sqlx::migrate::Migrator {
    let mut all: Vec<sqlx::migrate::Migration> = Vec::new();
    for m in [
        reeve_planning::migrator(),
        reeve_identity::migrator(),
        reeve_activity::migrator(),
    ] {
        all.extend(m.migrations.iter().cloned());
    }
    all.sort_by_key(|m| m.version);
    let mut combined = reeve_planning::migrator(); // any Migrator value; its migration list is replaced below
    combined.migrations = std::borrow::Cow::Owned(all);
    combined.set_ignore_missing(false);
    combined
}

/// The server's pool options. Every connection taken from the pool is first checked for a transaction left open by
/// a cancelled `begin`, and such a connection is closed instead of handed out.
///
/// sqlx 0.8.6 counts a transaction (`transaction_depth += 1`) only after the reply to `BEGIN` arrives, and its
/// cancellation guard rolls back only when that count is above zero. A handler dropped while it waits for that reply
/// (the browser aborted the request; hyper drops the handler) therefore returns its connection to the pool inside a
/// transaction sqlx does not know about. For story readiness's `BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY` the
/// next statement on that connection fixes a snapshot that every later read on it keeps, so a session signed in
/// afterwards is "not found" there and one request in N answers 401 (the CI smoke of run 40285287399; reproduced by
/// `tests/pool.rs`), and a write on it fails as read-only.
///
/// The check replaces sqlx's acquire-time ping, so it costs no extra round trip: one simple-protocol statement, which
/// is the first statement of its own implicit transaction on a clean connection, where `statement_timestamp()` equals
/// `transaction_timestamp()` (PostgreSQL 16 docs, 9.9.5); inside a leftover transaction they differ. A leftover
/// aborted transaction makes the statement fail, which closes the connection as well.
pub fn pool_options() -> sqlx::postgres::PgPoolOptions {
    sqlx::postgres::PgPoolOptions::new()
        .test_before_acquire(false)
        .before_acquire(|conn, _meta| Box::pin(outside_any_transaction(conn)))
}

/// Whether a pooled connection is clean; `false` (close it) when a transaction is still open on it.
async fn outside_any_transaction(conn: &mut sqlx::PgConnection) -> Result<bool, sqlx::Error> {
    use sqlx::{Executor, Row};
    // A bare `&str` has no arguments, so it goes out as one simple-protocol Query message (one statement).
    let row = conn
        .fetch_one("SELECT statement_timestamp() <> transaction_timestamp()")
        .await?;
    let leftover: bool = row.try_get(0)?;
    if leftover {
        tracing::warn!(
            "closing a pooled connection left inside a transaction by a cancelled begin"
        );
    }
    Ok(!leftover)
}

pub async fn run_migrations(pool: &PgPool) -> anyhow::Result<()> {
    composed_migrator().run(pool).await?;
    Ok(())
}

/// Builds the provider adapter from `[oidc]`. Google discovers its metadata here, so a server that cannot reach
/// Google fails at startup rather than at the first sign-in.
async fn build_oidc(config: &ServerConfig) -> anyhow::Result<Option<Arc<dyn OidcClient>>> {
    let redirect = format!("{}/auth/callback", config.public_origin());
    Ok(match &config.oidc {
        None => None,
        Some(OidcSection::Google {
            client_id,
            client_secret,
        }) => Some(
            reeve_identity::make_google_oidc(reeve_identity::GoogleOidcConfig {
                client_id: client_id.clone(),
                client_secret: client_secret.clone(),
                redirect_url: redirect,
            })
            .await?,
        ),
        #[cfg(feature = "static-oidc")]
        Some(OidcSection::Static { static_email }) => {
            Some(reeve_identity::make_static_oidc(static_email, &redirect))
        }
    })
}

/// The fetcher of client metadata documents, chosen as `build_oidc` chooses the provider: the loopback-only one
/// for the static provider (which loads only on loopback, in builds with `static-oidc`), the production one in
/// every other case. `ServerConfig::validate` admits a loopback client host under the same condition.
#[cfg_attr(not(feature = "static-oidc"), allow(unused_variables))]
fn build_client_fetcher(
    config: &ServerConfig,
) -> anyhow::Result<Arc<dyn reeve_identity::ClientDocumentFetcher>> {
    #[cfg(feature = "static-oidc")]
    if matches!(config.oidc, Some(OidcSection::Static { .. })) {
        return reeve_identity::make_loopback_client_fetcher();
    }
    reeve_identity::make_client_fetcher()
}

/// The planning handlers over `pool`, with the instance settings from `config`. The one place they are built, for the
/// server (`build_state`) and the offline export (`ExportSources::offline`), so the two cannot differ in settings.
pub(crate) fn handlers_for(
    config: &ServerConfig,
    pool: PgPool,
    activity: Arc<dyn reeve_activity::ActivityStore>,
) -> Handlers {
    // The activity store implements planning's ActivitySink itself; upcast the Arc (Rust >= 1.86 trait upcasting).
    let sink: Arc<dyn reeve_planning::ActivitySink> = activity;
    Handlers {
        ports: reeve_planning::make_planning_ports(pool.clone()),
        activity: sink,
        pool,
        settings: reeve_planning::Settings {
            key_prefix: config.key_prefix.clone(),
            horizon_buckets: config.horizon_buckets.clone(),
            repos: config.repos.iter().map(|r| r.name.clone()).collect(),
            done_window_days: config.board.done_window_days,
            sprint_length_days: config.sprints.length_days,
            upkeep_line: config.sprints.upkeep_line,
        },
    }
}

impl AppState {
    /// What `export_document` reads, from this state (cheap clones: a pool handle, `Arc`s and the identity lists).
    pub fn export_sources(&self) -> crate::export::ExportSources {
        crate::export::ExportSources {
            pool: self.pool.clone(),
            handlers: self.handlers.clone(),
            activity: self.activity.clone(),
            identity: self.identity.clone(),
            access: self.access.clone(),
        }
    }
}

pub async fn build_state(config: ServerConfig, pool: PgPool) -> anyhow::Result<AppState> {
    config.validate()?;
    let oidc = build_oidc(&config).await?;
    let client_fetcher = build_client_fetcher(&config)?;
    let activity = reeve_activity::make_activity_store(pool.clone());
    let handlers = handlers_for(&config, pool.clone(), activity.clone());
    // A directory override is a development convenience; release builds serve the embedded files only.
    #[cfg(debug_assertions)]
    let assets = match std::env::var_os("REEVE_ASSETS_DIR") {
        Some(dir) => crate::assets::Assets::Dir(dir.into()),
        None => crate::assets::Assets::Embedded,
    };
    #[cfg(not(debug_assertions))]
    let assets = crate::assets::Assets::Embedded;
    let repo_urls = Arc::new(browsable_repo_urls(&config));
    Ok(AppState {
        handlers,
        tokens: reeve_identity::make_token_store(pool.clone()),
        identity: config.identity(),
        access: reeve_identity::make_access_store(pool.clone()),
        activity,
        config: Arc::new(config),
        pool,
        oidc,
        logins: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        assets,
        repo_urls,
        client_fetcher,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_applied_version_missing_from_the_composed_list_fails() {
        let db = reeve_testdb::database(&composed_migrator()).await;
        let pool = db.pool.clone();
        run_migrations(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) VALUES (1999, 'gone', true, '\\x00', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let err = run_migrations(&pool).await.unwrap_err();
        assert!(
            matches!(
                err.downcast_ref::<sqlx::migrate::MigrateError>(),
                Some(sqlx::migrate::MigrateError::VersionMissing(1999))
            ),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn composed_versions_are_unique_and_in_their_crate_ranges() {
        let all: Vec<i64> = composed_migrator()
            .migrations
            .iter()
            .map(|m| m.version)
            .collect();
        let unique: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(unique.len(), all.len(), "duplicate versions: {all:?}");
        let range = |m: sqlx::migrate::Migrator, lo: i64| {
            m.migrations
                .iter()
                .all(|m| (lo..lo + 1000).contains(&m.version))
        };
        assert!(range(reeve_planning::migrator(), 1000));
        assert!(range(reeve_identity::migrator(), 2000));
        assert!(range(reeve_activity::migrator(), 3000));
    }

    /// Every browsable shape gives the same https base: no user, password, port, `.git` or trailing slash. Catches a
    /// credential or an ssh port reaching an API caller, and an ssh url dropped for lack of an https scheme.
    #[test]
    fn browse_base_derives_https_from_https_scp_and_ssh_urls() {
        let same = "https://github.com/acme/orion";
        for raw in [
            "https://github.com/acme/orion",
            "https://github.com/acme/orion.git",
            "https://github.com/acme/orion/",
            "https://bot:SECRET@github.com/acme/orion.git?x=1#y",
            "git@github.com:acme/orion.git",
            "git@github.com:acme/orion",
            "  git@github.com:acme/orion.git/ ",
            "ssh://git@github.com/acme/orion.git",
            "ssh://git@github.com:2222/acme/orion.git",
            "ssh://git:SECRET@github.com:2222/acme/orion",
        ] {
            assert_eq!(browse_base(raw).as_deref(), Some(same), "{raw}");
        }
        assert_eq!(
            browse_base("git@git.example.com:team/sub/repo.git").as_deref(),
            Some("https://git.example.com/team/sub/repo")
        );
    }

    /// Anything that cannot be browsed is left out: plain http, other schemes, a missing host, a missing path.
    #[test]
    fn browse_base_refuses_what_cannot_be_browsed() {
        for raw in [
            "",
            "http://git.example.com/acme/plain",
            "ftp://git.example.com/acme/plain",
            "file:///srv/git/orion",
            "git@:acme/orion.git",
            "git@github.com",
            "ssh://git@/acme/orion",
            // No dot in the host: an ssh host alias (or localhost), not a web host.
            "git@github-knowledge-base:Acme/kb.git",
            "ssh://git@github-knowledge-base/Acme/kb.git",
            "https://localhost/acme/orion",
            "orion",
        ] {
            assert_eq!(browse_base(raw), None, "{raw:?}");
        }
    }
}
