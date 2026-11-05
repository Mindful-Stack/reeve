//! Test databases for the planner's tests (test support, not a bounded context; a dev-dependency only).
//!
//! With `REEVE_TEST_DATABASE_URL` set (scripts/test.sh starts one PostgreSQL 16 per run and exports it), every call
//! creates a database of its own, cloned from a template that the migration set's first caller builds by running the
//! committed migrator. Unset, every call starts a PostgreSQL 16 container of its own, as the tests always did. Nothing
//! is ever skipped: an unreachable server fails the test.
use sha2::{Digest, Sha256};
use sqlx::{Connection, Executor, PgConnection, PgPool, migrate::Migrator};
use std::time::{Duration, Instant};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner},
};

/// The run's server, exported by scripts/test.sh.
pub const URL_VAR: &str = "REEVE_TEST_DATABASE_URL";

/// The server settings of the run's server and of every container. scripts/test.sh's `settings=(…)` line must list
/// the same (a test checks it). Durability is off (no test exercises crash recovery); pg_stat_statements is preloaded
/// with room for every test database's entries, so a counted measurement is never evicted.
pub const SETTINGS: [&str; 6] = [
    "fsync=off",
    "synchronous_commit=off",
    "full_page_writes=off",
    "max_connections=1000",
    "shared_preload_libraries=pg_stat_statements",
    "pg_stat_statements.max=100000",
];

enum Source {
    Run(String),
    Container,
}

fn source(var: Option<String>) -> Source {
    match var {
        Some(url) => Source::Run(url),
        None => Source::Container,
    }
}

fn source_from_env() -> Source {
    source(std::env::var_os(URL_VAR).map(|v| {
        v.into_string()
            .unwrap_or_else(|_| panic!("{URL_VAR} must be valid UTF-8"))
    }))
}

/// `reeve_tmpl_` and the first 16 hex characters of a SHA-256 over the set's versions and checksums.
fn template_name(m: &Migrator) -> String {
    let mut h = Sha256::new();
    for mig in m.iter() {
        h.update(mig.version.to_be_bytes());
        h.update(&*mig.checksum);
    }
    format!("reeve_tmpl_{}", &hex::encode(h.finalize())[..16])
}

/// `base` with its database replaced by `database`.
fn url_for(base: &str, database: &str) -> String {
    let mut u = url::Url::parse(base).unwrap_or_else(|e| panic!("{URL_VAR} is not a URL: {e}"));
    u.set_path(&format!("/{database}"));
    u.to_string()
}

/// A test's database. Dropping it leaves the database on the run's server (the server is discarded with the run)
/// or, for `database()` and `empty_database()` in container mode, stops the test's own container. The container of
/// `own_container()` is the caller's.
pub struct TestDb {
    /// A pool on the test's database.
    pub pool: PgPool,
    /// The test's database as a URL, for configuration files and server processes.
    pub url: String,
    /// The test's database's name.
    pub name: String,
    /// Where `CREATE DATABASE` goes: the run server's maintenance database, or the container's.
    admin: String,
    _container: Option<ContainerAsync<Postgres>>,
}

// Exists so that moving a field out of a TestDb is a compile error (E0509): `database(&m).await.pool` would drop the
// fallback container under a live pool. Callers keep the TestDb bound and clone its pool.
impl Drop for TestDb {
    fn drop(&mut self) {}
}

impl TestDb {
    /// A new, empty database next to this one, named `<this database>_<suffix>`, for a test that needs several.
    /// Returns its name; connect by parsing `db.url` into `PgConnectOptions` and setting `.database(&name)`.
    pub async fn sibling(&self, suffix: &str) -> String {
        assert!(
            !suffix.is_empty()
                && suffix
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
            "a sibling suffix is lowercase letters, digits and underscores: {suffix:?}"
        );
        let name = format!("{}_{suffix}", self.name);
        assert!(
            name.len() <= 63,
            "a sibling's name fits PostgreSQL's 63-byte identifier limit, which truncates silently: {name:?}"
        );
        create_database(&self.admin, &name, "template0").await;
        name
    }
}

/// `url` without its password, for messages.
fn redacted(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut u) => {
            let _ = u.set_password(None);
            u.to_string()
        }
        Err(_) => "(not a URL)".to_string(),
    }
}

async fn admin_connection(admin: &str) -> PgConnection {
    PgConnection::connect(admin).await.unwrap_or_else(|e| {
        panic!(
            "{URL_VAR} names a server this test cannot reach ({}): {e}",
            redacted(admin)
        )
    })
}

