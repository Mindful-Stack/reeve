mod common;
use common::*;
use serde_json::json;

#[tokio::test]
async fn tokens_are_owner_scoped_and_automation_cannot_issue() {
    let h = harness().await;
    let (st, b) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/tokens",
        Some(json!({"kind":"automation","label":"lorekeeper","expires_in_days":90})),
    )
    .await;
    assert_eq!(st, 200);
    let secret = b["value"]["secret"].as_str().unwrap().to_string();
    let id = b["value"]["token"]["id"].as_i64().unwrap();
    assert!(secret.starts_with("reeve_"));
    // the new automation token works, cannot issue, and its writes are logged as the owner via automation
    let (st, _) = call(&h.app, Some(&secret), "GET", "/api/v1/me", None).await;
    assert_eq!(st, 200);
    let (st, ini) = call(
        &h.app,
        Some(&secret),
        "POST",
        "/api/v1/initiatives",
        Some(json!({"idempotency_key":"bot-i1","title":"Bot","horizon":"Now"})),
    )
    .await;
    assert_eq!(st, 200);
    let (_, log) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        &format!(
            "/api/v1/activity?entity_key={}",
            ini["value"]["key"].as_str().unwrap()
        ),
        None,
    )
    .await;
    assert_eq!(log["value"][0]["actor"], "dan@example.com");
    assert_eq!(log["value"][0]["via"], "automation");
    let (st, b) = call(
        &h.app,
        Some(&secret),
        "POST",
        "/api/v1/tokens",
        Some(json!({"kind":"interactive"})),
    )
    .await;
    assert_eq!(st, 403);
    assert_eq!(b["outcome"], "error");
    // list shows it without the secret; the victim's list does not
    let (_, mine) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/tokens", None).await;
    assert!(
        mine["value"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == id && t.get("secret").is_none())
    );
    let (_, theirs) = call(&h.app, Some(&h.victim_token), "GET", "/api/v1/tokens", None).await;
    assert!(
        !theirs["value"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == id)
    );
    // the victim cannot revoke it (404), the owner can (200), then it is dead
    assert_eq!(
        call(
            &h.app,
            Some(&h.victim_token),
            "DELETE",
            &format!("/api/v1/tokens/{id}"),
            None
        )
        .await
        .0,
        404
    );
    assert_eq!(
        call(
            &h.app,
            Some(&h.admin_token),
            "DELETE",
            &format!("/api/v1/tokens/{id}"),
            None
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(&h.app, Some(&secret), "GET", "/api/v1/me", None)
            .await
            .0,
        401
    );
    // expires_in_days 0 or > 365 is a validation error
    assert_eq!(
        call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            "/api/v1/tokens",
            Some(json!({"kind":"interactive","expires_in_days":0}))
        )
        .await
        .0,
        400
    );
    assert_eq!(
        call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            "/api/v1/tokens",
            Some(json!({"kind":"interactive","expires_in_days":366}))
        )
        .await
        .0,
        400
    );
}

#[tokio::test]
async fn users_are_admin_only_and_disable_revokes_sessions() {
    let h = harness().await;
    assert_eq!(
        call(&h.app, Some(&h.victim_token), "GET", "/api/v1/users", None)
            .await
            .0,
        403
    );
    let (st, b) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/users", None).await;
    assert_eq!(st, 200);
    let victim = b["value"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["email"] == "bob@example.com")
        .unwrap();
    assert_eq!(victim["role"], "member");
    assert_eq!(victim["disabled"], false);
    assert_eq!(
        call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            "/api/v1/users/bob@example.com/disable",
            None
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(&h.app, Some(&h.victim_token), "GET", "/api/v1/me", None)
            .await
            .0,
        401
    );
    assert_eq!(
        call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            "/api/v1/users/bob@example.com/enable",
            None
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(&h.app, Some(&h.victim_token), "GET", "/api/v1/me", None)
            .await
            .0,
        401,
        "enable must not resurrect tokens"
    );
    // an admin cannot disable themselves (the last admin would lock the instance), whatever the casing
    assert_eq!(
        call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            "/api/v1/users/Dan@Example.com/disable",
            None
        )
        .await
        .0,
        400
    );
    // enable of an address outside the allowlist is refused and creates no user row
    assert_eq!(
        call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            "/api/v1/users/x@other.org/enable",
            None
        )
        .await
        .0,
        400
    );
    let (_, b) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/users", None).await;
    assert!(
        !b["value"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["email"] == "x@other.org")
    );
}

