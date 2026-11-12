mod common;
use common::*;
use serde_json::json;

#[tokio::test]
async fn auth_envelope_gate_and_automation_rule() {
    let h = harness().await;
    assert_eq!(
        call(&h.app, None, "GET", "/api/v1/stories", None).await.0,
        401
    );
    assert_eq!(
        call(&h.app, Some("reeve_bogus"), "GET", "/api/v1/stories", None)
            .await
            .0,
        401
    );

    let (st, ini) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/initiatives",
        Some(json!({"idempotency_key":"i1","title":"Orion","horizon":"Now"})),
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(ini["outcome"], "ok");
    let ini_key = ini["value"]["key"].as_str().unwrap().to_string();
    let (_, epic) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/epics",
        Some(json!({"idempotency_key":"e1","title":"v2","initiative":ini_key})),
    )
    .await;
    let epic_key = epic["value"]["key"].as_str().unwrap().to_string();
    let (_, story) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/stories",
        Some(json!({"idempotency_key":"s1","title":"Booking","kind":"story","epic":epic_key,"criteria":["works"],"labels":[]})),
    )
    .await;
    let key = story["value"]["key"].as_str().unwrap().to_string();

    // Refused with reasons, as 422 and the refused envelope.
    let (st, body) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        &format!("/api/v1/stories/{key}/move"),
        Some(json!({"version":1,"to":"in_progress"})),
    )
    .await;
    assert_eq!(st, 422);
    assert_eq!(body["outcome"], "refused");
    assert!(
        body["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["reason"] == "ancestor_discovery_incomplete")
    );
    assert_eq!(
        body["overridable"], true,
        "readiness facts alone: the server says an override may get past them"
    );

    // Automation token with an override is still refused; interactive override passes.
    let (st, body_bot) = call(
        &h.app,
        Some(&h.bot_token),
        "POST",
        &format!("/api/v1/stories/{key}/move"),
        Some(json!({"version":1,"to":"in_progress","override_reason":"urgent"})),
    )
    .await;
    assert_eq!(st, 422);
    assert!(
        body_reasons(&body_bot)
            .iter()
            .any(|r| r == "automation_cannot_override"),
        "{body_bot}"
    );
    assert_eq!(
        body_bot["overridable"], false,
        "a delivery rule: no override gets past it"
    );
    let (st, moved) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        &format!("/api/v1/stories/{key}/move"),
        Some(json!({"version":1,"to":"in_progress","override_reason":"urgent"})),
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(moved["value"]["delivery"], "in_progress");
    assert_eq!(moved["value"]["owner"], "dan@example.com");

    // Stale version -> 409 conflict envelope.
    let (st, body) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        &format!("/api/v1/stories/{key}/move"),
        Some(json!({"version":1,"to":"todo"})),
    )
    .await;
    assert_eq!(st, 409);
    assert_eq!(body["code"], "conflict");
    assert_eq!(body["current"], moved["value"]["version"], "{body}");

    // Override is visible in the activity feed.
    let (_, feed) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        "/api/v1/activity?action=override",
        None,
    )
    .await;
    assert_eq!(feed["value"].as_array().unwrap().len(), 1);
    assert_eq!(feed["value"][0]["via"], "interactive");

    // Health needs no token.
    assert_eq!(call(&h.app, None, "GET", "/health", None).await.0, 200);
}

/// An item of any kind without an epic is untriaged (proposed kb/ADR-0018). The override rule: automation never gets
/// past a refusal (kb/ADR-0015 R7, kb/ADR-0018 R2) and a refusal writes nothing; only an interactive caller's
/// override moves it, logged once with its reason (kb/ADR-0018 R2).
#[tokio::test]
async fn an_untriaged_bug_moves_only_by_a_logged_interactive_override() {
    let h = harness().await;
    let admin = Some(h.admin_token.as_str());
    let (st, s) = call(
        &h.app,
        admin,
        "POST",
        "/api/v1/stories",
        Some(json!({"idempotency_key":"u","title":"Crash on save","kind":"bug","criteria":["it saves"],"labels":[]})),
    )
    .await;
    assert_eq!(st, 200, "{s}");
    let key = s["value"]["key"].as_str().unwrap().to_string();
    let move_path = format!("/api/v1/stories/{key}/move");
    let activity_len = || async {
        let (st, feed) = call(
            &h.app,
            admin,
            "GET",
            &format!("/api/v1/activity?entity_key={key}"),
            None,
        )
        .await;
        assert_eq!(st, 200, "{feed}");
        feed["value"].as_array().unwrap().len()
    };
    let before = activity_len().await;

    let (st, body) = call(
        &h.app,
        Some(&h.bot_token),
        "POST",
        &move_path,
        Some(json!({"version":1,"to":"in_progress"})),
    )
    .await;
    assert_eq!(st, 422, "{body}");
    assert_eq!(body_reasons(&body), vec!["untriaged"], "{body}");
    let (st, body) = call(
        &h.app,
        Some(&h.bot_token),
        "POST",
        &move_path,
        Some(json!({"version":1,"to":"in_progress","override_reason":"urgent"})),
    )
    .await;
    assert_eq!(st, 422, "{body}");
    assert!(
        body_reasons(&body)
            .iter()
            .any(|r| r == "automation_cannot_override"),
        "{body}"
    );
    assert_eq!(activity_len().await, before, "a refusal writes nothing");

    let (st, moved) = call(
        &h.app,
        admin,
        "POST",
        &move_path,
        Some(json!({"version":1,"to":"in_progress","override_reason":"urgent"})),
    )
    .await;
    assert_eq!(st, 200, "{moved}");
    assert_eq!(moved["value"]["delivery"], "in_progress");
    let (_, feed) = call(
        &h.app,
        admin,
        "GET",
        "/api/v1/activity?action=override",
        None,
    )
    .await;
    let entries = feed["value"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{feed}");
    assert_eq!(entries[0]["via"], "interactive");
    assert_eq!(entries[0]["entity_key"], key.as_str(), "{feed}");
    assert_eq!(entries[0]["payload"]["reason"], "urgent", "{feed}");
    assert!(
        entries[0]["payload"]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["reason"] == "untriaged"),
        "{feed}"
    );
}

fn body_reasons(b: &serde_json::Value) -> Vec<String> {
    b["reasons"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|r| r["reason"].as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn every_documented_operation_is_gated() {
    let h = harness().await;
    let doc = reeve_server::openapi_json();
    let mut checked = 0;
    for (path, item) in doc["paths"].as_object().unwrap() {
        let concrete = path
            .replace("{key}", "ABC-1")
            .replace("{blocker}", "ABC-2")
            .replace("{id}", "1")
            .replace("{email}", "someone@example.com");
        for (method, _) in item.as_object().unwrap() {
            if !["get", "post", "put", "patch", "delete"].contains(&method.as_str()) {
                continue;
            }
            let (st, body) = call(&h.app, None, &method.to_uppercase(), &concrete, None).await;
            assert_eq!(st, 401, "{method} {path}");
            assert_eq!(body["code"], "unauthenticated", "{method} {path}");
            checked += 1;
        }
    }
    assert!(checked >= 25, "only {checked} operations walked");
}

#[tokio::test]
async fn bad_credentials_and_disabled_users_are_401() {
    let h = harness().await;
    let get = |auth: String| {
        let app = h.app.clone();
        async move {
            use axum::body::Body;
            use tower::ServiceExt;
            let req = axum::http::Request::builder()
                .uri("/api/v1/me")
                .header("authorization", auth)
                .body(Body::empty())
                .unwrap();
            let res = app.oneshot(req).await.unwrap();
            (
                res.status().as_u16(),
                res.headers().get("www-authenticate").cloned(),
            )
        }
    };
    let (st, www) = get(format!("Basic {}", h.admin_token)).await;
    assert_eq!(st, 401);
    assert_eq!(www.unwrap(), "Bearer");
    assert_eq!(get("Bearer ".into()).await.0, 401);
    assert_eq!(get("Bearer".into()).await.0, 401);
    assert_eq!(get(format!("bearer {}", h.admin_token)).await.0, 200);

    assert_eq!(
        call(&h.app, Some(&h.victim_token), "GET", "/api/v1/me", None)
            .await
            .0,
        200
    );
    let pool = sqlx::PgPool::connect(&h.url).await.unwrap();
    let tokens = reeve_identity::make_token_store(pool.clone());
    let log: std::sync::Arc<dyn reeve_identity::IdentityActivity> =
        reeve_activity::make_activity_store(pool.clone());
    reeve_identity::disable_user(
        &pool,
        &*tokens,
        &*log,
        "test",
        reeve_protocol::Via::System,
        "bob@example.com",
        chrono::Utc::now(),
    )
    .await
    .unwrap();
    assert_eq!(
        call(&h.app, Some(&h.victim_token), "GET", "/api/v1/me", None)
            .await
            .0,
        401
    );
}

#[tokio::test]
async fn malformed_input_and_unknown_routes_are_envelopes() {
    let h = harness().await;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/api/v1/stories")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", h.admin_token))
        .body(Body::from("{not json"))
        .unwrap();
    let res = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status().as_u16(), 400);
    let v: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(v["outcome"], "error");
    assert_eq!(v["code"], "validation");

    let (st, v) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        "/api/v1/stories/not-a-key",
        None,
    )
    .await;
    // The segment is resolved: a string that is neither a key nor an external reference names nothing.
    assert_eq!((st, v["code"].as_str()), (404, Some("not_found")));
    let (st, v) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        "/api/v1/activity?limit=0",
        None,
    )
    .await;
    assert_eq!((st, v["code"].as_str()), (400, Some("validation")));
    let (st, v) = call(
        &h.app,
        Some(&h.admin_token),
        "DELETE",
        "/api/v1/stories",
        None,
    )
    .await;
    assert_eq!((st, v["outcome"].as_str()), (405, Some("error")));
    let (st, v) = call(&h.app, None, "GET", "/api/v1/nope", None).await;
    assert_eq!(
        (st, v["outcome"].as_str(), v["code"].as_str()),
        (404, Some("error"), Some("not_found"))
    );
}