/// `sql`, retried on SQLSTATE 55006 ("database is being accessed by other users") until about 10 s have passed; any
/// other error, or 55006 after that, panics. PostgreSQL itself waits about 5 s for the other sessions to leave before
/// raising 55006, so in practice this is a second attempt of about 5 s more.
async fn execute_retrying(conn: &mut PgConnection, sql: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match conn.execute(sql).await {
            Ok(_) => return,
            Err(e)
                if e.as_database_error().and_then(|d| d.code()).as_deref() == Some("55006")
                    && Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(100)).await
            }
            Err(e) => panic!("{sql}: {e}"),
        }
    }
}

/// `CREATE DATABASE name TEMPLATE template`, retried on SQLSTATE 55006 (`execute_retrying`). Clones of one template may run concurrently (PostgreSQL 16 takes a
/// share lock on the source); 55006 only means a session on the template, which `ALLOW_CONNECTIONS false` keeps out.
async fn create_database(admin: &str, name: &str, template: &str) {
    let mut conn = admin_connection(admin).await;
    execute_retrying(
        &mut conn,
        &format!("CREATE DATABASE {name} TEMPLATE {template}"),
    )
    .await;
}

/// Publishes the built scratch database `building` as the template `name`: marks it a template closed to new
/// connections, ends any session still on it, then renames it. Only the builder connects to `building` (under the
/// template's advisory lock), and its pool is closed by now, so a session found here should not exist; the rename would
/// otherwise wait about 5 s for it and fail with 55006 (seen under the full suite's load, cause unknown). Each one is
/// reported on stderr (shown for a failing test, or with nextest's `--success-output`) so the culprit can be found.
async fn publish(lock: &mut PgConnection, building: &str, name: &str) {
    lock.execute(
        format!("ALTER DATABASE {building} WITH IS_TEMPLATE true ALLOW_CONNECTIONS false").as_str(),
    )
    .await
    .unwrap();
    let lingering: Vec<String> = sqlx::query_scalar(
        "SELECT format('pid %s, %s, application %L, client %s, state %s, wait %s, connected %s', pid, backend_type, \
         application_name, coalesce(client_addr::text, 'local'), coalesce(state, '-'), \
         coalesce(wait_event_type || '/' || wait_event, '-'), backend_start) \
         FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()",
    )
    .bind(building)
    .fetch_all(&mut *lock)
    .await
    .unwrap();
    if !lingering.is_empty() {
        eprintln!(
            "reeve-testdb: ending {} session(s) still on {building} before publishing it: {}",
            lingering.len(),
            lingering.join("; ")
        );
        // With a timeout, each call waits for its backend to exit (up to 5 s), not only for the signal to be sent.
        sqlx::query(
            "SELECT pg_terminate_backend(pid, 5000) FROM pg_stat_activity \
             WHERE datname = $1 AND pid <> pg_backend_pid()",
        )
        .bind(building)
        .execute(&mut *lock)
        .await
        .unwrap();
    }
    execute_retrying(lock, &format!("ALTER DATABASE {building} RENAME TO {name}")).await;
}

/// The migration set's template on the run's server, built by the first caller under an advisory lock taken on the
/// maintenance database (advisory locks belong to a database, so every process must lock the same one). A template
/// exists under its final name only once complete and closed: it is built as `<name>_building`, marked a template
/// and closed to connections, and only then renamed, the one step that publishes it. A builder that fails or dies at
/// any point leaves at most the `_building` database, which the next one drops.
async fn template(admin: &str, migrator: &Migrator) -> String {
    let name = template_name(migrator);
    let key = u64::from_str_radix(&name["reeve_tmpl_".len()..], 16).unwrap() as i64;
    let mut lock = admin_connection(admin).await;
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(key)
        .execute(&mut lock)
        .await
        .unwrap();
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(&name)
            .fetch_one(&mut lock)
            .await
            .unwrap();
    if !exists {
        let building = format!("{name}_building");
        let leftover: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
                .bind(&building)
                .fetch_one(&mut lock)
                .await
                .unwrap();
        if leftover {
            // A dead builder's: it may already be marked a template, which DROP DATABASE refuses.
            lock.execute(format!("ALTER DATABASE {building} WITH IS_TEMPLATE false").as_str())
                .await
                .unwrap();
            lock.execute(format!("DROP DATABASE {building} WITH (FORCE)").as_str())
                .await
                .unwrap();
        }
        lock.execute(format!("CREATE DATABASE {building} TEMPLATE template0").as_str())
            .await
            .unwrap();
        let pool = PgPool::connect(&url_for(admin, &building)).await.unwrap();
        // A failure here panics; the lock goes with this session and the scratch database stays unpublished.
        migrator
            .run(&pool)
            .await
            .unwrap_or_else(|e| panic!("migrating the template {name}: {e}"));
        pool.close().await;
        publish(&mut lock, &building, &name).await;
    }
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(key)
        .execute(&mut lock)
        .await
        .unwrap();
    name
}

