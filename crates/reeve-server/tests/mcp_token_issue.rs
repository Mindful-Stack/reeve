//! `reeve-server mcp-token-issue`: the developer command that mints an MCP token, in builds with the `static-oidc`
//! feature only. Its absence from a production build is tested in `config_no_static.rs`.
#![cfg(feature = "static-oidc")]
mod common;
use common::*;

async fn run(cfg: &std::path::Path, email: &str) -> std::process::Output {
    tokio::process::Command::new(env!("CARGO_BIN_EXE_reeve-server"))
        .arg("--config")
        .arg(cfg)
        .args(["mcp-token-issue", "--email", email])
        .env_remove("REEVE_CONFIG")
        .env("RUST_LOG", "warn")
        .output()
        .await
        .unwrap()
}

#[tokio::test]
async fn mcp_token_issue_prints_a_one_hour_mcp_token_for_an_allowed_user() {
    let h = harness().await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("reeve.toml");
    std::fs::write(
        &cfg,
        format!(
            "database_url = \"{}\"\nkey_prefix = \"TST\"\nallowed_domains = [\"example.com\"]\nadmins = [\"dan@example.com\"]\n",
            h.url
        ),
    )
    .unwrap();
    let before = chrono::Utc::now();
    let out = run(&cfg, "bob@example.com").await;
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let secret = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(secret.starts_with("reeve_"), "{secret:?}");
    let (kind, audience, expires): (String, String, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as("SELECT kind, audience, expires_at FROM tokens WHERE hash = $1")
            .bind(reeve_identity::hash_secret(&secret))
            .fetch_one(&h.state.pool)
            .await
            .unwrap();
    assert_eq!((kind.as_str(), audience.as_str()), ("interactive", "mcp"));
    let expires = expires.expect("an MCP token always expires");
    assert!(
        expires > before + chrono::Duration::minutes(59),
        "{expires}"
    );
    assert!(
        expires <= chrono::Utc::now() + chrono::Duration::hours(1),
        "{expires}"
    );
    // It is an MCP token to the server: refused on the API, accepted by the MCP function.
    assert_eq!(
        call(&h.app, Some(&secret), "GET", "/api/v1/me", None)
            .await
            .0,
        401
    );
    let policy = h.state.access.load(&h.state.identity).await.unwrap();
    let who =
        reeve_identity::authenticate_mcp(&*h.state.tokens, &policy, &secret, chrono::Utc::now())
            .await
            .unwrap();
    assert_eq!(who.email, "bob@example.com");

    let refused = run(&cfg, "mallory@evil.example").await;
    assert_eq!(refused.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("email not allowed"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
}
