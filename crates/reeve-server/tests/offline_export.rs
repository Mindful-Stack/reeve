//! The operator's offline export (`reeve-server export`): read-only, refuses a database at another migration, reads
//! the named database.
mod common;
use common::*;
use reeve_server::{
    check_migrations, composed_migrator, offline_options, pool_options, run_migrations,
};
use sqlx::postgres::PgConnectOptions;

/// A new database next to the harness's (`<its name>_<suffix>`) and a pool on it, migrated when `migrate`. Returns
/// the database's name: on a shared server, fixed names would collide between tests running side by side.
async fn other_db(h: &Harness, suffix: &str, migrate: bool) -> (String, sqlx::PgPool) {
    let name = h.db.sibling(suffix).await;
    let opts: PgConnectOptions = h.url.parse::<PgConnectOptions>().unwrap().database(&name);
    let pool = pool_options().connect_with(opts).await.unwrap();
    if migrate {
        run_migrations(&pool).await.unwrap();
    }
    (name, pool)
}

fn expected_latest() -> i64 {
    composed_migrator().migrations.last().unwrap().version
}

#[tokio::test]
async fn offline_connections_cannot_write() {
    let h = harness().await;
    let pool = pool_options()
        .connect_with(offline_options(&h.url, None).unwrap())
        .await
        .unwrap();
    let err = sqlx::query("CREATE TABLE nope (a int)")
        .execute(&pool)
        .await
        .unwrap_err();
    let code = err
        .as_database_error()
        .and_then(|e| e.code())
        .map(|c| c.to_string());
    assert_eq!(
        code.as_deref(),
        Some("25006"),
        "read_only_sql_transaction: {err}"
    );
    // the migration check reads only: it passes on a read-only connection and writes nothing
    check_migrations(&pool).await.unwrap();
}

#[tokio::test]
async fn the_database_override_reaches_that_database_as_the_configured_user() {
    let h = harness().await;
    let (other, _other) = other_db(&h, "other", false).await;
    let pool = pool_options()
        .connect_with(offline_options(&h.url, Some(&other)).unwrap())
        .await
        .unwrap();
    let (db, user): (String, String) =
        sqlx::query_as("SELECT current_database()::text, current_user::text")
            .fetch_one(&pool)
            .await
            .unwrap();
    let configured: PgConnectOptions = h.url.parse().unwrap();
    assert_eq!(db, other);
    assert_ne!(
        db, h.db.name,
        "the override must reach another database than the harness's"
    );
    assert_eq!(user, configured.get_username());
}

#[tokio::test]
async fn migration_check_accepts_exactly_the_binarys_migrations() {
    let h = harness().await;
    check_migrations(&h.state.pool).await.unwrap();

    // behind: the newest row removed
    let m = expected_latest();
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = $1")
        .bind(m)
        .execute(&h.state.pool)
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&h.state.pool)
        .await
        .unwrap();
    let err = check_migrations(&h.state.pool)
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(
        err,
        format!(
            "database is at migration {n}, this binary expects {m}; run `reeve-server migrate` first"
        )
    );

    // ahead or diverged: an applied version this binary does not know
    let (_, ahead) = other_db(&h, "ahead", true).await;
    sqlx::query("INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) VALUES ($1, 'newer', true, '\\x00', 0)")
        .bind(m + 1)
        .execute(&ahead)
        .await
        .unwrap();
    let err = check_migrations(&ahead).await.unwrap_err().to_string();
    assert!(
        err.contains(&format!(
            "database is at migration {}, this binary expects {m}",
            m + 1
        )) && err.contains("the binary that migrated it"),
        "{err}"
    );

    // a failed migration
    let (_, dirty) = other_db(&h, "dirty", true).await;
    sqlx::query("UPDATE _sqlx_migrations SET success = false WHERE version = $1")
        .bind(m)
        .execute(&dirty)
        .await
        .unwrap();
    let err = check_migrations(&dirty).await.unwrap_err().to_string();
    assert!(err.contains(&format!("failed migration {m}")), "{err}");

    // the same versions, but one applied from different SQL: sqlx's own migrator refuses it, and so must the check
    let (_, changed) = other_db(&h, "changed", true).await;
    sqlx::query("UPDATE _sqlx_migrations SET checksum = '\\x00' WHERE version = $1")
        .bind(m)
        .execute(&changed)
        .await
        .unwrap();
    let sqlx_says = run_migrations(&changed).await.unwrap_err();
    assert!(
        matches!(
            sqlx_says.downcast_ref::<sqlx::migrate::MigrateError>(),
            Some(sqlx::migrate::MigrateError::VersionMismatch(v)) if *v == m
        ),
        "the oracle: sqlx rejects the changed checksum: {sqlx_says:?}"
    );
    let err = check_migrations(&changed).await.unwrap_err().to_string();
    assert_eq!(
        err,
        format!(
            "database migration {m} differs from this binary's (checksum mismatch); export with the binary that migrated it"
        )
    );

    // never migrated
    let (_, empty) = other_db(&h, "empty", false).await;
    let err = check_migrations(&empty).await.unwrap_err().to_string();
    assert!(err.contains("has no migrations"), "{err}");
}

