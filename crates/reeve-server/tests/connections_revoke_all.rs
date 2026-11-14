//! `reeve-server connections-revoke-all`: the operator command the restore procedure runs before the server
//! starts (kb/ADR-0029 R5, R9). The real binary against the harness's database. Compiled only with
//! `static-oidc` (the connections come from the flow harness; the command itself is in every build).
#![cfg(feature = "static-oidc")]
mod common;
use common::{oauth::*, *};
use serde_json::json;

async fn run(cfg: &std::path::Path) -> std::process::Output {
    tokio::process::Command::new(env!("CARGO_BIN_EXE_reeve-server"))
        .arg("--config")
        .arg(cfg)
        .arg("connections-revoke-all")
        .env_remove("REEVE_CONFIG")
        .env("RUST_LOG", "warn")
        .output()
        .await
        .unwrap()
}

/// What a restore must guarantee: the state a dump brings back (a connection whose rotated refresh token the
/// app no longer holds is the same to this command as any live connection) ends, with its entries. Catches a
/// command that leaves a connection live, one that writes no entry or names a person as the actor, one that
/// rewrites a connection that was revoked before, and a second run that does anything.
#[tokio::test]
async fn it_ends_every_live_connection_with_an_entry_each_and_a_second_run_does_nothing() {
    let f = flow().await;
    let h = &f.h;
    let bobs = connect(&f, &h.victim_token, "read").await;
    let dans = connect(&f, &h.admin_token, "write").await;
    let cut = connect(&f, &h.admin_token, "read").await;
    // One was revoked before (by a reused refresh token) and must keep its reason.
    assert_eq!(token(&f, &refresh_form(&cut.refresh_token)).await.0, 200);
    assert_eq!(token(&f, &refresh_form(&cut.refresh_token)).await.0, 400);
    assert_eq!(
        mcp_status(&f, &bobs.access_token).await,
        200,
        "the control: live before"
    );

    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("reeve.toml");
    std::fs::write(
        &cfg,
        format!("database_url = \"{}\"\nkey_prefix = \"TST\"\nallowed_domains = [\"example.com\"]\nadmins = [\"dan@example.com\"]\n", h.url),
    )
    .unwrap();
    let out = run(&cfg).await;
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "revoked 2 connections"
    );

    for c in [&bobs, &dans] {
        assert_eq!(mcp_status(&f, &c.access_token).await, 401);
        let (status, _, body) = token(&f, &refresh_form(&c.refresh_token)).await;
        assert_eq!(
            (status, &body["error"]),
            (400, &json!("invalid_grant")),
            "{body}"
        );
    }
    let reasons: Vec<(i64, Option<String>)> =
        sqlx::query_as("SELECT id, revoked_reason FROM connections ORDER BY id")
            .fetch_all(&h.state.pool)
            .await
            .unwrap();
    assert_eq!(
        reasons,
        [
            (bobs.connection_id, Some("restored".to_string())),
            (dans.connection_id, Some("restored".to_string())),
            (cut.connection_id, Some("refresh_token_reused".to_string())),
        ]
    );
    let (_, log) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        "/api/v1/activity?action=connection_revoked",
        None,
    )
    .await;
    let mut restored: Vec<(String, String, String, i64)> = log["value"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["payload"]["reason"] == "restored")
        .map(|e| {
            (
                e["actor"].as_str().unwrap().to_string(),
                e["via"].as_str().unwrap().to_string(),
                e["entity_key"].as_str().unwrap().to_string(),
                e["payload"]["connection_id"].as_i64().unwrap(),
            )
        })
        .collect();
    restored.sort_by_key(|e| e.3);
    let entry = |owner: &str, id: i64| {
        (
            "operator".to_string(),
            "system".to_string(),
            owner.to_string(),
            id,
        )
    };
    assert_eq!(
        restored,
        [
            entry("bob@example.com", bobs.connection_id),
            entry("dan@example.com", dans.connection_id)
        ]
    );

    let again = run(&cfg).await;
    assert_eq!(
        String::from_utf8_lossy(&again.stdout).trim(),
        "revoked 0 connections"
    );
}