#[tokio::test]
async fn full_journey_over_the_remaining_routes() {
    let h = harness().await;
    let t = Some(h.admin_token.as_str());
    let (_, me) = call(&h.app, t, "GET", "/api/v1/me", None).await;
    assert_eq!(me["value"]["email"], "dan@example.com");
    assert_eq!(me["value"]["role"], "admin");

    let (_, ini) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/initiatives",
        Some(json!({"idempotency_key":"i","title":"I","horizon":"Now","owner":"dan@example.com"})),
    )
    .await;
    let ik = ini["value"]["key"].as_str().unwrap().to_string();
    let (_, epic) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/epics",
        Some(json!({"idempotency_key":"e","title":"E","initiative":ik})),
    )
    .await;
    let ek = epic["value"]["key"].as_str().unwrap().to_string();

    // Discovery on both ancestors.
    let (_, cur) = call(&h.app, t, "GET", &format!("/api/v1/initiatives/{ik}"), None).await;
    let (st, d) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/initiatives/{ik}/discovery"),
        Some(json!({"version":cur["value"]["version"],"complete":true})),
    )
    .await;
    assert_eq!(st, 200, "{d}");
    assert!(!d["value"]["discovery_complete"].is_null());
    let (_, cur) = call(&h.app, t, "GET", &format!("/api/v1/epics/{ek}"), None).await;
    let (st, d) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/epics/{ek}/discovery"),
        Some(json!({"version":cur["value"]["version"],"complete":true})),
    )
    .await;
    assert_eq!(st, 200, "{d}");
    assert!(!d["value"]["discovery_complete"].is_null());

    // A ready story under the epic.
    let (_, s2) = call(&h.app, t, "POST", "/api/v1/stories", Some(json!({"idempotency_key":"s2","title":"Ready","kind":"story","epic":ek,"criteria":["ok"],"labels":[]}))).await;
    let k2 = s2["value"]["key"].as_str().unwrap().to_string();
    let (st, ready) = call(&h.app, t, "GET", "/api/v1/stories?ready=true", None).await;
    assert_eq!(st, 200);
    assert!(
        ready["value"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["story"]["key"] == k2.as_str()),
        "{ready}"
    );
    let (_, r) = call(
        &h.app,
        t,
        "GET",
        &format!("/api/v1/stories/{k2}/ready"),
        None,
    )
    .await;
    assert_eq!(r["value"]["ready"], true);

    // A second story, edited to clear its epic and owner, becomes untriaged.
    let (_, s1) = call(&h.app, t, "POST", "/api/v1/stories", Some(json!({"idempotency_key":"s1","title":"Loose","kind":"story","epic":ek,"criteria":["ok"],"labels":[]}))).await;
    let k1 = s1["value"]["key"].as_str().unwrap().to_string();
    let (st, e) = call(
        &h.app,
        t,
        "PATCH",
        &format!("/api/v1/stories/{k1}"),
        Some(json!({"version":1,"epic":null,"owner":null})),
    )
    .await;
    assert_eq!(st, 200, "{e}");
    assert!(e["value"]["epic"].is_null());
    let (_, un) = call(&h.app, t, "GET", "/api/v1/stories?untriaged=true", None).await;
    assert!(
        un["value"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["story"]["key"] == k1.as_str())
    );

    // Comment on the story.
    let (st, c) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/stories/{k1}/comments"),
        Some(json!({"body":"hello"})),
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(c["value"][0]["body"], "hello");

    // Decision: require, link, check, close.
    let (st, d) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/decisions",
        Some(json!({"idempotency_key":"d1","on":k1,"title":"Pick","decider":"dan@example.com"})),
    )
    .await;
    assert_eq!(st, 200, "{d}");
    let id = d["value"]["id"].as_i64().unwrap();
    // The repository must be configured and the path must stay inside it.
    for body in [
        json!({"repo":"../elsewhere","path":"docs/adr/0001.md","record_id":"ADR-0001"}),
        json!({"repo":"orion","path":"../../etc/passwd","record_id":"ADR-0001"}),
    ] {
        let (st, bad) = call(
            &h.app,
            t,
            "POST",
            &format!("/api/v1/decisions/{id}/link"),
            Some(body),
        )
        .await;
        assert_eq!(
            (st, bad["code"].as_str()),
            (400, Some("validation")),
            "{bad}"
        );
    }
    let (st, d) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/decisions/{id}/link"),
        Some(json!({"repo":"orion","path":"docs/adr/0001.md","record_id":"ADR-0001"})),
    )
    .await;
    assert_eq!(st, 200, "{d}");
    let ver = d["value"]["version"].as_i64().unwrap();
    let (st, stale) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/decisions/{id}/check"),
        Some(json!({"record_id":"ADR-0001","version":ver + 5,"check":{"kind":"proposed_pr_open"}})),
    )
    .await;
    assert_eq!((st, stale["code"].as_str()), (409, Some("conflict")));
    let (st, d) = call(&h.app, t, "POST", &format!("/api/v1/decisions/{id}/check"), Some(json!({"record_id":"ADR-0001","version":ver,"check":{"kind":"accepted","commit":"abc"}}))).await;
    assert_eq!(st, 200, "{d}");
    assert_eq!(d["value"]["check"]["kind"], "accepted");
    let (st, d) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/decisions/{id}/close"),
        Some(json!({"kind":"decided","reason":"done"})),
    )
    .await;
    assert_eq!(st, 200, "{d}");
    assert!(!d["value"]["closed"].is_null());
    // The unscoped list leaves the closed decision out unless asked for it; an item-scoped list always has it.
    for (path, expected) in [
        ("/api/v1/decisions", 0),
        ("/api/v1/decisions?include_closed=true", 1),
        (&*format!("/api/v1/decisions?on={k1}"), 1),
    ] {
        let (st, l) = call(&h.app, t, "GET", path, None).await;
        assert_eq!(st, 200, "{l}");
        assert_eq!(
            l["value"].as_array().unwrap().len(),
            expected,
            "{path}: {l}"
        );
    }
    let (st, c) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/decisions/{id}/comments"),
        Some(json!({"body":"noted"})),
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(c["value"].as_array().unwrap().len(), 1);
    // kb/ADR-0013 R3: decided on a one-way decision is a validation error (400); withdrawn closes.
    let (st, d1) = call(&h.app, t, "POST", "/api/v1/decisions",
        Some(json!({"idempotency_key":"ow","on":k1,"title":"Pricing","decider":"dan@example.com","reversibility":"one_way"}))).await;
    assert_eq!(
        (st, d1["value"]["reversibility"].as_str()),
        (200, Some("one_way"))
    );
    let id1 = d1["value"]["id"].as_i64().unwrap();
    let (st, r) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/decisions/{id1}/close"),
        Some(json!({"kind":"decided","reason":"flat fee"})),
    )
    .await;
    assert_eq!((st, r["code"].as_str()), (400, Some("validation")), "{r}");
    let (st, w) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/decisions/{id1}/close"),
        Some(json!({"kind":"withdrawn","reason":"dropped"})),
    )
    .await;
    assert_eq!(
        (st, w["value"]["closed"]["kind"].as_str()),
        (200, Some("withdrawn"))
    );
    // A create without the field is unset; an edit that omits it leaves it; an explicit null clears it.
    let (st, d2) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/decisions",
        Some(json!({"idempotency_key":"ow2","on":k1,"title":"Copy","decider":"dan@example.com"})),
    )
    .await;
    assert_eq!(st, 200, "{d2}");
    assert!(d2["value"]["reversibility"].is_null());
    let id2 = d2["value"]["id"].as_i64().unwrap();
    let (st, e) = call(
        &h.app,
        t,
        "PATCH",
        &format!("/api/v1/decisions/{id2}"),
        Some(json!({"version":d2["value"]["version"],"reversibility":"two_way"})),
    )
    .await;
    assert_eq!(st, 200, "{e}");
    let (st, e) = call(
        &h.app,
        t,
        "PATCH",
        &format!("/api/v1/decisions/{id2}"),
        Some(json!({"version":e["value"]["version"],"title":"Copy v2"})),
    )
    .await;
    assert_eq!(st, 200, "{e}");
    assert_eq!(e["value"]["reversibility"], "two_way", "omitted leaves it");
    let before = e["value"]["version"].as_i64().unwrap();
    let (st, e) = call(
        &h.app,
        t,
        "PATCH",
        &format!("/api/v1/decisions/{id2}"),
        Some(json!({"version":before,"reversibility":null})),
    )
    .await;
    assert_eq!(st, 200, "{e}");
    assert_eq!(e["value"]["version"], before + 1, "{e}");
    assert!(e["value"]["reversibility"].is_null(), "null clears it");
}

#[tokio::test]
async fn epic_edit_reparents_and_clears_its_initiative() {
    let h = harness().await;
    let t = Some(h.admin_token.as_str());
    let mut inis = vec![];
    for k in ["a", "b"] {
        let (_, i) = call(
            &h.app,
            t,
            "POST",
            "/api/v1/initiatives",
            Some(json!({"idempotency_key":k,"title":k,"horizon":"Now"})),
        )
        .await;
        inis.push(i["value"]["key"].as_str().unwrap().to_string());
    }
    let (_, e) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/epics",
        Some(json!({"idempotency_key":"e","title":"E","initiative":inis[0]})),
    )
    .await;
    let ek = e["value"]["key"].as_str().unwrap().to_string();
    let (st, moved) = call(
        &h.app,
        t,
        "PATCH",
        &format!("/api/v1/epics/{ek}"),
        Some(json!({"version":1,"initiative":inis[1]})),
    )
    .await;
    assert_eq!(st, 200, "{moved}");
    assert_eq!(moved["value"]["initiative"], inis[1].as_str());
    let (st, cleared) = call(
        &h.app,
        t,
        "PATCH",
        &format!("/api/v1/epics/{ek}"),
        Some(json!({"version":moved["value"]["version"],"initiative":null})),
    )
    .await;
    assert_eq!(st, 200, "{cleared}");
    assert!(cleared["value"]["initiative"].is_null(), "{cleared}");
    // An edit that names no initiative leaves the parent alone.
    let (_, back) = call(
        &h.app,
        t,
        "PATCH",
        &format!("/api/v1/epics/{ek}"),
        Some(json!({"version":cleared["value"]["version"],"initiative":inis[0]})),
    )
    .await;
    let (_, titled) = call(
        &h.app,
        t,
        "PATCH",
        &format!("/api/v1/epics/{ek}"),
        Some(json!({"version":back["value"]["version"],"title":"E2"})),
    )
    .await;
    assert_eq!(titled["value"]["initiative"], inis[0].as_str(), "{titled}");
}

/// The per-request disabled check on its own: the user is disabled directly in the table while the token stays live
/// (not revoked, not expired), so only `owner_disabled` can turn the request away.
#[tokio::test]
async fn a_disabled_user_with_a_live_token_is_401() {
    let h = harness().await;
    assert_eq!(
        call(&h.app, Some(&h.victim_token), "GET", "/api/v1/me", None)
            .await
            .0,
        200
    );
    let pool = sqlx::PgPool::connect(&h.url).await.unwrap();
    sqlx::query("UPDATE users SET disabled = true WHERE email = 'bob@example.com'")
        .execute(&pool)
        .await
        .unwrap();
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM tokens WHERE owner = 'bob@example.com' AND revoked_at IS NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(live, 1, "the token itself is still live");
    assert_eq!(
        call(&h.app, Some(&h.victim_token), "GET", "/api/v1/me", None)
            .await
            .0,
        401
    );
}

#[tokio::test]
async fn config_returns_the_instance_settings_and_is_gated() {
    let h = harness().await;
    assert_eq!(
        call(&h.app, None, "GET", "/api/v1/config", None).await.0,
        401
    );
    let (st, b) = call(&h.app, Some(&h.victim_token), "GET", "/api/v1/config", None).await;
    assert_eq!(st, 200);
    assert_eq!(b["value"]["key_prefix"], "TST");
    assert_eq!(
        b["value"]["horizon_buckets"],
        json!(["Now", "Next", "Upcoming", "Later", "Won't do"])
    );
    assert_eq!(b["value"]["repos"], json!(["orion"]));
    assert_eq!(b["value"]["sign_in_configured"], true);
    assert_eq!(b["value"]["import_enabled"], false);
    assert_eq!(b["value"]["done_window_days"], 14);
}

#[tokio::test]
async fn config_reports_the_done_window() {
    let h = harness_with(|cfg| format!("{cfg}\n[board]\ndone_window_days = 7\n")).await;
    let (st, b) = call(&h.app, Some(&h.victim_token), "GET", "/api/v1/config", None).await;
    assert_eq!(st, 200);
    assert_eq!(b["value"]["done_window_days"], 7);
}

#[tokio::test]
async fn done_recent_lists_only_recent_done_stories() {
    use chrono::{Duration, Utc};
    let h = harness_with(|cfg| format!("{cfg}\n[board]\ndone_window_days = 7\n")).await;
    let t = Some(h.admin_token.as_str());
    let mut keys = Vec::new();
    for i in 0..4 {
        let (st, s) = call(&h.app, t, "POST", "/api/v1/stories", Some(json!({"idempotency_key":format!("d{i}"),"title":format!("S{i}"),"kind":"story","criteria":["ok"],"labels":[]}))).await;
        assert_eq!(st, 200, "{s}");
        keys.push(s["value"]["key"].as_str().unwrap().to_string());
    }
    for k in &keys[1..] {
        let (st, b) = call(
            &h.app,
            t,
            "POST",
            &format!("/api/v1/stories/{k}/move"),
            Some(json!({"version":1,"to":"done","resolution":"dropped"})),
        )
        .await;
        assert_eq!(st, 200, "{b}");
    }
    for (k, age) in [
        (&keys[2], Duration::days(6) + Duration::hours(23)),
        (&keys[3], Duration::days(7) + Duration::hours(1)),
    ] {
        sqlx::query("UPDATE stories SET done_at = $1 WHERE key = $2")
            .bind(Utc::now() - age)
            .bind(k)
            .execute(&h.state.pool)
            .await
            .unwrap();
    }
    let listed = |path: &'static str| {
        let h = &h;
        async move {
            let (st, b) = call(&h.app, Some(&h.admin_token), "GET", path, None).await;
            assert_eq!(st, 200, "{b}");
            let mut ks: Vec<String> = b["value"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["story"]["key"].as_str().unwrap().to_string())
                .collect();
            ks.sort();
            ks
        }
    };
    let mut want_recent = vec![keys[0].clone(), keys[1].clone(), keys[2].clone()];
    want_recent.sort();
    let mut want_all = keys.clone();
    want_all.sort();
    assert_eq!(listed("/api/v1/stories?done=recent").await, want_recent);
    assert_eq!(listed("/api/v1/stories?done=all").await, want_all);
    assert_eq!(listed("/api/v1/stories").await, want_all);
    let (st, v) = call(&h.app, t, "GET", "/api/v1/stories?done=soon", None).await;
    assert_eq!((st, v["code"].as_str()), (400, Some("validation")));
}