/// The identity activity entries for `email` with `action`, newest first.
async fn entries(h: &Harness, email: &str, action: &str) -> Vec<serde_json::Value> {
    let (st, b) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        &format!("/api/v1/activity?entity_key={email}&action={action}"),
        None,
    )
    .await;
    assert_eq!(st, 200);
    b["value"].as_array().unwrap().clone()
}

#[tokio::test]
async fn members_cannot_disable_or_enable() {
    let h = harness().await;
    for path in [
        "/api/v1/users/dan@example.com/disable",
        "/api/v1/users/bob@example.com/enable",
        "/api/v1/users/bob@example.com/disable",
    ] {
        assert_eq!(
            call(&h.app, Some(&h.victim_token), "POST", path, None)
                .await
                .0,
            403,
            "{path}"
        );
    }
    // the target is unchanged and nothing was logged
    assert_eq!(
        call(&h.app, Some(&h.admin_token), "GET", "/api/v1/me", None)
            .await
            .0,
        200
    );
    assert_eq!(
        call(&h.app, Some(&h.victim_token), "GET", "/api/v1/me", None)
            .await
            .0,
        200
    );
    assert!(
        entries(&h, "dan@example.com", "user_disabled")
            .await
            .is_empty()
    );
    assert!(
        entries(&h, "bob@example.com", "user_disabled")
            .await
            .is_empty()
    );
    assert!(
        entries(&h, "bob@example.com", "user_enabled")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn identity_writes_are_logged_with_actor_subject_and_channel() {
    let h = harness().await;
    // issue: actor and subject are the caller, via interactive, payload names the token
    let (_, b) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/tokens",
        Some(json!({"kind":"automation","label":"bot"})),
    )
    .await;
    let id = b["value"]["token"]["id"].as_i64().unwrap();
    let secret = b["value"]["secret"].as_str().unwrap().to_string();
    // the harness issued its own tokens (actor "test"); ours is the one whose payload names this id
    let issued: Vec<_> = entries(&h, "dan@example.com", "token_issued")
        .await
        .into_iter()
        .filter(|e| e["payload"]["token_id"] == id)
        .collect();
    assert_eq!(issued.len(), 1);
    assert_eq!(issued[0]["actor"], "dan@example.com");
    assert_eq!(issued[0]["via"], "interactive");
    assert_eq!(issued[0]["entity_key"], "dan@example.com");
    assert_eq!(issued[0]["payload"]["token_id"], id);
    // revoke interactively, then once more with an automation token on its owner's own token
    assert_eq!(
        call(
            &h.app,
            Some(&h.admin_token),
            "DELETE",
            &format!("/api/v1/tokens/{id}"),
            None
        )
        .await
        .0,
        200
    );
    let revoked = entries(&h, "dan@example.com", "token_revoked").await;
    assert_eq!(revoked.len(), 1);
    assert_eq!(revoked[0]["actor"], "dan@example.com");
    assert_eq!(revoked[0]["via"], "interactive");
    assert_eq!(revoked[0]["payload"]["token_id"], id);
    let (_, b2) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/tokens",
        Some(json!({"kind":"automation","label":"bot2"})),
    )
    .await;
    let id2 = b2["value"]["token"]["id"].as_i64().unwrap();
    let secret2 = b2["value"]["secret"].as_str().unwrap().to_string();
    assert_ne!(secret, secret2);
    assert_eq!(
        call(
            &h.app,
            Some(&secret2),
            "DELETE",
            &format!("/api/v1/tokens/{id2}"),
            None
        )
        .await
        .0,
        200
    );
    let revoked = entries(&h, "dan@example.com", "token_revoked").await;
    assert_eq!(revoked.len(), 2);
    assert_eq!(revoked[0]["payload"]["token_id"], id2);
    assert_eq!(revoked[0]["actor"], "dan@example.com");
    assert_eq!(revoked[0]["via"], "automation");
    // disable and enable: the admin is the actor, the target is the subject
    for (verb, action) in [("disable", "user_disabled"), ("enable", "user_enabled")] {
        assert_eq!(
            call(
                &h.app,
                Some(&h.admin_token),
                "POST",
                &format!("/api/v1/users/bob@example.com/{verb}"),
                None
            )
            .await
            .0,
            200
        );
        let e = entries(&h, "bob@example.com", action).await;
        assert_eq!(e.len(), 1, "{action}");
        assert_eq!(e[0]["actor"], "dan@example.com");
        assert_eq!(e[0]["entity_key"], "bob@example.com");
        assert_eq!(e[0]["via"], "interactive");
    }
    // the same two through an automation token log via automation
    let (_, b3) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/tokens",
        Some(json!({"kind":"automation","label":"bot3"})),
    )
    .await;
    let secret3 = b3["value"]["secret"].as_str().unwrap().to_string();
    for (verb, action) in [("disable", "user_disabled"), ("enable", "user_enabled")] {
        assert_eq!(
            call(
                &h.app,
                Some(&secret3),
                "POST",
                &format!("/api/v1/users/bob@example.com/{verb}"),
                None
            )
            .await
            .0,
            200
        );
        let e = entries(&h, "bob@example.com", action).await;
        assert_eq!(e.len(), 2, "{action}");
        assert_eq!(e[0]["actor"], "dan@example.com");
        assert_eq!(e[0]["via"], "automation");
    }
}