/// A new database on the run's server at `admin`: a clone of `migrator`'s template, or empty when `None`.
async fn on_run_server(admin: &str, migrator: Option<&Migrator>) -> TestDb {
    let source = match migrator {
        Some(m) => template(admin, m).await,
        None => "template0".to_string(),
    };
    let name = format!("reeve_test_{}", uuid::Uuid::new_v4().simple());
    create_database(admin, &name, &source).await;
    let url = url_for(admin, &name);
    let pool = PgPool::connect(&url).await.unwrap();
    TestDb {
        pool,
        url,
        name,
        admin: admin.to_string(),
        _container: None,
    }
}

/// PostgreSQL 16 with `SETTINGS`, its connection options and its maintenance database's URL.
async fn start_container() -> (ContainerAsync<Postgres>, String) {
    let mut cmd = vec!["postgres".to_string()];
    for s in SETTINGS {
        cmd.push("-c".into());
        cmd.push(s.into());
    }
    let c = Postgres::default()
        .with_tag("16")
        .with_cmd(cmd)
        .start()
        .await
        .expect("docker must be able to start postgres:16");
    let url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        c.get_host().await.unwrap(),
        c.get_host_port_ipv4(5432).await.unwrap()
    );
    (c, url)
}

/// A fresh database migrated by `migrator`: a clone of its template on the run's server, or a container of its own.
pub async fn database(migrator: &Migrator) -> TestDb {
    match source_from_env() {
        Source::Run(url) => on_run_server(&url, Some(migrator)).await,
        Source::Container => in_container(Some(migrator)).await,
    }
}

/// A fresh, empty database, for a test that runs a partial migrator itself.
pub async fn empty_database() -> TestDb {
    match source_from_env() {
        Source::Run(url) => on_run_server(&url, None).await,
        Source::Container => in_container(None).await,
    }
}

/// A PostgreSQL 16 container of the caller's own, whatever the environment says, with its `postgres` database
/// migrated by `migrator`: for a test that controls the server itself (stops it, runs commands inside it). The
/// `TestDb` is dead once the caller stops or drops the container; bind both (`let (c, db) = …`), never `let (_, db)`.
pub async fn own_container(migrator: &Migrator) -> (ContainerAsync<Postgres>, TestDb) {
    let (c, url) = start_container().await;
    let pool = PgPool::connect(&url).await.unwrap();
    migrator.run(&pool).await.unwrap();
    let db = TestDb {
        pool,
        url: url.clone(),
        name: "postgres".into(),
        admin: url,
        _container: None,
    };
    (c, db)
}