#[tokio::test]
async fn rank_places_a_todo_story_and_returns_the_story_envelope() {
    let h = harness().await;
    let mut keys = Vec::new();
    for i in 0..3 {
        let (_, s) = call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            "/api/v1/stories",
            Some(json!({"idempotency_key":format!("rk{i}"),"title":format!("S{i}"),"kind":"chore","criteria":["ok"],"labels":[]})),
        )
        .await;
        keys.push(s["value"]["key"].as_str().unwrap().to_string());
    }
    let (st, b) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        &format!("/api/v1/stories/{}/rank", keys[2]),
        Some(json!({"version":1,"before":keys[0]})),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["outcome"], "ok");
    assert_eq!(b["value"]["key"], keys[2].as_str());
    assert_eq!(b["value"]["rank"], 0);
    assert_eq!(b["value"]["version"], 2);
    let (_, list) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        "/api/v1/stories?delivery=todo",
        None,
    )
    .await;
    let order: Vec<&str> = list["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["story"]["key"].as_str().unwrap())
        .collect();
    assert_eq!(
        order,
        vec![keys[2].as_str(), keys[0].as_str(), keys[1].as_str()]
    );
    // A stale version is the conflict envelope; no `before` means the end of todo.
    let (st, b) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        &format!("/api/v1/stories/{}/rank", keys[2]),
        Some(json!({"version":1})),
    )
    .await;
    assert_eq!(st, 409);
    assert_eq!(b["current"], 2);
    let (_, feed) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        "/api/v1/activity?action=reranked",
        None,
    )
    .await;
    assert_eq!(feed["value"].as_array().unwrap().len(), 1);
}

/// Every key input resolves an external reference; each assertion distinguishes a route still typed `Key` for that input
/// (it would look for a literal `SCRUM-…` key and answer 404). Responses always carry the Reeve key (kb/ADR-0002 R5).
#[tokio::test]
async fn external_references_resolve_for_every_key_input() {
    let h = harness().await;
    let t = Some(h.admin_token.as_str());
    let (_, epic) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/epics",
        Some(json!({"idempotency_key":"xe","title":"Imported epic"})),
    )
    .await;
    let epic_key = epic["value"]["key"].as_str().unwrap().to_string();
    let mut stories = Vec::new();
    for i in 0..2 {
        let (_, s) = call(
            &h.app,
            t,
            "POST",
            "/api/v1/stories",
            Some(json!({"idempotency_key":format!("xs{i}"),"title":format!("S{i}"),"kind":"story","criteria":[],"labels":[]})),
        )
        .await;
        stories.push(s["value"]["key"].as_str().unwrap().to_string());
    }
    let (first, second) = (&stories[0], &stories[1]);
    sqlx::query("UPDATE epics SET external_ref = 'SCRUM-10' WHERE key = $1")
        .bind(&epic_key)
        .execute(&h.state.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE stories SET external_ref = 'SCRUM-11' WHERE key = $1")
        .bind(first)
        .execute(&h.state.pool)
        .await
        .unwrap();

    let (st, v) = call(&h.app, t, "GET", "/api/v1/stories/SCRUM-11", None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["value"]["key"], first.as_str());
    assert!(first.starts_with("TST-"), "{first}");
    assert_eq!(v["value"]["external_ref"], "SCRUM-11");

    let (st, v) = call(&h.app, t, "GET", "/api/v1/epics/SCRUM-11", None).await;
    assert_eq!((st, v["code"].as_str()), (404, Some("not_found")));

    let (st, v) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/stories/SCRUM-11/comments",
        Some(json!({"body":"via the jira key"})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["value"][0]["on_story"], first.as_str());

    let (st, v) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/stories",
        Some(json!({"idempotency_key":"xs-epic","title":"Under the epic","kind":"story","epic":"SCRUM-10","criteria":[],"labels":[]})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["value"]["epic"], epic_key.as_str());
    let child = v["value"]["key"].as_str().unwrap().to_string();

    let (st, v) = call(&h.app, t, "GET", "/api/v1/stories?epic=SCRUM-10", None).await;
    assert_eq!(st, 200, "{v}");
    let listed: Vec<&str> = v["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["story"]["key"].as_str().unwrap())
        .collect();
    assert_eq!(listed, vec![child.as_str()]);

    let (st, v) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/stories/{second}/blocks"),
        Some(json!({"blocker":"SCRUM-11"})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    let (_, v) = call(&h.app, t, "GET", &format!("/api/v1/stories/{second}"), None).await;
    assert_eq!(v["value"]["blocked_by"], json!([first]));
    let version = v["value"]["version"].as_i64().unwrap();

    let (st, v) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/stories/{second}/rank"),
        Some(json!({"version":version,"before":"SCRUM-11"})),
    )
    .await;
    assert_eq!(st, 200, "{v}");

    let (st, v) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/decisions",
        Some(json!({"idempotency_key":"xd","on":"SCRUM-10","title":"Pick","decider":"dan@example.com"})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["value"]["on"], epic_key.as_str());
}

/// List filters are queries: an external reference that matches nothing yields an empty 200 (as an unknown instance key
/// does), while a path parameter or body field naming it stays 404. Also covers the initiative and `on` inputs with
/// external references, path resolution on epics and initiatives, and `kind` over the wire.
#[tokio::test]
async fn list_filters_tolerate_unknown_references_and_remaining_inputs_resolve() {
    let h = harness().await;
    let t = Some(h.admin_token.as_str());
    let (_, ini) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/initiatives",
        Some(json!({"idempotency_key":"fi","title":"I","horizon":"Now"})),
    )
    .await;
    let ini_key = ini["value"]["key"].as_str().unwrap().to_string();
    sqlx::query("UPDATE initiatives SET external_ref = 'SCRUM-1' WHERE key = $1")
        .bind(&ini_key)
        .execute(&h.state.pool)
        .await
        .unwrap();

    // Epic created under the initiative by its external reference; the epic gets its own reference.
    let (st, e) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/epics",
        Some(json!({"idempotency_key":"fe","title":"E","initiative":"SCRUM-1"})),
    )
    .await;
    assert_eq!(st, 200, "{e}");
    assert_eq!(e["value"]["initiative"], ini_key.as_str());
    let epic_key = e["value"]["key"].as_str().unwrap().to_string();
    sqlx::query("UPDATE epics SET external_ref = 'SCRUM-2' WHERE key = $1")
        .bind(&epic_key)
        .execute(&h.state.pool)
        .await
        .unwrap();

    // Path params resolve external references for epics and initiatives.
    let (st, v) = call(&h.app, t, "GET", "/api/v1/epics/SCRUM-2", None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["value"]["key"], epic_key.as_str());
    let (st, v) = call(&h.app, t, "GET", "/api/v1/initiatives/SCRUM-1", None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["value"]["key"], ini_key.as_str());

    // Edit: a story's epic and an epic's initiative by external reference.
    let (_, s) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/stories",
        Some(json!({"idempotency_key":"fs","title":"S","kind":"story","criteria":[],"labels":[]})),
    )
    .await;
    let story_key = s["value"]["key"].as_str().unwrap().to_string();
    let (st, v) = call(
        &h.app,
        t,
        "PATCH",
        &format!("/api/v1/stories/{story_key}"),
        Some(json!({"version":1,"epic":"SCRUM-2"})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["value"]["epic"], epic_key.as_str());
    let (st, v) = call(
        &h.app,
        t,
        "PATCH",
        &format!("/api/v1/epics/{epic_key}"),
        Some(json!({"version":1,"initiative":"SCRUM-1"})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["value"]["initiative"], ini_key.as_str());

    // `kind` is plumbed through the PATCH route and the version bumps.
    let (_, cur) = call(
        &h.app,
        t,
        "GET",
        &format!("/api/v1/stories/{story_key}"),
        None,
    )
    .await;
    let version = cur["value"]["version"].as_i64().unwrap();
    let (st, v) = call(
        &h.app,
        t,
        "PATCH",
        &format!("/api/v1/stories/{story_key}"),
        Some(json!({"version":version,"kind":"bug"})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["value"]["kind"], "bug");
    assert_eq!(v["value"]["version"].as_i64().unwrap(), version + 1);
    let (_, v) = call(
        &h.app,
        t,
        "GET",
        &format!("/api/v1/stories/{story_key}"),
        None,
    )
    .await;
    assert_eq!(v["value"]["kind"], "bug");

    // `?initiative=` and `?on=` with external references.
    let (st, v) = call(&h.app, t, "GET", "/api/v1/epics?initiative=SCRUM-1", None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["value"].as_array().unwrap().len(), 1);
    assert_eq!(v["value"][0]["key"], epic_key.as_str());
    let (st, d) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/decisions",
        Some(json!({"idempotency_key":"fd","on":"SCRUM-1","title":"Pick","decider":"dan@example.com"})),
    )
    .await;
    assert_eq!(st, 200, "{d}");
    assert_eq!(d["value"]["on"], ini_key.as_str());
    let (st, v) = call(&h.app, t, "GET", "/api/v1/decisions?on=SCRUM-1", None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["value"].as_array().unwrap().len(), 1);

    // One filter of each kind: an unmatched reference is an empty list.
    for uri in [
        "/api/v1/stories?epic=SCRUM-99",
        "/api/v1/epics?initiative=SCRUM-99",
        "/api/v1/decisions?on=SCRUM-99",
    ] {
        let (st, v) = call(&h.app, t, "GET", uri, None).await;
        assert_eq!(st, 200, "{uri}: {v}");
        assert_eq!(v["value"], json!([]), "{uri}");
    }
    // An unknown instance key behaves the same way.
    let (st, v) = call(&h.app, t, "GET", "/api/v1/stories?epic=TST-999", None).await;
    assert_eq!((st, &v["value"]), (200, &json!([])));

    // Path params and body fields keep 404.
    let (st, v) = call(&h.app, t, "GET", "/api/v1/epics/SCRUM-99", None).await;
    assert_eq!((st, v["code"].as_str()), (404, Some("not_found")));
    let (st, v) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/stories",
        Some(json!({"idempotency_key":"fs2","title":"S2","kind":"story","epic":"SCRUM-99","criteria":[],"labels":[]})),
    )
    .await;
    assert_eq!((st, v["code"].as_str()), (404, Some("not_found")));
}

/// Only an admin's check stores `accepted`, the one result that satisfies a decision (kb/ADR-0018 R3), whatever the token
/// kind; every other result is stored from any caller; readiness follows the stored results only.
#[tokio::test]
async fn only_an_admin_stores_an_accepted_check() {
    let h = harness().await;
    let admin = Some(h.admin_token.as_str());
    let member = Some(h.victim_token.as_str());
    let ident_log = reeve_activity::make_activity_store(h.state.pool.clone());
    let member_bot = reeve_identity::issue_token(
        &h.state.pool,
        &*h.state.tokens,
        &*ident_log,
        &reeve_identity::AccessPolicy::floor_only(&h.state.identity),
        "test",
        reeve_protocol::Via::Interactive,
        &h.victim_email,
        reeve_identity::TokenKind::Automation,
        "member-bot",
        None,
        chrono::Utc::now(),
    )
    .await
    .unwrap()
    .secret;
    let epic = ready_epic(&h, "ready-epic").await;
    let (st, s) = call(
        &h.app,
        admin,
        "POST",
        "/api/v1/stories",
        Some(json!({"idempotency_key":"s","title":"Wire the queue","kind":"chore","epic":epic,"criteria":["messages arrive"],"labels":[]})),
    )
    .await;
    assert_eq!(st, 200, "{s}");
    let key = s["value"]["key"].as_str().unwrap().to_string();
    let (st, d) = call(
        &h.app,
        admin,
        "POST",
        "/api/v1/decisions",
        Some(json!({"idempotency_key":"d","on":key,"title":"Pick","decider":"dan@example.com"})),
    )
    .await;
    assert_eq!(st, 200, "{d}");
    let id = d["value"]["id"].as_i64().unwrap();
    let (st, d) = call(
        &h.app,
        member,
        "POST",
        &format!("/api/v1/decisions/{id}/link"),
        Some(json!({"repo":"orion","path":"docs/adr/0001.md","record_id":"ADR-0001"})),
    )
    .await;
    assert_eq!(st, 200, "{d}");
    let ready_path = format!("/api/v1/stories/{key}/ready");
    let check_path = format!("/api/v1/decisions/{id}/check");
    let first_reason = |b: &serde_json::Value| b["value"]["reasons"][0]["reason"].clone();
    let accepted = json!({"kind":"accepted","commit":"abc"});

    // A member's not_accepted is stored: it can only make readiness stricter.
    let ver = d["value"]["version"].as_i64().unwrap();
    let (st, d) = call(
        &h.app,
        member,
        "POST",
        &check_path,
        Some(json!({"record_id":"ADR-0001","version":ver,"check":{"kind":"not_accepted","status":"proposed","superseded_by":null,"commit":"abc"}})),
    )
    .await;
    assert_eq!(st, 200, "{d}");
    let ver = d["value"]["version"].as_i64().unwrap();
    let (_, r) = call(&h.app, admin, "GET", &ready_path, None).await;
    assert_eq!(first_reason(&r), "decision_not_accepted", "{r}");

    // A member's accepted, from an interactive or an automation token: 403 with the forbidden envelope, nothing stored.
    for t in [h.victim_token.as_str(), member_bot.as_str()] {
        let (st, b) = call(
            &h.app,
            Some(t),
            "POST",
            &check_path,
            Some(json!({"record_id":"ADR-0001","version":ver,"check":accepted})),
        )
        .await;
        assert_eq!(
            (st, b["outcome"].as_str(), b["code"].as_str()),
            (403, Some("error"), Some("forbidden")),
            "{b}"
        );
        let (_, r) = call(&h.app, admin, "GET", &ready_path, None).await;
        assert_eq!(r["value"]["ready"], false, "{r}");
        assert_eq!(first_reason(&r), "decision_not_accepted", "{r}");
    }

    // The admin's automation token (what the scheduled run uses): stored, and the story is ready.
    let (st, d) = call(
        &h.app,
        Some(&h.bot_token),
        "POST",
        &check_path,
        Some(json!({"record_id":"ADR-0001","version":ver,"check":accepted})),
    )
    .await;
    assert_eq!(st, 200, "{d}");
    let (_, r) = call(&h.app, admin, "GET", &ready_path, None).await;
    assert_eq!(r["value"]["ready"], true, "{r}");
}