use serde_json::{Value, json};
use std::process::Output;

/// A configuration file for the binary: the harness's database and identity settings, no sign-in section.
fn config_file(dir: &std::path::Path, url: &str) -> std::path::PathBuf {
    let path = dir.join("reeve.toml");
    std::fs::write(
        &path,
        format!(
            "database_url = \"{url}\"\nkey_prefix = \"TST\"\nallowed_domains = [\"example.com\"]\nadmins = [\"dan@example.com\"]\n"
        ),
    )
    .unwrap();
    path
}

/// Runs the built `reeve-server` binary with `--config <cfg>` and `args`.
async fn reeve_server(cfg: &std::path::Path, args: &[&str]) -> Output {
    tokio::process::Command::new(env!("CARGO_BIN_EXE_reeve-server"))
        .arg("--config")
        .arg(cfg)
        .args(args)
        .env_remove("REEVE_CONFIG")
        .env("RUST_LOG", "warn")
        .output()
        .await
        .unwrap()
}

/// The text with its top-level `exported_at` line removed: the rehearsal's comparison.
fn strip_at(text: &str) -> String {
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| !l.starts_with("  \"exported_at\": "))
        .collect();
    assert_eq!(
        kept.len() + 1,
        text.lines().count(),
        "one exported_at line in {text}"
    );
    kept.join("\n")
}

async fn one_story(h: &Harness) {
    story_keyed(h, "s1").await;
}

/// A new story under its own idempotency key (a repeated key returns the existing story and creates nothing).
async fn story_keyed(h: &Harness, key: &str) {
    let (st, b) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/stories",
        Some(json!({"idempotency_key": key,"title":"Booking","kind":"story","criteria":["works"],"labels":[]})),
    )
    .await;
    assert_eq!(st, 200, "{b}");
}

#[tokio::test]
async fn export_to_stdout_is_the_cli_document() {
    let h = harness().await;
    one_story(&h).await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config_file(dir.path(), &h.url);
    let before = chrono::Utc::now();
    let out = reeve_server(&cfg, &["export"]).await;
    let after = chrono::Utc::now();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.ends_with("}\n") && !text.ends_with("\n\n") && !text.contains('\r'),
        "one trailing newline, no carriage return: {text:?}"
    );
    let doc: Value = serde_json::from_str(&text).unwrap();
    let at: chrono::DateTime<chrono::Utc> = doc["exported_at"].as_str().unwrap().parse().unwrap();
    assert!(
        before <= at && at <= after,
        "exported_at {at} is the run's own clock"
    );
    // what `reeve export` would have written: the route's value, pretty-printed
    let (_, b) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/export", None).await;
    let cli = format!("{}\n", serde_json::to_string_pretty(&b["value"]).unwrap());
    assert_eq!(strip_at(&text), strip_at(&cli));
    assert_eq!(
        doc["stories"].as_array().unwrap().len(),
        1,
        "the seeded story is there"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("\"key_prefix\""),
        "the document goes to stdout only: {stderr}"
    );
}

#[tokio::test]
async fn export_out_writes_0600_and_replaces_the_file() {
    let h = harness().await;
    one_story(&h).await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config_file(dir.path(), &h.url);
    let file = dir.path().join("live.json");
    std::fs::write(&file, "old").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    let out = reeve_server(&cfg, &["export", "--out", file.to_str().unwrap()]).await;
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty(), "nothing on stdout with --out");
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(doc["stories"].as_array().unwrap().len(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        names.len(),
        2,
        "reeve.toml and live.json, no temporary: {names:?}"
    );
}

#[tokio::test]
async fn export_refuses_another_migration_writes_nothing_and_does_not_migrate() {
    let h = harness().await;
    let m = expected_latest();
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = $1")
        .bind(m)
        .execute(&h.state.pool)
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&h.state.pool)
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let cfg = config_file(dir.path(), &h.url);
    let file = dir.path().join("live.json");
    std::fs::write(&file, "old").unwrap();
    let out = reeve_server(&cfg, &["export", "--out", file.to_str().unwrap()]).await;
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&format!(
            "database is at migration {n}, this binary expects {m}"
        )),
        "{stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "old",
        "the previous file is intact"
    );
    let still: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version = $1")
        .bind(m)
        .fetch_one(&h.state.pool)
        .await
        .unwrap();
    assert_eq!(still, 0, "export never migrates");
}