async fn in_container(migrator: Option<&Migrator>) -> TestDb {
    let (c, url) = start_container().await;
    let pool = PgPool::connect(&url).await.unwrap();
    if let Some(m) = migrator {
        m.run(&pool).await.unwrap();
    }
    TestDb {
        pool,
        url: url.clone(),
        name: "postgres".into(),
        admin: url,
        _container: Some(c),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A migrator over `files` (name, SQL), written to a fresh directory.
    async fn migrator(files: &[(&str, &str)]) -> (tempfile::TempDir, Migrator) {
        let dir = tempfile::tempdir().unwrap();
        for (name, sql) in files {
            std::fs::write(dir.path().join(name), sql).unwrap();
        }
        let m = Migrator::new(dir.path()).await.unwrap();
        (dir, m)
    }

    #[tokio::test]
    async fn template_names_follow_the_migration_set() {
        let (_a, one) = migrator(&[("1_a.sql", "CREATE TABLE a (id int);")]).await;
        let (_b, same) = migrator(&[("1_a.sql", "CREATE TABLE a (id int);")]).await;
        let (_c, changed) = migrator(&[("1_a.sql", "CREATE TABLE a (id bigint);")]).await;
        let (_r, renamed) = migrator(&[("1_renamed.sql", "CREATE TABLE a (id int);")]).await;
        let (_d, more) = migrator(&[
            ("1_a.sql", "CREATE TABLE a (id int);"),
            ("2_b.sql", "CREATE TABLE b (id int);"),
        ])
        .await;
        let name = template_name(&one);
        assert_eq!(name.len(), "reeve_tmpl_".len() + 16, "{name}");
        assert!(name.starts_with("reeve_tmpl_"));
        assert_eq!(name, template_name(&same), "same files, same template");
        assert_eq!(
            name,
            template_name(&renamed),
            "the description is deliberately ignored"
        );
        assert_ne!(name, template_name(&changed), "changed SQL, new template");
        assert_ne!(name, template_name(&more), "added migration, new template");
    }

    #[test]
    fn the_variable_chooses_the_source() {
        assert!(matches!(source(None), Source::Container));
        match source(Some("postgres://u:p@h:5/postgres".into())) {
            Source::Run(url) => assert_eq!(url, "postgres://u:p@h:5/postgres"),
            Source::Container => panic!("a set variable must name the run's server"),
        }
    }

    #[test]
    fn scripts_test_sh_starts_the_server_with_these_settings() {
        let script = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test.sh"),
        )
        .unwrap();
        let line = script
            .lines()
            .find(|l| l.starts_with("settings=("))
            .expect("scripts/test.sh has a settings=(…) line");
        assert_eq!(line, format!("settings=({})", SETTINGS.join(" ")));
    }

    #[test]
    fn database_urls_keep_everything_but_the_database() {
        assert_eq!(
            url_for(
                "postgres://u:p@127.0.0.1:5432/postgres?sslmode=disable",
                "reeve_test_x"
            ),
            "postgres://u:p@127.0.0.1:5432/reeve_test_x?sslmode=disable"
        );
        assert_eq!(
            url_for("postgresql://u:p%40s%2Fx@h:5/postgres", "d"),
            "postgresql://u:p%40s%2Fx@h:5/d"
        );
        assert_eq!(
            url_for("postgres://u:p@h:5?sslmode=disable", "d"),
            "postgres://u:p@h:5/d?sslmode=disable"
        );
    }

    /// A PostgreSQL 16 container standing in for the run's server, and its maintenance database's URL.
    async fn server() -> (ContainerAsync<Postgres>, String) {
        let (c, url) = start_container().await;
        (c, url)
    }

    async fn tables(pool: &PgPool) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT tablename::text FROM pg_tables WHERE schemaname = 'public' ORDER BY 1",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    async fn templates(url: &str) -> Vec<String> {
        let mut c = PgConnection::connect(url).await.unwrap();
        sqlx::query_scalar(
            "SELECT datname::text FROM pg_database WHERE datname LIKE 'reeve_tmpl_%' ORDER BY 1",
        )
        .fetch_all(&mut c)
        .await
        .unwrap()
    }

    // The callers are separate sessions in one process; the cross-process case rests on the same server-side lock and
    // is exercised by the full suite's runs.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn concurrent_callers_build_one_template() {
        let (_c, url) = server().await;
        // The sleep keeps the first builder inside its migration while the others arrive.
        let (_d, m) =
            migrator(&[("1_a.sql", "CREATE TABLE a (id int); SELECT pg_sleep(1);")]).await;
        let m = std::sync::Arc::new(m);
        let start = std::sync::Arc::new(tokio::sync::Barrier::new(8));
        // Collected, so all eight are spawned now; a lazy iterator awaited in a loop would spawn them one at a time.
        let calls: Vec<_> = (0..8)
            .map(|_| {
                let (url, m, start) = (url.clone(), m.clone(), start.clone());
                tokio::spawn(async move {
                    start.wait().await; // every caller is running before any asks for the template
                    on_run_server(&url, Some(&*m)).await
                })
            })
            .collect();
        let mut dbs = Vec::new();
        for c in calls {
            dbs.push(c.await.unwrap());
        }
        assert_eq!(templates(&url).await, vec![template_name(&m)]);
        let mut names = std::collections::HashSet::new();
        for db in &dbs {
            assert_eq!(tables(&db.pool).await, vec!["_sqlx_migrations", "a"]);
            assert!(
                names.insert(db.name.clone()),
                "every caller gets its own database"
            );
        }
    }

    #[tokio::test]
    async fn a_failed_build_publishes_nothing_and_the_next_caller_builds() {
        let (_c, url) = server().await;
        // Fails (division by zero) until a database named reeve_go exists: the same migration set, first failing
        // and then succeeding, so both calls want the same template.
        let (_d, m) = migrator(&[(
            "1_a.sql",
            "CREATE TABLE a (id int); \
             SELECT 1 / (SELECT count(*)::int FROM pg_database WHERE datname = 'reeve_go');",
        )])
        .await;
        let m = std::sync::Arc::new(m);
        let first = {
            let (url, m) = (url.clone(), m.clone());
            tokio::spawn(async move { on_run_server(&url, Some(&*m)).await }).await
        };
        assert!(
            first
                .err()
                .expect("the failing migration must fail the caller")
                .is_panic(),
            "it fails by panicking"
        );
        assert_eq!(
            templates(&url).await,
            vec![format!("{}_building", template_name(&m))],
            "only the scratch database is left, never a template under its final name"
        );
        let mut admin = PgConnection::connect(&url).await.unwrap();
        admin.execute("CREATE DATABASE reeve_go").await.unwrap();
        let db = on_run_server(&url, Some(&m)).await;
        assert_eq!(tables(&db.pool).await, vec!["_sqlx_migrations", "a"]);
        assert_eq!(templates(&url).await, vec![template_name(&m)]);
    }

    #[tokio::test]
    async fn a_killed_builders_leftover_is_dropped_and_rebuilt() {
        let (_c, url) = server().await;
        let (_d, m) = migrator(&[("1_a.sql", "CREATE TABLE a (id int);")]).await;
        // What a builder killed mid-migration leaves: the scratch database, half done.
        let leftover = format!("{}_building", template_name(&m));
        let mut admin = PgConnection::connect(&url).await.unwrap();
        admin
            .execute(format!("CREATE DATABASE {leftover}").as_str())
            .await
            .unwrap();
        let mut half = PgConnection::connect(&url_for(&url, &leftover))
            .await
            .unwrap();
        half.execute("CREATE TABLE junk (id int)").await.unwrap();
        // A session of the dead builder that never closed: the rebuild must not wait for it.
        let db = on_run_server(&url, Some(&m)).await;
        assert_eq!(
            tables(&db.pool).await,
            vec!["_sqlx_migrations", "a"],
            "nothing of the leftover"
        );
        drop(half);
    }

    #[tokio::test]
    async fn clones_and_siblings_are_isolated() {
        let (_c, url) = server().await;
        let (_d, m) = migrator(&[("1_a.sql", "CREATE TABLE a (id int);")]).await;
        let one = on_run_server(&url, Some(&m)).await;
        let two = on_run_server(&url, Some(&m)).await;
        sqlx::query("INSERT INTO a VALUES (1)")
            .execute(&one.pool)
            .await
            .unwrap();
        let in_two: i64 = sqlx::query_scalar("SELECT count(*) FROM a")
            .fetch_one(&two.pool)
            .await
            .unwrap();
        assert_eq!(in_two, 0);
        let sib = one.sibling("other").await;
        assert_eq!(sib, format!("{}_other", one.name));
        let sib_pool = PgPool::connect(&url_for(&url, &sib)).await.unwrap();
        assert!(tables(&sib_pool).await.is_empty(), "a sibling starts empty");
        // The template refuses connections, so no test can write into it.
        let tmpl = PgConnection::connect(&url_for(&url, &template_name(&m))).await;
        assert!(tmpl.is_err(), "the template must not accept connections");
    }

    #[tokio::test]
    async fn a_changed_migration_set_gets_its_own_template() {
        let (_c, url) = server().await;
        let (_d1, a) = migrator(&[("1_a.sql", "CREATE TABLE a (id int);")]).await;
        let (_d2, ab) = migrator(&[
            ("1_a.sql", "CREATE TABLE a (id int);"),
            ("2_b.sql", "CREATE TABLE b (id int);"),
        ])
        .await;
        let one = on_run_server(&url, Some(&a)).await;
        let two = on_run_server(&url, Some(&ab)).await;
        assert_eq!(tables(&one.pool).await, vec!["_sqlx_migrations", "a"]);
        assert_eq!(tables(&two.pool).await, vec!["_sqlx_migrations", "a", "b"]);
        assert_eq!(templates(&url).await.len(), 2);
    }

    #[tokio::test]
    async fn an_unreachable_server_fails_and_never_falls_back() {
        let (_d, m) = migrator(&[("1_a.sql", "CREATE TABLE a (id int);")]).await;
        let m = std::sync::Arc::new(m);
        let r = tokio::spawn(async move {
            on_run_server(
                "postgres://postgres:postgres@127.0.0.1:1/postgres",
                Some(&*m),
            )
            .await
        })
        .await;
        let msg = r
            .err()
            .expect("an unreachable server must fail the caller")
            .into_panic();
        let msg = msg.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(
            msg.contains(URL_VAR),
            "the failure names the variable: {msg}"
        );
        assert!(
            !msg.contains(":postgres@"),
            "the password stays out of the message: {msg}"
        );
    }

    #[tokio::test]
    async fn publishing_ends_a_session_left_on_the_scratch_database() {
        let (_c, url) = server().await;
        let mut admin = PgConnection::connect(&url).await.unwrap();
        admin
            .execute("CREATE DATABASE reeve_tmpl_test_building")
            .await
            .unwrap();
        // A session that never leaves on its own: without ending it, the rename fails with 55006 after its retries.
        let mut lingering = PgConnection::connect(&url_for(&url, "reeve_tmpl_test_building"))
            .await
            .unwrap();
        publish(&mut admin, "reeve_tmpl_test_building", "reeve_tmpl_test").await;
        assert_eq!(templates(&url).await, vec!["reeve_tmpl_test"]);
        assert!(
            lingering.execute("SELECT 1").await.is_err(),
            "the session was ended"
        );
    }

    #[tokio::test]
    async fn a_builder_killed_between_closing_and_publishing_is_rebuilt() {
        let (_c, url) = server().await;
        let (_d, m) = migrator(&[("1_a.sql", "CREATE TABLE a (id int);")]).await;
        // A builder that died after marking its scratch database a template and closing it, before the rename.
        let leftover = format!("{}_building", template_name(&m));
        let mut admin = PgConnection::connect(&url).await.unwrap();
        admin
            .execute(format!("CREATE DATABASE {leftover}").as_str())
            .await
            .unwrap();
        let mut half = PgConnection::connect(&url_for(&url, &leftover))
            .await
            .unwrap();
        half.execute("CREATE TABLE junk (id int)").await.unwrap();
        drop(half);
        admin
            .execute(
                format!("ALTER DATABASE {leftover} WITH IS_TEMPLATE true ALLOW_CONNECTIONS false")
                    .as_str(),
            )
            .await
            .unwrap();
        let db = on_run_server(&url, Some(&m)).await;
        assert_eq!(
            tables(&db.pool).await,
            vec!["_sqlx_migrations", "a"],
            "nothing of the leftover"
        );
        assert_eq!(templates(&url).await, vec![template_name(&m)]);
    }

    #[tokio::test]
    async fn a_container_database_is_migrated_and_configured() {
        let (_d, m) = migrator(&[("1_a.sql", "CREATE TABLE a (id int);")]).await;
        let db = in_container(Some(&m)).await;
        assert_eq!(tables(&db.pool).await, vec!["_sqlx_migrations", "a"]);
        let max: String = sqlx::query_scalar("SHOW pg_stat_statements.max")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(max, "100000", "containers start with SETTINGS");
        let sib = db.sibling("other").await;
        assert_eq!(sib, "postgres_other");
        // PostgreSQL truncates identifiers over 63 bytes silently; a sibling refuses them.
        let long = "x".repeat(60);
        let r = tokio::spawn(async move { db.sibling(&long).await }).await;
        let msg = r.expect_err("a too-long suffix must panic").into_panic();
        let msg = msg.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(msg.contains("63-byte"), "the panic names the limit: {msg}");
    }

    #[tokio::test]
    async fn own_container_hands_the_container_to_the_caller() {
        let (_d, m) = migrator(&[("1_a.sql", "CREATE TABLE a (id int);")]).await;
        let (c, db) = own_container(&m).await;
        assert_eq!(
            db.name, "postgres",
            "the container's own database, as backup_restore's commands expect"
        );
        assert_eq!(tables(&db.pool).await, vec!["_sqlx_migrations", "a"]);
        c.stop().await.unwrap();
        // A fresh connection, not the pool (which would retry for its 30 s acquire timeout).
        assert!(
            PgConnection::connect(&db.url).await.is_err(),
            "stopping it is the caller's"
        );
    }

    #[tokio::test]
    async fn run_server_databases_have_the_run_settings_when_named() {
        // Under scripts/test.sh this checks the run's server; otherwise a container's. Either way the settings hold.
        let db = empty_database().await;
        for s in SETTINGS {
            let (k, v) = s.split_once('=').unwrap();
            let got: String = sqlx::query_scalar(&format!("SHOW {k}"))
                .fetch_one(&db.pool)
                .await
                .unwrap();
            assert_eq!(got, v, "{k}");
        }
    }
}