/// The blocker segment resolves like every key input (here by its external reference), the entry names the native key,
/// a repeat writes nothing, a missing story is 404, and a cookie caller needs the client header.
#[tokio::test]
async fn unblock_removes_one_edge_over_the_wire() {
    use axum::{body::Body, http::Request};
    let h = harness().await;
    let t = Some(h.admin_token.as_str());
    let mut keys = Vec::new();
    for i in 0..2 {
        let (_, s) = call(&h.app, t, "POST", "/api/v1/stories",
            Some(json!({"idempotency_key":format!("ub{i}"),"title":format!("U{i}"),"kind":"chore","criteria":[],"labels":[]}))).await;
        keys.push(s["value"]["key"].as_str().unwrap().to_string());
    }
    let (a, b) = (&keys[0], &keys[1]);
    // Closures return async blocks so the formatted path lives as long as the request that borrows it.
    let block = || async {
        call(
            &h.app,
            t,
            "POST",
            &format!("/api/v1/stories/{a}/blocks"),
            Some(json!({"blocker": b})),
        )
        .await
    };
    assert_eq!(block().await.0, 200);
    sqlx::query("UPDATE stories SET external_ref = 'SCRUM-21' WHERE key = $1")
        .bind(b)
        .execute(&h.state.pool)
        .await
        .unwrap();
    let blocked_by = || async {
        call(&h.app, t, "GET", &format!("/api/v1/stories/{a}"), None)
            .await
            .1["value"]["blocked_by"]
            .clone()
    };
    let unblocked = || async {
        let (_, log) = call(
            &h.app,
            t,
            "GET",
            &format!("/api/v1/activity?entity_type=story&entity_key={a}"),
            None,
        )
        .await;
        log["value"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["action"] == "unblocked")
            .map(|e| e["payload"]["blocker"].clone())
            .collect::<Vec<_>>()
    };

    let (st, v) = call(
        &h.app,
        t,
        "DELETE",
        &format!("/api/v1/stories/{a}/blocks/SCRUM-21"),
        None,
    )
    .await;
    assert_eq!((st, &v), (200, &json!({"outcome":"ok","value":{}})));
    assert_eq!(blocked_by().await, json!([]));
    assert_eq!(
        unblocked().await,
        vec![json!(b)],
        "the entry names the native key"
    );
    let (st, _) = call(
        &h.app,
        t,
        "DELETE",
        &format!("/api/v1/stories/{a}/blocks/{b}"),
        None,
    )
    .await;
    assert_eq!(st, 200, "a missing edge is a no-op");
    assert_eq!(unblocked().await.len(), 1);
    let (_, ini) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/initiatives",
        Some(
            json!({"idempotency_key":"ub-i","title":"I","horizon":"Now","owner":"dan@example.com"}),
        ),
    )
    .await;
    let ik = ini["value"]["key"].as_str().unwrap().to_string();
    let (_, epic) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/epics",
        Some(json!({"idempotency_key":"ub-e","title":"E","initiative":ik})),
    )
    .await;
    let ek = epic["value"]["key"].as_str().unwrap().to_string();
    for path in [
        format!("/api/v1/stories/{a}/blocks/TST-999"),
        format!("/api/v1/stories/TST-999/blocks/{b}"),
        format!("/api/v1/stories/{a}/blocks/{ek}"),
        format!("/api/v1/stories/{ek}/blocks/{b}"),
    ] {
        let (st, v) = call(&h.app, t, "DELETE", &path, None).await;
        assert_eq!((st, v["code"].as_str()), (404, Some("not_found")), "{path}");
    }

    // An automation token may unblock; the entry is logged `via: automation` (kb/ADR-0015 R7).
    assert_eq!(block().await.0, 200);
    let (st, v) = call(
        &h.app,
        Some(h.bot_token.as_str()),
        "DELETE",
        &format!("/api/v1/stories/{a}/blocks/{b}"),
        None,
    )
    .await;
    assert_eq!((st, &v), (200, &json!({"outcome":"ok","value":{}})));
    let (_, log) = call(
        &h.app,
        t,
        "GET",
        &format!("/api/v1/activity?entity_type=story&entity_key={a}&action=unblocked"),
        None,
    )
    .await;
    let vias: Vec<_> = log["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["via"].clone())
        .collect();
    assert!(vias.contains(&json!("automation")), "{log}");

    // A cookie caller: refused without the client header (and nothing changes), served with it.
    assert_eq!(block().await.0, 200);
    let cookie = format!("reeve_session={}", h.admin_token);
    let del = |client_header: bool| {
        let mut r =
            Request::delete(format!("/api/v1/stories/{a}/blocks/{b}")).header("cookie", &cookie);
        if client_header {
            r = r.header("x-reeve-client", "web");
        }
        r.body(Body::empty()).unwrap()
    };
    assert_eq!(call_raw(&h.app, del(false)).await.status(), 403);
    assert_eq!(blocked_by().await, json!([b]));
    assert_eq!(call_raw(&h.app, del(true)).await.status(), 200);
    assert_eq!(blocked_by().await, json!([]));
}