#[tokio::test]
async fn export_refuses_a_changed_migration_and_keeps_the_file() {
    let h = harness().await;
    let m = expected_latest();
    sqlx::query("UPDATE _sqlx_migrations SET checksum = '\\x00' WHERE version = $1")
        .bind(m)
        .execute(&h.state.pool)
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let cfg = config_file(dir.path(), &h.url);
    let file = dir.path().join("live.json");
    std::fs::write(&file, "old").unwrap();
    let out = reeve_server(&cfg, &["export", "--out", file.to_str().unwrap()]).await;
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&format!("database migration {m} differs")),
        "{stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "old",
        "the previous file is intact"
    );
}

#[tokio::test]
async fn export_reads_the_named_database() {
    let h = harness().await;
    one_story(&h).await;
    let (other, _other) = other_db(&h, "other", true).await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config_file(dir.path(), &h.url);
    let configured = reeve_server(&cfg, &["export"]).await;
    let named = reeve_server(&cfg, &["export", "--database", &other]).await;
    assert_eq!(
        named.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&named.stderr)
    );
    let configured: Value = serde_json::from_slice(&configured.stdout).unwrap();
    let named: Value = serde_json::from_slice(&named.stdout).unwrap();
    assert_eq!(configured["stories"].as_array().unwrap().len(), 1);
    assert_eq!(
        named["stories"],
        json!([]),
        "the other database has no story"
    );
    assert_eq!(named["users"], json!([]), "nor the harness's users");
}

#[tokio::test]
async fn export_fails_cleanly_on_a_missing_database() {
    let h = harness().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config_file(dir.path(), &h.url);
    let file = dir.path().join("live.json");
    let out = reeve_server(
        &cfg,
        &[
            "export",
            "--database",
            "no_such_db",
            "--out",
            file.to_str().unwrap(),
        ],
    )
    .await;
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no_such_db"), "{stderr}");
    assert!(
        !stderr.contains(":postgres@") && !stderr.contains("postgres://"),
        "no credentials in the error: {stderr}"
    );
    let configured: PgConnectOptions = h.url.parse().unwrap();
    assert!(
        stderr.contains(&format!(
            "connecting to database no_such_db on {}:{}",
            configured.get_host(),
            configured.get_port()
        )),
        "the error names what it tried to reach: {stderr}"
    );
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        names.len(),
        1,
        "only reeve.toml, no partial export: {names:?}"
    );
}

/// The subcommand's own pool is read-only, not only the export's transaction: `_sqlx_migrations` is swapped for a view
/// whose every read inserts a row, so the migration check (which runs outside the export's `READ ONLY` transaction)
/// succeeds and writes only if the session is writable. The binary must refuse, and the probe table stay empty.
#[tokio::test]
async fn the_subcommand_cannot_write_through_its_connection() {
    let h = harness().await;
    for stmt in [
        "CREATE TABLE probe_writes (at timestamptz NOT NULL DEFAULT now())",
        "CREATE FUNCTION probe_write() RETURNS boolean LANGUAGE plpgsql VOLATILE AS $$ BEGIN INSERT INTO probe_writes DEFAULT VALUES; RETURN true; END $$",
        "ALTER TABLE _sqlx_migrations RENAME TO _sqlx_migrations_t",
        "CREATE VIEW _sqlx_migrations AS SELECT * FROM _sqlx_migrations_t WHERE probe_write()",
    ] {
        sqlx::query(stmt).execute(&h.state.pool).await.unwrap();
    }
    // the probe works: a writable read through the view writes a row
    check_migrations(&h.state.pool).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM probe_writes")
        .fetch_one(&h.state.pool)
        .await
        .unwrap();
    assert!(n > 0, "the probe records a writable read");
    sqlx::query("TRUNCATE probe_writes")
        .execute(&h.state.pool)
        .await
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let cfg = config_file(dir.path(), &h.url);
    let out = reeve_server(&cfg, &["export"]).await;
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("read-only transaction"), "{stderr}");
    assert!(out.stdout.is_empty(), "no document");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM probe_writes")
        .fetch_one(&h.state.pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "the subcommand wrote nothing");
}

#[tokio::test]
async fn export_out_names_the_path_it_could_not_write() {
    let h = harness().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config_file(dir.path(), &h.url);
    let file = dir.path().join("missing-dir").join("live.json");
    let out = reeve_server(&cfg, &["export", "--out", file.to_str().unwrap()]).await;
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains(&format!("writing {}", file.display())),
        "{stderr}"
    );
}

