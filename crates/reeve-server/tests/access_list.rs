//! `reeve-server access-list`: the merged access lists with their sources, from the same config and database as serve.
mod common;
use common::*;

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

#[tokio::test]
async fn access_list_prints_floor_and_database_entries_with_their_sources() {
    let h = harness().await;
    for q in [
        "INSERT INTO allowed_entries (kind, value, added_by, added_at) VALUES ('email','guest@other.org','dan@example.com','2026-11-09T09:00:00Z')",
        "INSERT INTO admin_grants (email, added_by, added_at) VALUES ('eve@gone.org','dan@example.com','2026-11-09T09:00:00Z')",
    ] {
        sqlx::query(q).execute(&h.state.pool).await.unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_reeve-server"))
        .arg("--config")
        .arg(config_file(dir.path(), &h.url))
        .arg("access-list")
        .env_remove("REEVE_CONFIG")
        .env("RUST_LOG", "warn")
        .output()
        .await
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "domain\texample.com\treeve.toml\n\
         email\tguest@other.org\tadded by dan@example.com 2026-11-09\n\
         admin\tdan@example.com\treeve.toml\n\
         admin\teve@gone.org\tadded by dan@example.com 2026-11-09\tnot allowed\n"
    );
}

#[tokio::test]
async fn token_issue_accepts_a_guest_allowed_only_in_the_database() {
    let h = harness().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config_file(dir.path(), &h.url);
    let issue = || {
        tokio::process::Command::new(env!("CARGO_BIN_EXE_reeve-server"))
            .arg("--config")
            .arg(&cfg)
            .args([
                "token-issue",
                "--email",
                "guest@other.org",
                "--kind",
                "interactive",
                "--label",
                "x",
            ])
            .env_remove("REEVE_CONFIG")
            .env("RUST_LOG", "warn")
            .output()
    };
    assert_eq!(
        issue().await.unwrap().status.code(),
        Some(1),
        "not allowed yet"
    );
    sqlx::query("INSERT INTO allowed_entries (kind, value, added_by, added_at) VALUES ('email','guest@other.org','dan@example.com',now())")
        .execute(&h.state.pool)
        .await
        .unwrap();
    let out = issue().await.unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .starts_with("reeve_")
    );
}
