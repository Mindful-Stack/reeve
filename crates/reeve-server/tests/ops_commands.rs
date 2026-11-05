//! The operator subcommands that need no database: `healthcheck`, `migrations`, `check-config`.
use std::process::Output;
use std::time::{Duration, Instant};

/// The built binary with a configuration path that does not exist: a subcommand that loaded the configuration would
/// fail with "No such file or directory".
fn reeve_server(args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_reeve-server"))
        .arg("--config")
        .arg("/nonexistent/reeve.toml")
        .args(args)
        .env_remove("REEVE_CONFIG")
        .env("RUST_LOG", "warn")
        .output()
        .unwrap()
}

/// A local HTTP server answering `/health` with `status` (after `delay`), on a free port; returns its base URL.
async fn serve_health(status: u16, delay: Duration) -> String {
    use axum::{Router, http::StatusCode, routing::get};
    let app = Router::new().route(
        "/health",
        get(move || async move {
            tokio::time::sleep(delay).await;
            StatusCode::from_u16(status).unwrap()
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test(flavor = "multi_thread")]
async fn healthcheck_exits_0_only_on_200() {
    let ok = serve_health(200, Duration::ZERO).await;
    let down = serve_health(503, Duration::ZERO).await;
    let url = ok.clone();
    let out = tokio::task::spawn_blocking(move || reeve_server(&["healthcheck", "--url", &url]))
        .await
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = tokio::task::spawn_blocking(move || reeve_server(&["healthcheck", "--url", &down]))
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("503"),
        "names the status"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn healthcheck_fails_on_a_closed_port_and_on_a_hang() {
    // A port that was free a moment ago: nothing listens there.
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", l.local_addr().unwrap())
    };
    let out = tokio::task::spawn_blocking(move || reeve_server(&["healthcheck", "--url", &closed]))
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let hang = serve_health(200, Duration::from_secs(60)).await;
    let started = Instant::now();
    let out = tokio::task::spawn_blocking(move || reeve_server(&["healthcheck", "--url", &hang]))
        .await
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "the client gives up after its own timeout, took {:?}",
        started.elapsed()
    );
}

/// `migrations` lists the composed migrator, and its text is what the migrated database's ledger reads as.
#[tokio::test(flavor = "multi_thread")]
async fn migrations_lists_what_serve_applies_in_the_ledgers_format() {
    let out = tokio::task::spawn_blocking(|| reeve_server(&["migrations"]))
        .await
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let listed = String::from_utf8(out.stdout).unwrap();
    let expected: String = reeve_server::composed_migrator()
        .iter()
        .map(|m| format!("{} {}\n", m.version, hex::encode(&*m.checksum)))
        .collect();
    assert_eq!(listed, expected);
    assert!(listed.lines().all(|l| {
        let (v, sum) = l.split_once(' ').unwrap();
        v.parse::<i64>().is_ok()
            && sum.len() == 96
            && sum
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }));
    // The oracle: a clone of the template the server's own migrator (composed_migrator) built, so its ledger is the one
    // a fresh migrate writes, reads back as the same lines.
    let db = reeve_testdb::database(&reeve_server::composed_migrator()).await;
    let pool = db.pool.clone();
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT version, encode(checksum, 'hex') FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let ledger: String = rows.iter().map(|(v, h)| format!("{v} {h}\n")).collect();
    assert_eq!(listed, ledger);
}

/// A configuration file holding `text`, and the binary's `check-config` on it.
fn check_config(text: &str) -> Output {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reeve.toml");
    std::fs::write(&path, text).unwrap();
    std::process::Command::new(env!("CARGO_BIN_EXE_reeve-server"))
        .arg("check-config")
        .env("REEVE_CONFIG", &path)
        .env("RUST_LOG", "warn")
        .output()
        .unwrap()
}

/// Shaped like a production configuration, Google sign-in included; nothing listens on port 9, so a
/// subcommand that connected to the database would fail, and Google's discovery would need the network.
const PRODUCTION_SHAPED: &str = r#"public_url = "https://reeve.example.com"
bind = "0.0.0.0:7474"
database_url = "postgres://reeve:0000000000000000000000000000000000000000000000000000000000000000@127.0.0.1:9/reeve"
key_prefix = "ABC"
horizon_buckets = ["Now", "Next", "Later"]
allowed_domains = ["example.com"]
allowed_emails = ["reeve-check@example.com"]
admins = ["admin@example.com", "reeve-check@example.com"]
[oidc]
provider = "google"
client_id = "1234-abc.apps.googleusercontent.com"
client_secret = "GOCSPX-test"
[[repos]]
name = "knowledge-base"
url = "git@github-knowledge-base:Acme/knowledge-base.git"
adr_dir = "knowledge/adrs"
[github]
webhook_secret = "0123456789abcdef0123" # test value; gitleaks:allow
repos = ["Acme/portal"]
legacy_prefixes = ["SCRUM"]
[board]
done_window_days = 14
"#;

#[test]
fn check_config_accepts_a_production_configuration_without_touching_anything() {
    let out = check_config(PRODUCTION_SHAPED);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "configuration ok\n");
}

#[test]
fn check_config_refuses_what_serve_would_refuse() {
    let out =
        check_config(&PRODUCTION_SHAPED.replace("key_prefix = \"ABC\"", "key_prefix = \"abc\""));
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("key_prefix must be uppercase letters"));
    // Appended after `[board]`, whose table refuses unknown keys (`deny_unknown_fields`), as serve would.
    let out = check_config(&format!("{PRODUCTION_SHAPED}surprise = 1\n"));
    assert_eq!(
        out.status.code(),
        Some(1),
        "an unknown key is refused as serve refuses it"
    );
    let out = check_config(&PRODUCTION_SHAPED.replace("0123456789abcdef0123", "short"));
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("webhook_secret must be at least 16 characters")
    );
}