/// PATCH /decisions/{id}: planner fields change, the record and check survive, stale is 409, blank and unknown fields are
/// 400, closed is 400, unknown id is 404, and a cookie caller needs the client header.
#[tokio::test]
async fn decision_edit_over_the_wire() {
    use axum::{body::Body, http::Request};
    let h = harness().await;
    let t = Some(h.admin_token.as_str());
    let (_, s) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/stories",
        Some(
            json!({"idempotency_key":"de-s","title":"S","kind":"chore","criteria":[],"labels":[]}),
        ),
    )
    .await;
    let k = s["value"]["key"].as_str().unwrap().to_string();
    let require = |idem: &'static str| {
        call(
            &h.app,
            t,
            "POST",
            "/api/v1/decisions",
            Some(
                json!({"idempotency_key":idem,"on":k,"title":"Pick","decider":"dan@example.com","due":"2026-11-30"}),
            ),
        )
    };
    let (_, d) = require("de-d").await;
    let id = d["value"]["id"].as_i64().unwrap();
    let v1 = d["value"]["version"].as_i64().unwrap();
    let path = format!("/api/v1/decisions/{id}");

    let (st, v) = call(
        &h.app,
        t,
        "PATCH",
        &path,
        Some(json!({"version":v1,"decider":"bob@example.com"})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["value"]["decider"], "bob@example.com");
    assert_eq!(
        v["value"]["due"], "2026-11-30",
        "an absent due is left alone"
    );
    assert_eq!(v["value"]["version"], v1 + 1);
    let (st, v) = call(
        &h.app,
        t,
        "PATCH",
        &path,
        Some(json!({"version":v1 + 1,"due":null})),
    )
    .await;
    assert_eq!(
        (st, &v["value"]["due"]),
        (200, &serde_json::Value::Null),
        "null clears the due date"
    );
    let (st, v) = call(
        &h.app,
        t,
        "PATCH",
        &path,
        Some(json!({"version":v1 + 2,"due":"2026-12-15"})),
    )
    .await;
    assert_eq!((st, &v["value"]["due"]), (200, &json!("2026-12-15")), "{v}");
    // Refusals, with the precedence blank → version → closed pinned by combined cases. None moves the version or logs.
    let edited = || async {
        let q = format!("/api/v1/activity?entity_type=decision&entity_key={id}&action=edited");
        call(&h.app, t, "GET", &q, None).await.1["value"]
            .as_array()
            .unwrap()
            .len()
    };
    let version_now = || async {
        call(&h.app, t, "GET", &path, None).await.1["value"]["version"]
            .as_i64()
            .unwrap()
    };
    let conflict = |current: i64| json!({"outcome":"error","code":"conflict","message":format!("version conflict: current is {current}"),"current":current});
    let blank = json!({"outcome":"error","code":"validation","message":"title and decider must not be blank"});
    let cur = v1 + 3;
    let logged = edited().await;
    for (body, want_st, want) in [
        (json!({"version":v1,"title":"x"}), 409, conflict(cur)), // stale
        (
            json!({"version":v1,"decider":"bob@example.com"}),
            409,
            conflict(cur),
        ), // stale no-op: values match, still 409
        (json!({"version":cur,"title":"  "}), 400, blank.clone()), // blank
        (json!({"version":v1,"title":"  "}), 400, blank.clone()), // stale + blank: blank answers first
        (json!({"version":v1,"decider":""}), 400, blank.clone()),
    ] {
        let (st, v) = call(&h.app, t, "PATCH", &path, Some(body.clone())).await;
        assert_eq!((st, &v), (want_st, &want), "{body}");
    }
    // An unknown field is serde's refusal: the code is pinned, not serde's message text.
    let (st, v) = call(
        &h.app,
        t,
        "PATCH",
        &path,
        Some(json!({"version":cur,"record_id":"ADR-0001"})),
    )
    .await;
    assert_eq!((st, v["code"].as_str()), (400, Some("validation")), "{v}");
    // A malformed date is serde's refusal too.
    let (st, v) = call(
        &h.app,
        t,
        "PATCH",
        &path,
        Some(json!({"version":cur,"due":"2026-13-01"})),
    )
    .await;
    assert_eq!((st, v["code"].as_str()), (400, Some("validation")), "{v}");
    assert_eq!(version_now().await, cur, "no refusal moved the version");
    assert_eq!(edited().await, logged, "no refusal wrote an entry");

    // The record and its check survive a title edit.
    let (_, l) = call(
        &h.app,
        t,
        "POST",
        &format!("{path}/link"),
        Some(json!({"repo":"orion","path":"docs/adr/0001.md","record_id":"ADR-0001"})),
    )
    .await;
    let ver = l["value"]["version"].as_i64().unwrap();
    let (st, c) = call(&h.app, t, "POST", &format!("{path}/check"),
        Some(json!({"record_id":"ADR-0001","version":ver,"check":{"kind":"accepted","commit":"abc"}}))).await;
    assert_eq!(st, 200, "{c}");
    let ver = c["value"]["version"].as_i64().unwrap();
    // A member may edit planner fields of a satisfied decision; the admin-stored check survives (m2d: only storing
    // `accepted` is admin-only).
    let checked_at = c["value"]["checked_at"].clone();
    let (st, v) = call(
        &h.app,
        Some(h.victim_token.as_str()),
        "PATCH",
        &path,
        Some(json!({"version":ver,"title":"Pick the queue"})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["value"]["record"]["record_id"], "ADR-0001");
    assert_eq!(v["value"]["check"]["kind"], "accepted");
    assert_eq!(v["value"]["checked_at"], checked_at);
    // The retitle stays visible and attributable: the member is the actor, and the old title is in the entry.
    let (_, log) = call(
        &h.app,
        t,
        "GET",
        &format!("/api/v1/activity?entity_type=decision&entity_key={id}&action=edited"),
        None,
    )
    .await;
    let retitle = log["value"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["payload"]["to"]["title"] == "Pick the queue")
        .expect("the retitle is logged");
    assert_eq!(retitle["actor"], "bob@example.com");
    assert_eq!(
        retitle["payload"],
        json!({"fields":["title"],"from":{"title":"Pick"},"to":{"title":"Pick the queue"}})
    );

    let ver = v["value"]["version"].as_i64().unwrap();
    let (st, _) = call(
        &h.app,
        t,
        "POST",
        &format!("{path}/close"),
        Some(json!({"kind":"decided","reason":"done"})),
    )
    .await;
    assert_eq!(st, 200);
    let closed = ver + 1;
    let logged = edited().await;
    for (body, want_st, want) in [
        (
            json!({"version":closed,"title":"y"}),
            400,
            json!({"outcome":"error","code":"validation","message":"decision is closed"}),
        ),
        // stale + closed: the version answers first, so the caller reloads and then sees it closed
        (json!({"version":ver,"title":"y"}), 409, conflict(closed)),
        // blank on a closed decision, stale or not: blank answers first
        (json!({"version":ver,"title":" "}), 400, blank.clone()),
    ] {
        let (st, v) = call(&h.app, t, "PATCH", &path, Some(body.clone())).await;
        assert_eq!((st, &v), (want_st, &want), "{body}");
    }
    assert_eq!(version_now().await, closed);
    assert_eq!(edited().await, logged);
    let (st, v) = call(
        &h.app,
        t,
        "PATCH",
        "/api/v1/decisions/999999",
        Some(json!({"version":1,"title":"y"})),
    )
    .await;
    assert_eq!((st, v["code"].as_str()), (404, Some("not_found")), "{v}");

    // The automation token may edit; the entry says it came via automation.
    let (_, d3) = require("de-d3").await;
    let id3 = d3["value"]["id"].as_i64().unwrap();
    let (st, v) = call(
        &h.app,
        Some(h.bot_token.as_str()),
        "PATCH",
        &format!("/api/v1/decisions/{id3}"),
        Some(json!({"version":d3["value"]["version"],"title":"By bot"})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    let (_, log3) = call(
        &h.app,
        t,
        "GET",
        &format!("/api/v1/activity?entity_type=decision&entity_key={id3}&action=edited"),
        None,
    )
    .await;
    assert_eq!(log3["value"][0]["via"], "automation", "{log3}");

    // A cookie caller on an open decision.
    let (_, d2) = require("de-d2").await;
    let path2 = format!("/api/v1/decisions/{}", d2["value"]["id"]);
    let v2 = d2["value"]["version"].as_i64().unwrap();
    let cookie = format!("reeve_session={}", h.admin_token);
    let patch = |client_header: bool| {
        let mut r = Request::patch(&path2)
            .header("cookie", &cookie)
            .header("content-type", "application/json");
        if client_header {
            r = r.header("x-reeve-client", "web");
        }
        r.body(Body::from(
            json!({"version":v2,"title":"Via cookie"}).to_string(),
        ))
        .unwrap()
    };
    assert_eq!(call_raw(&h.app, patch(false)).await.status(), 403);
    assert_eq!(call_raw(&h.app, patch(true)).await.status(), 200);
}

/// Findings on an initiative and an epic are computed on read and advisory: a finding never blocks a move.
#[tokio::test]
async fn initiative_and_epic_findings_are_advisory() {
    let h = harness().await;
    let post = |path: String, body: serde_json::Value| {
        let (app, token) = (h.app.clone(), h.admin_token.clone());
        async move {
            let (st, b) = call(&app, Some(&token), "POST", &path, Some(body)).await;
            assert_eq!(st, 200, "{path}: {b}");
            b["value"].clone()
        }
    };
    let findings = |path: String| {
        let (app, token) = (h.app.clone(), h.victim_token.clone());
        async move {
            let (st, b) = call(&app, Some(&token), "GET", &path, None).await;
            assert_eq!(st, 200, "{path}: {b}");
            b["value"].clone()
        }
    };
    let ini = post(
        "/api/v1/initiatives".into(),
        json!({"idempotency_key":"i","title":"Leave the old host","horizon":"Now"}),
    )
    .await;
    let ik = ini["key"].as_str().unwrap().to_string();
    assert_eq!(
        findings(format!("/api/v1/initiatives/{ik}/findings")).await,
        json!([{"finding":"empty_brief"}])
    );
    let epic = post(
        "/api/v1/epics".into(),
        json!({"idempotency_key":"e","title":"Hosting","initiative":ik,"brief":"Migrated from Jira JPD-30."}),
    )
    .await;
    let ek = epic["key"].as_str().unwrap().to_string();
    let epic_findings = format!("/api/v1/epics/{ek}/findings");
    assert_eq!(
        findings(epic_findings.clone()).await,
        json!([{"finding":"brief_cites_without_decision","cites":["Jira"]}])
    );
    post(
        format!("/api/v1/epics/{ek}/discovery"),
        json!({"version": 1, "complete": true}),
    )
    .await;
    assert_eq!(
        findings(epic_findings.clone()).await,
        json!([
            {"finding":"brief_cites_without_decision","cites":["Jira"]},
            {"finding":"discovery_complete_without_stories"}
        ])
    );
    // a required decision on the epic answers the citation, and while open it is found against complete discovery
    let d = post(
        "/api/v1/decisions".into(),
        json!({"idempotency_key":"d","on":ek,"title":"Pick a host","decider":"dan@example.com","due":null}),
    )
    .await;
    assert_eq!(
        findings(epic_findings.clone()).await,
        json!([
            {"finding":"discovery_complete_with_open_decision","decision_id":d["id"],"title":"Pick a host"},
            {"finding":"discovery_complete_without_stories"}
        ])
    );
    // the initiative's findings are its own: its empty brief, not the epic's facts
    post(
        format!("/api/v1/decisions/{}/close", d["id"]),
        json!({"kind": "decided", "reason": "moot"}),
    )
    .await;
    post(
        format!("/api/v1/initiatives/{ik}/discovery"),
        json!({"version": 1, "complete": true}),
    )
    .await;
    let s = post(
        "/api/v1/stories".into(),
        json!({"idempotency_key":"s","title":"Move db","kind":"story","epic":ek,"criteria":["moved"],"labels":[]}),
    )
    .await;
    assert_eq!(
        findings(format!("/api/v1/initiatives/{ik}/findings")).await,
        json!([{"finding":"empty_brief"}])
    );
    // findings never block: the story is ready and moves while its initiative still has one
    let sk = s["key"].as_str().unwrap();
    let moved = post(
        format!("/api/v1/stories/{sk}/move"),
        json!({"version": 1, "to": "in_progress", "resolution": null, "override_reason": null}),
    )
    .await;
    assert_eq!(moved["delivery"], "in_progress", "{moved}");
    // read by an external reference too, and an unknown item is not found
    let (st, _) = call(
        &h.app,
        Some(&h.victim_token),
        "GET",
        "/api/v1/epics/TST-999/findings",
        None,
    )
    .await;
    assert_eq!(st, 404);
}

/// A story's owner is set on create and edit, cleared with null, and must be an allowed user.
#[tokio::test]
async fn a_story_owner_must_be_an_allowed_user() {
    let h = harness().await;
    let (st, b) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/stories",
        Some(json!({"idempotency_key":"s","title":"Owned","kind":"chore","criteria":[],"labels":[],"owner":"Anna@Example.com"})),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["value"]["owner"], "anna@example.com");
    let key = b["value"]["key"].as_str().unwrap().to_string();
    let (st, b) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/stories",
        Some(json!({"idempotency_key":"t","title":"Not ours","kind":"chore","criteria":[],"labels":[],"owner":"eve@elsewhere.org"})),
    )
    .await;
    assert_eq!(st, 400, "{b}");
    assert!(
        b["message"]
            .as_str()
            .unwrap()
            .contains("eve@elsewhere.org is not an allowed user"),
        "{b}"
    );
    let path = format!("/api/v1/stories/{key}");
    let (st, b) = call(
        &h.app,
        Some(&h.admin_token),
        "PATCH",
        &path,
        Some(json!({"version":1,"owner":"eve@elsewhere.org"})),
    )
    .await;
    assert_eq!(st, 400, "{b}");
    let (st, b) = call(
        &h.app,
        Some(&h.admin_token),
        "PATCH",
        &path,
        Some(json!({"version":1,"owner":null})),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["value"]["owner"], serde_json::Value::Null);
}

/// Any item takes comments by its key alone (`/items/{key}/comments`); a story's keep working by both routes.
#[tokio::test]
async fn any_item_takes_comments_by_its_key() {
    let h = harness().await;
    let mk = |path: &'static str, body: serde_json::Value| {
        let (app, token) = (h.app.clone(), h.admin_token.clone());
        async move {
            let (st, b) = call(&app, Some(&token), "POST", path, Some(body)).await;
            assert_eq!(st, 200, "{b}");
            b["value"]["key"].as_str().unwrap().to_string()
        }
    };
    let ini = mk(
        "/api/v1/initiatives",
        json!({"idempotency_key":"i","title":"Ini","horizon":"Now"}),
    )
    .await;
    let epic = mk(
        "/api/v1/epics",
        json!({"idempotency_key":"e","title":"Epic","initiative":ini}),
    )
    .await;
    let story = mk(
        "/api/v1/stories",
        json!({"idempotency_key":"s","title":"S","kind":"chore","criteria":[],"labels":[]}),
    )
    .await;
    for (key, field) in [
        (&ini, "on_initiative"),
        (&epic, "on_epic"),
        (&story, "on_story"),
    ] {
        let path = format!("/api/v1/items/{key}/comments");
        let body = json!({"body": format!("About {key}"), "idempotency_key": format!("c-{key}")});
        let (st, b) = call(
            &h.app,
            Some(&h.admin_token),
            "POST",
            &path,
            Some(body.clone()),
        )
        .await;
        assert_eq!(st, 200, "{b}");
        // a repeated key adds nothing
        call(&h.app, Some(&h.admin_token), "POST", &path, Some(body)).await;
        let (st, b) = call(&h.app, Some(&h.victim_token), "GET", &path, None).await;
        assert_eq!(st, 200, "{b}");
        let list = b["value"].as_array().unwrap();
        assert_eq!(list.len(), 1, "{key}: {b}");
        assert_eq!(list[0][field], key.as_str(), "{b}");
        assert_eq!(list[0]["body"], format!("About {key}"));
    }
    // the story's own route sees the same thread
    let (_, b) = call(
        &h.app,
        Some(&h.victim_token),
        "GET",
        &format!("/api/v1/stories/{story}/comments"),
        None,
    )
    .await;
    assert_eq!(b["value"].as_array().unwrap().len(), 1);
    // the comment is in the item's activity
    let (_, act) = call(
        &h.app,
        Some(&h.admin_token),
        "GET",
        &format!("/api/v1/activity?entity_type=epic&entity_key={epic}&action=commented"),
        None,
    )
    .await;
    assert_eq!(act["value"].as_array().unwrap().len(), 1, "{act}");
    let (st, _) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        "/api/v1/items/TST-999/comments",
        Some(json!({"body":"x"})),
    )
    .await;
    assert_eq!(st, 404);
}