/// `--snapshot`: the document is the exported snapshot's state; a later commit is not in it.
#[tokio::test]
async fn export_snapshot_reads_the_exported_snapshot_not_later_writes() {
    use sqlx::Connection;
    let h = harness().await;
    story_keyed(&h, "before-snapshot").await;
    // The exporting session, as scripts/backup.sh holds it: a REPEATABLE READ transaction kept open.
    let mut exporter = sqlx::PgConnection::connect(&h.url).await.unwrap();
    sqlx::Executor::execute(&mut exporter, "BEGIN ISOLATION LEVEL REPEATABLE READ")
        .await
        .unwrap();
    let id: String = sqlx::query_scalar("SELECT pg_export_snapshot()")
        .fetch_one(&mut exporter)
        .await
        .unwrap();
    story_keyed(&h, "after-snapshot").await; // committed after the snapshot was exported
    let dir = tempfile::tempdir().unwrap();
    let cfg = config_file(dir.path(), &h.url);
    let stories = |o: &Output| -> usize {
        let v: Value = serde_json::from_slice(&o.stdout).unwrap();
        v["stories"].as_array().unwrap().len()
    };
    let at = reeve_server(&cfg, &["export", "--snapshot", &id]).await;
    assert_eq!(
        at.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&at.stderr)
    );
    let now = reeve_server(&cfg, &["export"]).await;
    assert_eq!((stories(&at), stories(&now)), (1, 2));
    // Once the exporting transaction ends, the id is no longer importable.
    sqlx::Executor::execute(&mut exporter, "COMMIT")
        .await
        .unwrap();
    let gone = reeve_server(&cfg, &["export", "--snapshot", &id]).await;
    assert_eq!(gone.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&gone.stderr).contains("snapshot"),
        "{}",
        String::from_utf8_lossy(&gone.stderr)
    );
}

/// A malformed id is refused before connecting: the configured database does not exist, and the error is the id's.
#[tokio::test]
async fn export_snapshot_refuses_a_malformed_id_before_connecting() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config_file(dir.path(), "postgres://reeve:reeve@127.0.0.1:9/reeve");
    for bad in [
        "x'; DROP TABLE stories; --",
        "",
        "00000003-00000005",
        "0000003-00000005-1",
        "00000003-0000000g-1",
    ] {
        let out = reeve_server(&cfg, &["export", "--snapshot", bad]).await;
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{bad:?}: {err}");
        assert!(
            err.contains("is not a PostgreSQL snapshot id"),
            "{bad:?}: {err}"
        );
    }
}

/// `--at` is the document's `exported_at` and the instant urgency is computed at: before a mark's expiry it is active
/// and spreads, after it the mark is still shown but inactive and makes nothing urgent. A time that is not RFC 3339 is
/// refused before anything is read.
#[tokio::test]
async fn export_at_computes_urgency_at_that_instant() {
    let h = harness().await;
    let (st, b) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/stories",
        Some(json!({"idempotency_key":"s1","title":"Booking","kind":"story","criteria":["works"],"labels":[]})),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    let key = b["value"]["key"].as_str().unwrap().to_string();
    let (st, b) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        &format!("/api/v1/stories/{key}/urgent"),
        Some(json!({"reason":"hot"})),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    let expires: chrono::DateTime<chrono::Utc> = b["value"]["urgent_mark"]["expires_at"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let cfg = config_file(dir.path(), &h.url);
    let export_at = |at: chrono::DateTime<chrono::Utc>| {
        let cfg = cfg.clone();
        async move {
            let at = at.to_rfc3339();
            let out = reeve_server(&cfg, &["export", "--at", &at]).await;
            assert_eq!(
                out.status.code(),
                Some(0),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
            let got: chrono::DateTime<chrono::Utc> =
                doc["exported_at"].as_str().unwrap().parse().unwrap();
            assert_eq!(got.to_rfc3339(), at, "exported_at is --at");
            doc["stories"][0].clone()
        }
    };
    let before = export_at(expires - chrono::Duration::seconds(1)).await;
    assert_eq!(before["urgent_mark"]["active"], true);
    assert_eq!(before["urgent_mark"]["reason"], "hot");
    assert_eq!(
        before["urgent_sources"],
        json!([{"key": key, "item": "story"}])
    );
    let after = export_at(expires).await;
    assert_eq!(after["urgent_mark"]["active"], false);
    assert_eq!(
        after["urgent_mark"]["reason"], "hot",
        "an expired mark still shows"
    );
    assert_eq!(after["urgent_sources"], json!([]));

    let out = reeve_server(&cfg, &["export", "--at", "yesterday"]).await;
    assert_eq!(out.status.code(), Some(2), "clap refuses the value");
    assert!(out.stdout.is_empty());
}
