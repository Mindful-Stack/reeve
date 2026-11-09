//! The whole-instance export, shared by `GET /api/v1/export` (admin, interactive token) and the operator's offline
//! `reeve-server export`, so the two documents cannot drift. Every context's export reader runs on ONE
//! `REPEATABLE READ READ ONLY` transaction, so the document is one snapshot; it holds emails and roles, never token
//! hashes, secrets or configuration. The restore source is a `pg_dump` archive, not this document (deploy/README.md).
use crate::{config::ServerConfig, error::AppError};
use anyhow::{Context, anyhow, bail};
use chrono::{DateTime, Utc};
use reeve_identity::IdentityConfig;
use reeve_planning::Handlers;
use reeve_protocol::{ActivityEntryDto, ExportDto, UserDto};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::sync::Arc;

/// The activity page size; the loop reads pages until one comes back short.
const PAGE: i64 = 500;

/// A PostgreSQL exported snapshot id (`pg_export_snapshot()`, `%08X-%08X-%d` in PostgreSQL 16, e.g.
/// `00000003-00000005-1`). Validated before use because `SET TRANSACTION SNAPSHOT` takes no bind parameters: the id is
/// spliced into the statement, and this shape admits no quote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotId(String);

impl SnapshotId {
    pub fn parse(raw: &str) -> anyhow::Result<Self> {
        let hex8 = |s: &str| {
            s.len() == 8
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
        };
        let parts: Vec<&str> = raw.split('-').collect();
        let ok = parts.len() == 3
            && hex8(parts[0])
            && hex8(parts[1])
            && !parts[2].is_empty()
            && parts[2].bytes().all(|b| b.is_ascii_digit());
        if !ok {
            bail!(
                "{raw:?} is not a PostgreSQL snapshot id (pg_export_snapshot() prints e.g. 00000003-00000005-1)"
            );
        }
        Ok(Self(raw.to_string()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What the export reads: the three contexts' export readers and the configuration that decides roles.
#[derive(Clone)]
pub struct ExportSources {
    pub(crate) pool: PgPool,
    pub(crate) handlers: Handlers,
    pub(crate) activity: Arc<dyn reeve_activity::ActivityStore>,
    pub(crate) identity: IdentityConfig,
}

impl ExportSources {
    /// The sources of an offline export, built from the configuration and a pool the way `build_state` builds them,
    /// without the OIDC provider (Google's discovery would need the network) or anything else only a server needs.
    /// The caller must have validated the configuration (`ServerConfig::validate`); this does not.
    pub fn offline(config: &ServerConfig, pool: PgPool) -> Self {
        let activity = reeve_activity::make_activity_store(pool.clone());
        Self {
            handlers: crate::state::handlers_for(config, pool.clone(), activity.clone()),
            activity,
            identity: config.identity(),
            pool,
        }
    }
}

/// The export document at `exported_at`, which the caller takes from its clock (the request time, or the
/// subcommand's start), never this function.
pub async fn export_document(
    src: &ExportSources,
    exported_at: DateTime<Utc>,
) -> Result<ExportDto, AppError> {
    export_document_in(src, exported_at, None).await
}

/// The export document read inside `snapshot` when one is given (`SET TRANSACTION SNAPSHOT`, the transaction's first
/// statement), so a dump taken with `pg_dump --snapshot` of the same id and this document are one state.
pub async fn export_document_in(
    src: &ExportSources,
    exported_at: DateTime<Utc>,
    snapshot: Option<&SnapshotId>,
) -> Result<ExportDto, AppError> {
    let mut tx = src
        .pool
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await?;
    if let Some(id) = snapshot {
        // A bare &str goes out as one simple-protocol statement: no prepare, nothing before it in the transaction.
        sqlx::Executor::execute(
            &mut *tx,
            format!("SET TRANSACTION SNAPSHOT '{}'", id.as_str()).as_str(),
        )
        .await?;
    }
    // The first query: without a snapshot, PostgreSQL takes one here, not at BEGIN.
    let urgency = src.handlers.urgency_in(&mut tx, exported_at).await?;
    let initiatives = src.handlers.export_initiatives(&mut tx, &urgency).await?;
    let epics = src.handlers.export_epics(&mut tx, &urgency).await?;
    let stories = src.handlers.export_stories(&mut tx, &urgency).await?;
    let sprints = src.handlers.export_sprints(&mut tx).await?;
    let decisions = src.handlers.export_decisions(&mut tx).await?;
    let comments = src.handlers.export_comments(&mut tx).await?;
    let users = reeve_identity::list_users_in(&mut tx)
        .await?
        .into_iter()
        .map(|u| UserDto {
            role: src.identity.role_for(&u.email),
            email: u.email,
            display_name: u.display_name,
            disabled: u.disabled,
            created_at: u.created_at,
        })
        .collect();
    let mut activity: Vec<ActivityEntryDto> = Vec::new();
    loop {
        let after = activity.last().map(|e| e.id).unwrap_or(0);
        let page = src
            .activity
            .list_all_in(&mut tx, after, PAGE)
            .await
            .map_err(|reeve_activity::ActivityError::Db(e)| AppError::from(e))?;
        let done = (page.len() as i64) < PAGE;
        activity.extend(page.into_iter().map(crate::routes::activity::to_dto));
        if done {
            break;
        }
    }
    tx.commit().await?;
    Ok(ExportDto {
        exported_at,
        key_prefix: src.handlers.settings.key_prefix.clone(),
        initiatives,
        epics,
        stories,
        sprints,
        decisions,
        comments,
        activity,
        users,
    })
}

/// The document as `reeve export` writes it: through `serde_json::Value` (sorted keys), pretty-printed, one trailing
/// newline. An offline export and a CLI export of one snapshot therefore differ only on the `exported_at` line.
pub fn export_text(doc: &ExportDto) -> String {
    let value = serde_json::to_value(doc).expect("an ExportDto always serialises");
    format!(
        "{}\n",
        serde_json::to_string_pretty(&value).expect("a JSON value always serialises")
    )
}

/// The offline export's connection: `database_url` with only its database replaced by `database` (credentials, host,
/// port and TLS settings stay as configured, so no password goes on a command line), and every transaction read-only.
pub fn offline_options(
    database_url: &str,
    database: Option<&str>,
) -> anyhow::Result<PgConnectOptions> {
    let mut o: PgConnectOptions = database_url
        .parse()
        .map_err(|e| anyhow!("database_url is not a URL sqlx can connect to: {e}"))?;
    if let Some(d) = database {
        if d.is_empty() {
            bail!("--database must name a database");
        }
        o = o.database(d);
    }
    Ok(o.options([("default_transaction_read_only", "on")]))
}

/// Refuses unless the database's applied migrations are exactly the ones this binary carries (same versions, same
/// checksums, all successful), so an export never reads a schema it does not know. The checksum comparison is the one
/// sqlx's migrator makes before it applies anything. Reads `_sqlx_migrations`; never migrates.
pub async fn check_migrations(pool: &PgPool) -> anyhow::Result<()> {
    let migrator = crate::state::composed_migrator();
    let expected: Vec<i64> = migrator.migrations.iter().map(|m| m.version).collect();
    let checksums: std::collections::HashMap<i64, &[u8]> = migrator
        .migrations
        .iter()
        .map(|m| (m.version, &*m.checksum))
        .collect();
    let m = *expected.last().expect("the binary carries migrations");
    let present: bool = sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations') IS NOT NULL")
        .fetch_one(pool)
        .await
        .context("checking for _sqlx_migrations")?;
    if !present {
        bail!(
            "database has no migrations, this binary expects {m}; run `reeve-server migrate` first"
        );
    }
    let applied: Vec<(i64, bool, Vec<u8>)> =
        sqlx::query_as("SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .context("reading applied migrations from _sqlx_migrations")?;
    if let Some((v, ..)) = applied.iter().find(|(_, ok, _)| !ok) {
        bail!("database has a failed migration {v}; fix it and run `reeve-server migrate` first");
    }
    if let Some((v, ..)) = applied.iter().find(|(v, _, sum)| {
        checksums
            .get(v)
            .is_some_and(|known| *known != sum.as_slice())
    }) {
        bail!(
            "database migration {v} differs from this binary's (checksum mismatch); export with the binary that migrated it"
        );
    }
    let versions: Vec<i64> = applied.iter().map(|(v, ..)| *v).collect();
    if versions == expected {
        return Ok(());
    }
    let n = versions
        .last()
        .map(|v| v.to_string())
        .unwrap_or_else(|| "none".into());
    if expected.starts_with(&versions) {
        bail!(
            "database is at migration {n}, this binary expects {m}; run `reeve-server migrate` first"
        );
    }
    bail!(
        "database is at migration {n}, this binary expects {m}; export with the binary that migrated it"
    );
}

/// `reeve-server export`: connect read-only to the configured database (or `database` on the same server), refuse a
/// database at another migration, and return the document's text. The caller writes it; nothing is written here.
pub async fn offline_export(
    config: &ServerConfig,
    database: Option<&str>,
    snapshot: Option<&SnapshotId>,
    exported_at: DateTime<Utc>,
) -> anyhow::Result<String> {
    let options = offline_options(&config.database_url, database)?;
    // Names the database, host and port it tried; never the user or the password.
    let target = format!(
        "connecting to database {} on {}:{}",
        options.get_database().unwrap_or("(default)"),
        options.get_host(),
        options.get_port()
    );
    let pool = crate::state::pool_options()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect_with(options)
        .await
        .context(target)?;
    check_migrations(&pool).await?;
    let doc = export_document_in(
        &ExportSources::offline(config, pool.clone()),
        exported_at,
        snapshot,
    )
    .await
    .map_err(|e| {
        anyhow!(
            "export failed{}: {e:?}",
            snapshot
                .map(|s| format!(" (snapshot {})", s.as_str()))
                .unwrap_or_default()
        )
    })?;
    pool.close().await;
    Ok(export_text(&doc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_ids_are_postgres_shaped() {
        for ok in ["00000003-00000005-1", "0000000A-0000001B-12"] {
            assert_eq!(SnapshotId::parse(ok).unwrap().as_str(), ok);
        }
        for bad in [
            "",
            "00000003-00000005-1'",
            "00000003-00000005-",
            "00000003-0000001b-1",
            "00000003-00000005-1-2",
        ] {
            assert!(SnapshotId::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn export_text_is_the_cli_shape() {
        let doc = ExportDto {
            exported_at: "2026-10-26T10:00:00Z".parse().unwrap(),
            key_prefix: "TST".into(),
            initiatives: vec![],
            epics: vec![],
            stories: vec![],
            sprints: vec![],
            decisions: vec![],
            comments: vec![],
            activity: vec![],
            users: vec![],
        };
        let text = export_text(&doc);
        // what `reeve export` writes: the document as a JSON value, pretty-printed, one trailing newline
        let cli = format!(
            "{}\n",
            serde_json::to_string_pretty(&serde_json::to_value(&doc).unwrap()).unwrap()
        );
        assert_eq!(text, cli);
        assert!(text.ends_with("}\n") && !text.ends_with("\n\n"));
        let at: Vec<&str> = text
            .lines()
            .filter(|l| l.starts_with("  \"exported_at\": "))
            .collect();
        assert_eq!(
            at.len(),
            1,
            "exactly one top-level exported_at line: {text}"
        );
        let keys: Vec<&str> = text
            .lines()
            .filter(|l| l.starts_with("  \"") && !l.starts_with("   "))
            .map(|l| l.trim_start().split('"').nth(1).unwrap())
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "top-level keys in the CLI's (sorted) order");
    }

    #[test]
    fn offline_options_replace_only_the_database_and_are_read_only() {
        let url = "postgres://u:p@db.internal:5433/reeve?sslmode=prefer";
        let o = offline_options(url, Some("reeve_rehearsal")).unwrap();
        let base: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
        assert_eq!(o.get_database(), Some("reeve_rehearsal"));
        assert_eq!(
            (o.get_host(), o.get_port(), o.get_username()),
            (base.get_host(), base.get_port(), base.get_username()),
            "host, port and user come from database_url"
        );
        assert!(
            o.get_options()
                .unwrap_or("")
                .contains("-c default_transaction_read_only=on"),
            "{:?}",
            o.get_options()
        );
        let o = offline_options(url, None).unwrap();
        assert_eq!(o.get_database(), base.get_database());
        let url = "postgres://u:p@db.internal/reeve?sslmode=require&options=-c%20search_path%3Dx";
        let o = offline_options(url, None).unwrap();
        let base: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
        assert!(
            matches!(o.get_ssl_mode(), sqlx::postgres::PgSslMode::Require),
            "{:?}",
            o.get_ssl_mode()
        );
        assert_eq!(
            format!("{:?}", o.get_ssl_mode()),
            format!("{:?}", base.get_ssl_mode())
        );
        let opts = o.get_options().unwrap_or("");
        assert!(
            opts.contains("search_path=x"),
            "appended, not replaced: {opts}"
        );
        assert!(opts.contains("default_transaction_read_only=on"), "{opts}");
        assert!(
            offline_options(url, Some("")).is_err(),
            "an empty name is refused"
        );
        let err = offline_options("host=db user=reeve", None).unwrap_err();
        assert!(err.to_string().contains("database_url"), "{err}");
    }
}