/// An initiative's decisions are those on it, its epics and their stories, read where they live, never copied.
#[tokio::test]
async fn an_initiative_lists_every_decision_under_it() {
    let h = harness().await;
    let mk = |path: &'static str, body: serde_json::Value| {
        let (app, token) = (h.app.clone(), h.admin_token.clone());
        async move {
            let (st, b) = call(&app, Some(&token), "POST", path, Some(body)).await;
            assert_eq!(st, 200, "{b}");
            b["value"].clone()
        }
    };
    let ini = mk(
        "/api/v1/initiatives",
        json!({"idempotency_key":"i","title":"Ini","horizon":"Now"}),
    )
    .await["key"]
        .clone();
    let other = mk(
        "/api/v1/initiatives",
        json!({"idempotency_key":"o","title":"Other","horizon":"Now"}),
    )
    .await["key"]
        .clone();
    let epic = mk(
        "/api/v1/epics",
        json!({"idempotency_key":"e","title":"Epic","initiative":ini}),
    )
    .await["key"]
        .clone();
    let story = mk("/api/v1/stories", json!({"idempotency_key":"s","title":"S","kind":"story","epic":epic,"criteria":["x"],"labels":[]})).await["key"].clone();
    let decide = |idem: &'static str, on: serde_json::Value| {
        mk(
            "/api/v1/decisions",
            json!({"idempotency_key":idem,"on":on,"title":idem,"decider":"dan@example.com","due":null}),
        )
    };
    let d_ini = decide("on-ini", ini.clone()).await["id"].clone();
    let d_epic = decide("on-epic", epic.clone()).await["id"].clone();
    let d_story = decide("on-story", story.clone()).await["id"].clone();
    decide("elsewhere", other.clone()).await;
    // closing one shows everywhere: it is the same decision
    mk(
        &*Box::leak(format!("/api/v1/decisions/{d_story}/close").into_boxed_str()),
        json!({"kind":"decided","reason":"moot"}),
    )
    .await;

    let (st, b) = call(
        &h.app,
        Some(&h.victim_token),
        "GET",
        &format!("/api/v1/decisions?initiative={}", ini.as_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(st, 200, "{b}");
    let got: Vec<(
        serde_json::Value,
        serde_json::Value,
        serde_json::Value,
        bool,
    )> = b["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["id"].clone(),
                d["on"].clone(),
                d["on_type"].clone(),
                d["closed"].is_null(),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (d_ini, ini.clone(), json!("initiative"), true),
            (d_epic, epic.clone(), json!("epic"), true),
            (d_story, story.clone(), json!("story"), false),
        ]
    );
    // the epic's own list is unchanged: one decision, the one on it
    let (_, b) = call(
        &h.app,
        Some(&h.victim_token),
        "GET",
        &format!("/api/v1/decisions?on={}", epic.as_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(b["value"].as_array().unwrap().len(), 1);
}

/// Spec §1: `GET /decisions/queue` answers groups in server order with the stories each blocking decision holds
/// back. `/decisions/queue` must not be captured by `/decisions/{id}` (a capture answers 400 on a non-numeric id).
#[tokio::test]
async fn the_decision_queue_groups_orders_and_scopes() {
    let h = harness().await;
    let t = Some(h.admin_token.as_str());
    let mk = |path: String, body: serde_json::Value| {
        let app = h.app.clone();
        let token = h.admin_token.clone();
        async move {
            let (st, v) = call(&app, Some(&token), "POST", &path, Some(body)).await;
            assert_eq!(st, 200, "{v}");
            v["value"].clone()
        }
    };
    let ini = mk(
        "/api/v1/initiatives".into(),
        json!({"idempotency_key":"q-i","title":"I","horizon":"Now"}),
    )
    .await["key"]
        .clone();
    let other = mk(
        "/api/v1/initiatives".into(),
        json!({"idempotency_key":"q-o","title":"O","horizon":"Now"}),
    )
    .await["key"]
        .clone();
    let e1 = mk(
        "/api/v1/epics".into(),
        json!({"idempotency_key":"q-e1","title":"E1","initiative":other}),
    )
    .await["key"]
        .clone();
    let e2 = mk(
        "/api/v1/epics".into(),
        json!({"idempotency_key":"q-e2","title":"E2","initiative":ini}),
    )
    .await["key"]
        .clone();
    let story = |idem: &'static str, epic: serde_json::Value| {
        mk(
            "/api/v1/stories".into(),
            json!({"idempotency_key":idem,"title":idem,"kind":"story","epic":epic,"criteria":["x"],"labels":[]}),
        )
    };
    let s1 = story("q-s1", e1.clone()).await["key"].clone();
    let s2 = story("q-s2", e1.clone()).await["key"].clone();
    let s4 = story("q-s4", e2.clone()).await["key"].clone();
    let decide = |idem: &'static str, on: serde_json::Value| {
        mk(
            "/api/v1/decisions".into(),
            json!({"idempotency_key":idem,"on":on,"title":idem,"decider":"dan@example.com","due":null}),
        )
    };
    let d_epic = decide("q-d-epic", e1.clone()).await["id"].as_i64().unwrap();
    let d_ini = decide("q-d-ini", ini.clone()).await["id"].as_i64().unwrap();
    let d_s4 = decide("q-d-s4", s4.clone()).await["id"].as_i64().unwrap();
    // satisfied: linked to a record the admin's check found accepted
    let d_sat = decide("q-d-sat", s1.clone()).await;
    let sat = d_sat["id"].as_i64().unwrap();
    let linked = mk(
        format!("/api/v1/decisions/{sat}/link"),
        json!({"repo":"r","path":"p.md","record_id":"ADR-0001","pr_url":null}),
    )
    .await;
    mk(format!("/api/v1/decisions/{sat}/check"), json!({"record_id":"ADR-0001","version":linked["version"],"check":{"kind":"accepted","commit":"abc"}})).await;
    let d_closed = decide("q-d-closed", s2.clone()).await["id"]
        .as_i64()
        .unwrap();
    mk(
        format!("/api/v1/decisions/{d_closed}/close"),
        json!({"kind":"withdrawn","reason":"moot"}),
    )
    .await;

    let get = |q: String| {
        let app = h.app.clone();
        let token = h.admin_token.clone();
        async move {
            call(
                &app,
                Some(&token),
                "GET",
                &format!("/api/v1/decisions/queue{q}"),
                None,
            )
            .await
        }
    };
    let ids = |v: &serde_json::Value| -> Vec<(i64, String)> {
        v["value"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                (
                    e["decision"]["id"].as_i64().unwrap(),
                    e["group"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    };
    let (st, b) = get(String::new()).await;
    assert_eq!(st, 200, "{b}");
    let got = ids(&b);
    assert_eq!(
        got.iter().map(|g| g.1.as_str()).collect::<Vec<_>>(),
        ["blocking", "blocking", "blocking", "satisfied"],
        "{b}"
    );
    let mut blocking: Vec<i64> = got[..3].iter().map(|g| g.0).collect();
    blocking.sort();
    assert_eq!(blocking, [d_epic, d_ini, d_s4]);
    assert_eq!(got[3].0, sat);
    let entry = |id: i64| {
        b["value"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["decision"]["id"] == id)
            .unwrap()
            .clone()
    };
    let mut keys: Vec<_> = entry(d_epic)["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["key"].clone())
        .collect();
    keys.sort_by_key(|k| k.as_str().unwrap().to_string());
    assert_eq!(keys, [s1.clone(), s2.clone()]);
    let mut titles: Vec<_> = entry(d_epic)["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["title"].as_str().unwrap().to_string())
        .collect();
    titles.sort();
    assert_eq!(titles, ["q-s1", "q-s2"]);
    assert_eq!(entry(d_ini)["blocks"].as_array().unwrap().len(), 1);
    assert_eq!(entry(d_ini)["blocks"][0]["key"], s4);
    assert_eq!(entry(sat)["blocks"], json!([]));
    // the satisfied one is never counted
    assert_eq!(
        b["value"]["summary"],
        // all three sit under a first-bucket initiative still in discovery, so each is needed next sprint
        json!({"open":3,"blocking":3,"blocked_stories":3,"overdue":0,"review":0,"this_sprint":0,"next_sprint":3})
    );

    let (st, b) = get("?include_closed=true".into()).await;
    assert_eq!(st, 200, "{b}");
    let got = ids(&b);
    assert_eq!(
        got.last().unwrap(),
        &(d_closed, "closed".to_string()),
        "{b}"
    );
    assert_eq!(got.len(), 5);
    assert_eq!(b["value"]["summary"]["open"], 3);

    let (st, b) = get(format!("?on={}", e1.as_str().unwrap())).await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(ids(&b), [(d_epic, "blocking".to_string())]);
    let (st, b) = get(format!("?initiative={}", ini.as_str().unwrap())).await;
    assert_eq!(st, 200, "{b}");
    // the initiative's own decision and the one under it, not just those on the initiative
    let mut under = ids(&b);
    under.sort();
    assert_eq!(
        under,
        [
            (d_ini, "blocking".to_string()),
            (d_s4, "blocking".to_string())
        ]
    );

    // an unknown reference matches nothing, like GET /decisions
    for q in ["?on=ABC-9999", "?initiative=ABC-9999"] {
        let (st, b) = get(q.into()).await;
        assert_eq!(st, 200, "{q}: {b}");
        assert_eq!(b["value"]["entries"], json!([]));
        assert_eq!(
            b["value"]["summary"],
            json!({"open":0,"blocking":0,"blocked_stories":0,"overdue":0,"review":0,"this_sprint":0,"next_sprint":0})
        );
    }
    let (st, b) = get(format!(
        "?on={}&initiative={}",
        e1.as_str().unwrap(),
        ini.as_str().unwrap()
    ))
    .await;
    assert_eq!((st, b["code"].as_str()), (400, Some("validation")), "{b}");

    // the plain list keeps its shape
    let (st, b) = call(&h.app, t, "GET", "/api/v1/decisions", None).await;
    assert_eq!(st, 200, "{b}");
    let all = b["value"].as_array().unwrap();
    assert_eq!(all.len(), 4, "{b}");
    for d in all {
        assert!(d["id"].as_i64().is_some());
        assert!(d.get("blocks").is_none());
    }
}

/// POSTs `body` as the admin and returns the `value`, failing the test on anything but 200.
async fn admin_post(h: &Harness, path: &str, body: serde_json::Value) -> serde_json::Value {
    let (st, v) = call(&h.app, Some(&h.admin_token), "POST", path, Some(body)).await;
    assert_eq!(st, 200, "{path}: {v}");
    v["value"].clone()
}

async fn admin_get(h: &Harness, path: &str) -> (u16, serde_json::Value) {
    let (st, v) = call(&h.app, Some(&h.admin_token), "GET", path, None).await;
    (st, v)
}

/// A chore (ready without an epic or criteria), left todo.
async fn chore(h: &Harness, idem: &str) -> String {
    admin_post(
        h,
        "/api/v1/stories",
        json!({"idempotency_key":idem,"title":idem,"kind":"chore","criteria":[],"labels":[]}),
    )
    .await["key"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn open_decision(h: &Harness, idem: &str, on: &str) -> i64 {
    admin_post(
        h,
        "/api/v1/decisions",
        json!({"idempotency_key":idem,"on":on,"title":idem,"decider":"dan@example.com","due":null}),
    )
    .await["id"]
        .as_i64()
        .unwrap()
}

/// Spec "Decisions on the board": `GET /decisions/{id}/context` answers the queue entry of one decision (lane, window,
/// next step, what it holds), 404 for an id that does not exist, and it is not captured by `/decisions/{id}`.
#[tokio::test]
async fn decision_context_route() {
    let h = harness().await;
    let s1 = chore(&h, "c-s1").await;
    let d = open_decision(&h, "c-d", &s1).await;
    let (st, v) = admin_get(&h, &format!("/api/v1/decisions/{d}/context")).await;
    assert_eq!(st, 200, "{v}");
    let e = &v["value"];
    assert_eq!(e["decision"]["id"], d);
    assert_eq!(e["group"], "blocking");
    assert_eq!(e["lane"], "beyond", "{e}");
    assert_eq!(e["next_step"], json!({"kind":"write_record"}), "{e}");
    assert_eq!(e["blocks"].as_array().unwrap().len(), 1, "{e}");
    assert_eq!(e["blocks"][0]["key"], s1);
    assert_eq!(e["blocks"][0]["kind"], "chore");
    assert_eq!(e["blocks"][0]["delivery"], "todo");
    assert_eq!(e["under_way"], json!([]));
    let (st, v) = admin_get(&h, "/api/v1/decisions/999999/context").await;
    assert_eq!(st, 404, "{v}");
}

/// Spec A5.1: `browse_url` is derived from `[[repos]].url` on every response that carries a decision. An `https://`
/// URL loses `.git`, a trailing `/` and any credentials; an scp-style `git@host:org/repo` and an `ssh://git@host:port/org/repo` URL give the https form; `http://` and a missing URL give none; each path
/// segment is percent-encoded. The export keeps the field out.
#[tokio::test]
async fn browse_url_from_repo_config() {
    let h = harness_with(|cfg| {
        cfg.replace(
            "[[repos]]\nname = \"orion\"\nadr_dir = \"docs/adr\"",
            "[[repos]]\nname = \"orion\"\nadr_dir = \"docs/adr\"\nurl = \"https://github.com/acme/orion.git\"\n\
             [[repos]]\nname = \"slash\"\nadr_dir = \"docs/adr\"\nurl = \"https://github.com/acme/slash/\"\n\
             [[repos]]\nname = \"cred\"\nadr_dir = \"docs/adr\"\nurl = \"https://bot:SECRET-TOKEN@github.com/acme/cred.git\"\n\
             [[repos]]\nname = \"plain\"\nadr_dir = \"docs/adr\"\nurl = \"http://git.example.com/acme/plain\"\n\
             [[repos]]\nname = \"ssh\"\nadr_dir = \"docs/adr\"\nurl = \"git@github.com:acme/ssh.git\"\n\
             [[repos]]\nname = \"sshurl\"\nadr_dir = \"docs/adr\"\nurl = \"ssh://git:SECRET@github.com:2222/acme/sshurl.git\"\n\
             [[repos]]\nname = \"alias\"\nadr_dir = \"docs/adr\"\nurl = \"git@github-kb:acme/alias.git\"\n\
             [[repos]]\nname = \"aliasset\"\nadr_dir = \"docs/adr\"\nurl = \"git@github-kb:acme/aliasset.git\"\nbrowse_url = \"https://github.com/acme/aliasset\"\n\
             [[repos]]\nname = \"bare\"\nadr_dir = \"docs/adr\"",
        )
    })
    .await;
    let s = chore(&h, "b-s").await;
    let cases: [(&str, &str, Option<&str>); 10] = [
        (
            "orion",
            "docs/adr/0001.md",
            Some("https://github.com/acme/orion/blob/HEAD/docs/adr/0001.md"),
        ),
        (
            "slash",
            "docs/adr/0002.md",
            Some("https://github.com/acme/slash/blob/HEAD/docs/adr/0002.md"),
        ),
        (
            "cred",
            "docs/adr/0003.md",
            Some("https://github.com/acme/cred/blob/HEAD/docs/adr/0003.md"),
        ),
        ("plain", "docs/adr/0004.md", None),
        (
            "ssh",
            "docs/adr/0005.md",
            Some("https://github.com/acme/ssh/blob/HEAD/docs/adr/0005.md"),
        ),
        ("bare", "docs/adr/0006.md", None),
        ("alias", "docs/adr/0008.md", None),
        (
            "aliasset",
            "docs/adr/0009.md",
            Some("https://github.com/acme/aliasset/blob/HEAD/docs/adr/0009.md"),
        ),
        (
            "sshurl",
            "docs/adr/0007.md",
            Some("https://github.com/acme/sshurl/blob/HEAD/docs/adr/0007.md"),
        ),
        // spaces, '#', '?' and '%' are encoded per segment; '/' stays
        (
            "orion",
            "docs/a b/c#d?e%f.md",
            Some("https://github.com/acme/orion/blob/HEAD/docs/a%20b/c%23d%3Fe%25f.md"),
        ),
    ];
    let mut ids = Vec::new();
    for (i, (repo, path, want)) in cases.iter().enumerate() {
        let id = open_decision(&h, &format!("b-{i}"), &s).await;
        let linked = admin_post(
            &h,
            &format!("/api/v1/decisions/{id}/link"),
            json!({"repo":repo,"path":path,"record_id":"ADR-1","pr_url":null}),
        )
        .await;
        let want = want.map(|w| json!(w));
        assert_eq!(
            linked["record"].get("browse_url").cloned(),
            want,
            "link {i}: {linked}"
        );
        ids.push((id, i, want));
    }
    for (id, i, want) in &ids {
        let (st, one) = admin_get(&h, &format!("/api/v1/decisions/{id}")).await;
        assert_eq!(st, 200, "{one}");
        assert_eq!(
            one["value"]["record"].get("browse_url").cloned(),
            *want,
            "get {i}"
        );
        let (_, ctx) = admin_get(&h, &format!("/api/v1/decisions/{id}/context")).await;
        assert_eq!(
            ctx["value"]["decision"]["record"]
                .get("browse_url")
                .cloned(),
            *want,
            "context {i}"
        );
        let check = admin_post(
            &h,
            &format!("/api/v1/decisions/{id}/check"),
            json!({"record_id":"ADR-1","version":one["value"]["version"],"check":{"kind":"not_found"}}),
        )
        .await;
        assert_eq!(
            check["record"].get("browse_url").cloned(),
            *want,
            "check {i}"
        );
        let (_, one) = admin_get(&h, &format!("/api/v1/decisions/{id}")).await;
        let (st, edited) = call(
            &h.app,
            Some(&h.admin_token),
            "PATCH",
            &format!("/api/v1/decisions/{id}"),
            Some(json!({"version":one["value"]["version"],"title":format!("edited {i}")})),
        )
        .await;
        assert_eq!(st, 200, "{edited}");
        assert_eq!(
            edited["value"]["record"].get("browse_url").cloned(),
            *want,
            "edit {i}"
        );
    }
    let (_, list) = admin_get(&h, "/api/v1/decisions").await;
    let (_, queue) = admin_get(&h, "/api/v1/decisions/queue").await;
    assert_eq!(list["value"].as_array().unwrap().len(), cases.len());
    assert_eq!(
        queue["value"]["entries"].as_array().unwrap().len(),
        cases.len()
    );
    for (id, i, want) in &ids {
        let in_list = list["value"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == *id)
            .unwrap();
        assert_eq!(
            in_list["record"].get("browse_url").cloned(),
            *want,
            "list {i}"
        );
        let in_queue = queue["value"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["decision"]["id"] == *id)
            .unwrap();
        assert_eq!(
            in_queue["decision"]["record"].get("browse_url").cloned(),
            *want,
            "queue {i}"
        );
    }
    // the story context's decisions carry it too, each its own
    let (_, sctx) = admin_get(&h, &format!("/api/v1/stories/{s}/context")).await;
    let in_ctx = sctx["value"]["required_decisions"].as_array().unwrap();
    assert_eq!(in_ctx.len(), cases.len(), "{sctx}");
    for (id, i, want) in &ids {
        let d = in_ctx.iter().find(|d| d["id"] == *id).unwrap();
        assert_eq!(
            d["record"].get("browse_url").cloned(),
            *want,
            "story context {i}"
        );
    }
    // the export is a document of the planner's own facts: no derived link, and no credential from configuration
    let (st, exp) = admin_get(&h, "/api/v1/export").await;
    assert_eq!(st, 200);
    let text = exp.to_string();
    assert!(
        text.contains("docs/adr/0001.md"),
        "the export holds the decisions"
    );
    assert!(
        !text.contains("browse_url"),
        "the export must not carry browse_url"
    );
    assert!(!text.contains("SECRET-TOKEN"));
    // close answers with it too
    for (id, i, want) in &ids {
        let closed = admin_post(
            &h,
            &format!("/api/v1/decisions/{id}/close"),
            json!({"kind":"withdrawn","reason":"moot"}),
        )
        .await;
        assert_eq!(
            closed["record"].get("browse_url").cloned(),
            *want,
            "close {i}"
        );
    }
    // a decision with no record has none
    let bare = open_decision(&h, "b-none", &s).await;
    let (_, one) = admin_get(&h, &format!("/api/v1/decisions/{bare}")).await;
    assert_eq!(one["value"]["record"], serde_json::Value::Null);
}

/// Spec A5.2: the queue answers lanes in `QueueLane` order, whatever the group, so the groups interleave: an open-group
/// entry in this_sprint comes before a blocking-group entry in next_sprint, and both before beyond and satisfied.
#[tokio::test]
async fn queue_entries_in_lane_order() {
    let h = harness().await;
    let mv = |key: String, version: serde_json::Value, to: &'static str| {
        let h = &h;
        async move {
            admin_post(
                h,
                &format!("/api/v1/stories/{key}/move"),
                json!({"version":version,"to":to,"override_reason":"test seeding"}),
            )
            .await
        }
    };
    let this_story = chore(&h, "l-this").await;
    let next_story = chore(&h, "l-next").await;
    let far_story = chore(&h, "l-far").await;
    let sat_story = chore(&h, "l-sat").await;
    // goal one serves the this-sprint story; opening makes it active and creates the next draft
    let g1 = admin_post(
        &h,
        "/api/v1/goals",
        json!({"idempotency_key":"l-g1","outcome":"Ship the thing","check":"shipped"}),
    )
    .await;
    let g1id = g1["goal"]["id"].as_i64().unwrap();
    admin_post(
        &h,
        &format!("/api/v1/goals/{g1id}/links"),
        json!({"items":[this_story]}),
    )
    .await;
    let (_, sp) = admin_get(&h, "/api/v1/sprints").await;
    let draft = sp["value"]["draft"].clone();
    admin_post(
        &h,
        &format!("/api/v1/sprints/{}/open", draft["id"]),
        json!({"version":draft["version"]}),
    )
    .await;
    // goal two (in the new draft) serves the next-sprint story
    let g2 = admin_post(
        &h,
        "/api/v1/goals",
        json!({"idempotency_key":"l-g2","outcome":"Then this","check":"done"}),
    )
    .await;
    let g2id = g2["goal"]["id"].as_i64().unwrap();
    admin_post(
        &h,
        &format!("/api/v1/goals/{g2id}/links"),
        json!({"items":[next_story]}),
    )
    .await;
    // the this-sprint story is started before its decision exists, so the decision holds nothing (open group)
    let st = admin_get(&h, &format!("/api/v1/stories/{this_story}"))
        .await
        .1["value"]
        .clone();
    mv(this_story.clone(), st["version"].clone(), "in_progress").await;

    let d_far = open_decision(&h, "l-d-far", &far_story).await;
    let d_next = open_decision(&h, "l-d-next", &next_story).await;
    let d_this = open_decision(&h, "l-d-this", &this_story).await;
    let d_sat = open_decision(&h, "l-d-sat", &sat_story).await;
    let linked = admin_post(
        &h,
        &format!("/api/v1/decisions/{d_sat}/link"),
        json!({"repo":"orion","path":"p.md","record_id":"ADR-9","pr_url":null}),
    )
    .await;
    admin_post(&h, &format!("/api/v1/decisions/{d_sat}/check"), json!({"record_id":"ADR-9","version":linked["version"],"check":{"kind":"accepted","commit":"abc"}})).await;

    let (st, q) = admin_get(&h, "/api/v1/decisions/queue").await;
    assert_eq!(st, 200, "{q}");
    let entries = q["value"]["entries"].as_array().unwrap();
    // the count first, so an empty or one-entry answer fails here and not on an index
    assert_eq!(entries.len(), 4, "{q}");
    let got: Vec<(i64, &str, &str)> = entries
        .iter()
        .map(|e| {
            (
                e["decision"]["id"].as_i64().unwrap(),
                e["group"].as_str().unwrap(),
                e["lane"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (d_this, "open", "this_sprint"),
            (d_next, "blocking", "next_sprint"),
            (d_far, "blocking", "beyond"),
            (d_sat, "satisfied", "satisfied"),
        ],
        "{q}"
    );
    assert_eq!(entries[0]["under_way"][0]["key"], this_story, "{q}");
    assert_eq!(entries[0]["under_way"][0]["delivery"], "in_progress");
    let sum = &q["value"]["summary"];
    assert_eq!(
        (
            sum["review"].as_u64(),
            sum["this_sprint"].as_u64(),
            sum["next_sprint"].as_u64()
        ),
        (Some(0), Some(1), Some(1)),
        "{sum}"
    );
}

/// Every versioned write answers a stale version with the conflict envelope carrying the row's version now, so a CLI
/// or web caller can re-read and retry without parsing the message. Story move, story rank and decision edit are pinned
/// with their routes above; this covers the rest.
#[tokio::test]
async fn every_versioned_write_reports_current_on_conflict() {
    let h = harness().await;
    let t = Some(h.admin_token.as_str());
    let created = |path: &'static str, body: serde_json::Value| {
        let app = h.app.clone();
        async move {
            let (st, v) = call(&app, t, "POST", path, Some(body)).await;
            assert_eq!(st, 200, "{v}");
            v["value"].clone()
        }
    };
    let ini = created(
        "/api/v1/initiatives",
        json!({"idempotency_key":"cv-i","title":"I","horizon":"Now"}),
    )
    .await;
    let ik = ini["key"].as_str().unwrap().to_string();
    let epic = created(
        "/api/v1/epics",
        json!({"idempotency_key":"cv-e","title":"E","initiative":ik}),
    )
    .await;
    let ek = epic["key"].as_str().unwrap().to_string();
    let story = created(
        "/api/v1/stories",
        json!({"idempotency_key":"cv-s","title":"S","kind":"chore","criteria":[],"labels":[]}),
    )
    .await;
    let sk = story["key"].as_str().unwrap().to_string();
    let d = created(
        "/api/v1/decisions",
        json!({"idempotency_key":"cv-d","on":sk,"title":"Pick","decider":"dan@example.com"}),
    )
    .await;
    let id = d["id"].as_i64().unwrap();
    let (st, linked) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/decisions/{id}/link"),
        Some(json!({"repo":"orion","path":"docs/adr/0001.md","record_id":"ADR-0001"})),
    )
    .await;
    assert_eq!(st, 200, "{linked}");

    let cases = [
        (
            "PATCH",
            format!("/api/v1/initiatives/{ik}"),
            &ini,
            json!({"title":"x"}),
        ),
        (
            "POST",
            format!("/api/v1/initiatives/{ik}/discovery"),
            &ini,
            json!({"complete":true}),
        ),
        (
            "PATCH",
            format!("/api/v1/epics/{ek}"),
            &epic,
            json!({"title":"x"}),
        ),
        (
            "POST",
            format!("/api/v1/epics/{ek}/discovery"),
            &epic,
            json!({"complete":true}),
        ),
        (
            "PATCH",
            format!("/api/v1/stories/{sk}"),
            &story,
            json!({"title":"x"}),
        ),
        (
            "POST",
            format!("/api/v1/decisions/{id}/check"),
            &linked["value"],
            json!({"record_id":"ADR-0001","check":{"kind":"proposed_pr_open"}}),
        ),
    ];
    for (method, path, row, mut body) in cases {
        let current = row["version"].as_i64().unwrap();
        body["version"] = json!(current + 5);
        let (st, b) = call(&h.app, t, method, &path, Some(body)).await;
        assert_eq!(st, 409, "{method} {path}: {b}");
        assert_eq!(
            b,
            json!({"outcome":"error","code":"conflict","message":format!("version conflict: current is {current}"),"current":current}),
            "{method} {path}"
        );
    }
}

#[tokio::test]
async fn urgent_routes_mark_clear_and_list_the_war_room() {
    let h = harness().await;
    let t = Some(h.victim_token.as_str());
    let (_, i) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/initiatives",
        Some(json!({"idempotency_key":"u1","title":"War","horizon":"Now"})),
    )
    .await;
    let key = i["value"]["key"].as_str().unwrap().to_string();
    assert_eq!(i["value"]["urgent_sources"], json!([]));
    assert_eq!(
        i["value"].get("urgent_mark"),
        Some(&serde_json::Value::Null)
    );
    let (st, b) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/initiatives/{key}/urgent"),
        Some(json!({"reason":"Customer X"})),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["value"]["urgent_mark"]["reason"], "Customer X");
    assert_eq!(b["value"]["urgent_mark"]["active"], true);
    assert_eq!(
        b["value"]["urgent_sources"],
        json!([{"key": key, "item": "initiative"}])
    );
    let (_, room) = call(&h.app, t, "GET", "/api/v1/war-room", None).await;
    assert_eq!(
        room["value"][0]["item"],
        json!({"key": key, "item": "initiative"})
    );
    assert_eq!(room["value"][0]["mark"]["active"], true);
    assert_eq!(room["value"][0]["mark"]["reason"], "Customer X");
    let (st, _) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/war-room/clear",
        Some(json!({"items":[{"key": key, "item":"initiative"}]})),
    )
    .await;
    assert_eq!(st, 200);
    let (_, room) = call(&h.app, t, "GET", "/api/v1/war-room", None).await;
    assert_eq!(room["value"], json!([]));

    // The same round trip for an epic and a story, in a chain under the initiative.
    let (_, e) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/epics",
        Some(json!({"idempotency_key":"u1e","title":"E","initiative":key})),
    )
    .await;
    let ek = e["value"]["key"].as_str().unwrap().to_string();
    let (_, sy) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/stories",
        Some(json!({"idempotency_key":"u1s","title":"S","kind":"story","epic":ek,"criteria":["ok"],"labels":[]})),
    )
    .await;
    let sk = sy["value"]["key"].as_str().unwrap().to_string();
    // A marked story makes its lineage urgent on every read: the initiative, the epic and the story itself.
    let (st, b) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/stories/{sk}/urgent"),
        Some(json!({})),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    let from_story = json!([{"key": sk, "item": "story"}]);
    assert_eq!(b["value"]["urgent_sources"], from_story);
    for path in [
        format!("/api/v1/initiatives/{key}"),
        format!("/api/v1/epics/{ek}"),
        format!("/api/v1/stories/{sk}"),
    ] {
        let (_, got) = call(&h.app, t, "GET", &path, None).await;
        assert_eq!(got["value"]["urgent_sources"], from_story, "{path}");
    }
    let (_, epics) = call(&h.app, t, "GET", "/api/v1/epics", None).await;
    assert_eq!(epics["value"][0]["urgent_sources"], from_story);
    let (st, b) = call(
        &h.app,
        t,
        "DELETE",
        &format!("/api/v1/stories/{sk}/urgent"),
        None,
    )
    .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(b["value"]["urgent_sources"], json!([]));
    assert_eq!(b["value"]["urgent_mark"], serde_json::Value::Null);
    for (plural, item, k) in [
        ("initiatives", "initiative", &key),
        ("epics", "epic", &ek),
        ("stories", "story", &sk),
    ] {
        let path = format!("/api/v1/{plural}/{k}/urgent");
        let (st, b) = call(&h.app, t, "POST", &path, Some(json!({}))).await;
        assert_eq!(st, 200, "{item}: {b}");
        let (_, room) = call(&h.app, t, "GET", "/api/v1/war-room", None).await;
        let row = json!({"key": k, "item": item});
        assert!(
            room["value"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["item"] == row),
            "{item} listed: {room}"
        );
        let (st, b) = call(&h.app, t, "DELETE", &path, None).await;
        assert_eq!(st, 200, "{item}: {b}");
        let (_, room) = call(&h.app, t, "GET", "/api/v1/war-room", None).await;
        assert!(
            !room["value"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["item"] == row),
            "{item} gone: {room}"
        );
    }

    // Renew: mark, then renew the row; the returned list has it, active.
    let (st, _) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/epics/{ek}/urgent"),
        Some(json!({"reason":"again"})),
    )
    .await;
    assert_eq!(st, 200);
    let (st, room) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/war-room/renew",
        Some(json!({"items":[{"key": ek, "item":"epic"}]})),
    )
    .await;
    assert_eq!(st, 200, "{room}");
    let row = room["value"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["item"] == json!({"key": ek, "item": "epic"}))
        .expect("renewed row listed");
    assert_eq!(row["mark"]["active"], true);
}