#[tokio::test]
async fn expiry_label_and_revoke_edges() {
    let h = harness().await;
    let issue = |body: serde_json::Value| {
        let h = &h;
        async move {
            call(
                &h.app,
                Some(&h.admin_token),
                "POST",
                "/api/v1/tokens",
                Some(body),
            )
            .await
        }
    };
    let when =
        |v: &serde_json::Value| chrono::DateTime::parse_from_rfc3339(v.as_str().unwrap()).unwrap();
    // 90 days is exactly 90 days after creation; none means never
    let (st, b) = issue(json!({"kind":"interactive","expires_in_days":90})).await;
    assert_eq!(st, 200);
    let t = &b["value"]["token"];
    assert_eq!(
        when(&t["expires_at"]) - when(&t["created_at"]),
        chrono::Duration::days(90)
    );
    assert_eq!(t["revoked_at"], serde_json::Value::Null);
    let (st, b) = issue(json!({"kind":"interactive"})).await;
    assert_eq!(st, 200);
    assert_eq!(b["value"]["token"]["expires_at"], serde_json::Value::Null);
    // the boundaries are accepted
    for d in [1, 365] {
        let (st, b) = issue(json!({"kind":"interactive","expires_in_days":d})).await;
        assert_eq!(st, 200, "{d}");
        let t = &b["value"]["token"];
        assert_eq!(
            when(&t["expires_at"]) - when(&t["created_at"]),
            chrono::Duration::days(d)
        );
    }
    // the response describes the stored token
    let (st, b) = issue(json!({"kind":"automation","label":"described"})).await;
    assert_eq!(st, 200);
    let id = b["value"]["token"]["id"].as_i64().unwrap();
    let (_, mine) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/tokens", None).await;
    let listed = mine["value"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap();
    // the database keeps microseconds, the response keeps the clock's nanoseconds: compare the rest exactly
    let mut mine = listed.clone();
    let mut given = b["value"]["token"].clone();
    let (a, c) = (when(&mine["created_at"]), when(&given["created_at"]));
    assert!((a - c).abs() < chrono::Duration::milliseconds(1));
    mine["created_at"] = json!(null);
    given["created_at"] = json!(null);
    assert_eq!(mine, given);
    // a label of 100 characters is accepted, 101 is a validation error
    assert_eq!(
        issue(json!({"kind":"interactive","label":"é".repeat(100)}))
            .await
            .0,
        200
    );
    assert_eq!(
        issue(json!({"kind":"interactive","label":"é".repeat(101)}))
            .await
            .0,
        400
    );
    // revoking an unknown id, and the same id twice, are both 404; the first revoke shows in the list
    let path = format!("/api/v1/tokens/{id}");
    assert_eq!(
        call(
            &h.app,
            Some(&h.admin_token),
            "DELETE",
            "/api/v1/tokens/999999",
            None
        )
        .await
        .0,
        404
    );
    assert_eq!(
        call(&h.app, Some(&h.admin_token), "DELETE", &path, None)
            .await
            .0,
        200
    );
    assert_eq!(
        call(&h.app, Some(&h.admin_token), "DELETE", &path, None)
            .await
            .0,
        404
    );
    let (_, mine) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/tokens", None).await;
    let listed = mine["value"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap();
    assert!(listed["revoked_at"].is_string());
    // disable of an address with no user row is refused before any write
    assert_eq!(
        call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            "/api/v1/users/x@other.org/disable",
            None
        )
        .await
        .0,
        404
    );
    assert_eq!(
        call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            "/api/v1/users/garbage/disable",
            None
        )
        .await
        .0,
        404
    );
    let (_, u) = call(&h.app, Some(&h.admin_token), "GET", "/api/v1/users", None).await;
    assert!(
        !u["value"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["email"] == "x@other.org" || u["email"] == "garbage")
    );
}