#[tokio::test]
async fn urgent_writes_refuse_automation_and_a_wrong_typed_item_fails() {
    let h = harness().await;
    let t = Some(h.victim_token.as_str());
    let bot = Some(h.bot_token.as_str());
    let (_, i) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/initiatives",
        Some(json!({"idempotency_key":"u2","title":"War","horizon":"Now"})),
    )
    .await;
    let key = i["value"]["key"].as_str().unwrap().to_string();
    let items = json!({"items":[{"key": key, "item":"initiative"}]});
    let calls = [
        (
            "POST",
            format!("/api/v1/initiatives/{key}/urgent"),
            Some(json!({})),
        ),
        ("DELETE", format!("/api/v1/initiatives/{key}/urgent"), None),
        (
            "POST",
            "/api/v1/war-room/clear".to_string(),
            Some(items.clone()),
        ),
        ("POST", "/api/v1/war-room/renew".to_string(), Some(items)),
    ];
    for (m, p, b) in calls {
        let (st, body) = call(&h.app, bot, m, &p, b).await;
        assert_eq!(st, 422, "{m} {p}: {body}");
        assert_eq!(body["outcome"], "refused", "{m} {p}");
        assert_eq!(body["reasons"], json!([{"reason": "interactive_only"}]));
    }
    // The refused writes wrote nothing.
    let (_, room) = call(&h.app, t, "GET", "/api/v1/war-room", None).await;
    assert_eq!(room["value"], json!([]));
    // The key names an initiative, not an epic: the whole request fails.
    let (st, body) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/war-room/clear",
        Some(json!({"items":[{"key": key, "item":"epic"}]})),
    )
    .await;
    assert_eq!(st, 400, "{body}");
    assert_eq!(body["code"], "validation");
}

/// kb/ADR-0025 R2 over the wire: the promise routes answer with the item, refuse automation with the same envelope as
/// the urgent routes, refuse a done story with the domain's text, and exist only for stories and epics (R1).
#[tokio::test]
async fn promise_routes_answer_with_the_item_and_refuse_as_urgent_does() {
    let h = harness().await;
    let t = Some(h.admin_token.as_str());
    let bot = Some(h.bot_token.as_str());
    // An active sprint, so `this_sprint` has a planned end.
    admin_post(
        &h,
        "/api/v1/goals",
        json!({"idempotency_key": "g", "outcome": "o", "check": "c"}),
    )
    .await;
    let (_, all) = admin_get(&h, "/api/v1/sprints").await;
    let draft = all["value"]["draft"].clone();
    let opened = admin_post(
        &h,
        &format!("/api/v1/sprints/{}/open", draft["id"]),
        json!({"version": draft["version"]}),
    )
    .await;
    let ends = opened["ends_on"].as_str().unwrap().to_string();
    let story = chore(&h, "p1").await;
    let done = chore(&h, "p2").await;
    let epic = admin_post(
        &h,
        "/api/v1/epics",
        json!({"idempotency_key": "pe", "title": "E", "description": ""}),
    )
    .await["key"]
        .as_str()
        .unwrap()
        .to_string();

    let (st, v) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/stories/{story}/promise"),
        Some(json!({"shortcut": "this_sprint", "to": "Acme"})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["value"]["key"], story.as_str());
    assert_eq!(v["value"]["promise"]["own"]["on"], ends.as_str());
    assert_eq!(v["value"]["promise"]["own"]["to"], "Acme");
    assert_eq!(v["value"]["promise"]["due"], "this_sprint");
    let story_promise = v["value"]["promise"].clone();
    let (st, v) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/epics/{epic}/promise"),
        Some(json!({"on": ends})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["value"]["key"], epic.as_str());
    assert_eq!(v["value"]["promise"]["fact"]["on"], ends.as_str());
    let epic_promise = v["value"]["promise"].clone();

    for (m, p, b) in [
        (
            "POST",
            format!("/api/v1/stories/{story}/promise"),
            Some(json!({"on": ends})),
        ),
        ("DELETE", format!("/api/v1/stories/{story}/promise"), None),
        (
            "POST",
            format!("/api/v1/epics/{epic}/promise"),
            Some(json!({"on": ends})),
        ),
        ("DELETE", format!("/api/v1/epics/{epic}/promise"), None),
    ] {
        let (st, body) = call(&h.app, bot, m, &p, b).await;
        assert_eq!(st, 422, "{m} {p}: {body}");
        assert_eq!(body["outcome"], "refused", "{m} {p}");
        assert_eq!(body["reasons"], json!([{"reason": "interactive_only"}]));
        assert_eq!(body["overridable"], false, "{m} {p}");
    }
    // The refused writes changed nothing.
    let (_, sv) = admin_get(&h, &format!("/api/v1/stories/{story}")).await;
    assert_eq!(sv["value"]["promise"], story_promise);
    let (_, ev) = admin_get(&h, &format!("/api/v1/epics/{epic}")).await;
    assert_eq!(ev["value"]["promise"], epic_promise);
    // The sprint view lists the promise (kb/ADR-0025 R8): the active sprint's list is read from `GET /sprints/{id}`;
    // `GET /sprints` leaves it empty and `promises_after` null until a client renders it there (GRA-610).
    let (_, one) = admin_get(&h, &format!("/api/v1/sprints/{}", opened["id"])).await;
    let promises = &one["value"]["promises"];
    assert_eq!(promises.as_array().map(Vec::len), Some(1), "{one}");
    assert_eq!(promises[0]["key"], story.as_str(), "{one}");
    assert_eq!(promises[0]["on"], ends.as_str());
    assert_eq!(promises[0]["to"], "Acme");
    assert_eq!(promises[0]["holder"], "own");
    assert_eq!(promises[0]["epic_key"], serde_json::Value::Null);
    assert_eq!(promises[0]["state"], "open");
    let (_, all) = admin_get(&h, "/api/v1/sprints").await;
    assert_eq!(all["value"]["active"]["promises"], json!([]), "{all}");
    assert_eq!(
        all["value"]["active"]["promises_after"],
        serde_json::Value::Null
    );
    // An unknown field is refused, not ignored.
    let (st, body) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/stories/{story}/promise"),
        Some(json!({"date": "2026-11-20"})),
    )
    .await;
    assert_eq!(st, 400, "{body}");
    assert_eq!(
        body,
        json!({"outcome": "error", "code": "validation",
               "message": "Failed to deserialize the JSON body into the target type: date: unknown field `date`, \
                           expected one of `on`, `shortcut`, `to` at line 1 column 7"})
    );
    // A NUL in the recipient is the person's error, not the server's.
    let (st, body) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/stories/{story}/promise"),
        Some(json!({"on": ends, "to": "Ac\u{0}me"})),
    )
    .await;
    assert_eq!(st, 400, "{body}");
    assert_eq!(
        body["message"],
        "the recipient cannot contain a NUL character"
    );

    sqlx::query(
        "UPDATE stories SET delivery = 'done', resolution = 'completed', done_at = now() WHERE key = $1",
    )
    .bind(&done)
    .execute(&h.state.pool)
    .await
    .unwrap();
    let (st, body) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/stories/{done}/promise"),
        Some(json!({"on": ends})),
    )
    .await;
    assert_eq!(st, 400, "{body}");
    assert_eq!(
        body,
        json!({"outcome": "error", "code": "validation",
               "message": format!("{done} is done: a done story cannot be promised")})
    );

    for p in [
        format!("/api/v1/stories/{story}/promise"),
        format!("/api/v1/epics/{epic}/promise"),
    ] {
        let (st, v) = call(&h.app, t, "DELETE", &p, None).await;
        assert_eq!(st, 200, "{p}: {v}");
        assert_eq!(v["value"]["promise"], serde_json::Value::Null, "{p}");
    }
    // With no promise, a request without a date or a shortcut has nothing to change, and says which item.
    let (st, body) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/stories/{story}/promise"),
        Some(json!({"to": "Acme"})),
    )
    .await;
    assert_eq!(st, 400, "{body}");
    assert_eq!(
        body,
        json!({"outcome": "error", "code": "validation",
               "message": format!("{story} has no promise to change: give a date")})
    );

    // Initiatives cannot be promised (R1): no such route.
    let (_, i) = call(
        &h.app,
        t,
        "POST",
        "/api/v1/initiatives",
        Some(json!({"idempotency_key":"pi","title":"I","horizon":"Now"})),
    )
    .await;
    let ik = i["value"]["key"].as_str().unwrap().to_string();
    let (st, v) = call(
        &h.app,
        t,
        "POST",
        &format!("/api/v1/initiatives/{ik}/promise"),
        Some(json!({"on": ends})),
    )
    .await;
    assert_eq!((st, v["code"].as_str()), (404, Some("not_found")), "{v}");
}
