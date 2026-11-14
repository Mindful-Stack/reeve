//! The write tools of `/mcp` (GRA-661), through the router, with real tokens from the sign-in flow: each tool
//! writes what its API route writes, answers a small receipt, records the client, needs the `write` scope, counts
//! against the write limit, and can never override the readiness gate.
#![cfg(feature = "static-oidc")]
mod common;
use common::oauth::{Flow, connect, flow};
use common::*;
use serde_json::{Value, json};

const BOB: &str = "bob@example.com";
const DAN: &str = "dan@example.com";

/// A real access token of `scope` ("read" or "write") for the person whose API token `session` is, through the
/// sign-in flow's own routes. `f.h.admin_token` is dan's session and `f.h.victim_token` is bob's.
async fn token(f: &Flow, session: &str, scope: &str) -> String {
    connect(f, session, scope).await.access_token
}

/// How many rows `table` has.
async fn rows(h: &Harness, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(&h.state.pool)
        .await
        .unwrap()
}

/// The reserved prefix, at the API: the four routes that store a caller's idempotency key in `stories` or
/// `comments` refuse a key in the MCP callers' space, and write nothing. The control is the same request with a
/// key outside it.
#[tokio::test]
async fn the_api_refuses_an_idempotency_key_in_the_mcp_callers_space() {
    let f = flow().await;
    let h = &f.h;
    let epic = ready_epic(h, "rk-e1").await;
    let story = key(&post(
        h,
        "/api/v1/stories",
        json!({"idempotency_key": "rk-s1", "title": "Booking", "kind": "story", "epic": epic, "criteria": [], "labels": []}),
    )
    .await);
    let decision = post(
        h,
        "/api/v1/decisions",
        json!({"idempotency_key": "rk-d1", "on": epic, "title": "Which queue", "decider": DAN}),
    )
    .await["id"]
        .as_i64()
        .unwrap();
    let reserved = "mcp:bob@example.com:k1";
    let story_body = |idem: &str| json!({"idempotency_key": idem, "title": "Planted", "kind": "story", "criteria": [], "labels": []});
    let comment_body = |idem: &str| json!({"body": "planted", "idempotency_key": idem});
    let routes = [
        (
            "/api/v1/stories".to_string(),
            story_body(reserved),
            story_body("mcpx:k1"),
        ),
        (
            format!("/api/v1/items/{story}/comments"),
            comment_body(reserved),
            comment_body("mcpx:k2"),
        ),
        (
            format!("/api/v1/stories/{story}/comments"),
            comment_body(reserved),
            comment_body("mcpx:k3"),
        ),
        (
            format!("/api/v1/decisions/{decision}/comments"),
            comment_body(reserved),
            comment_body("mcpx:k4"),
        ),
    ];
    for (path, refused, control) in routes {
        let before = (rows(h, "stories").await, rows(h, "comments").await);
        let (status, body) = call(&h.app, Some(&h.admin_token), "POST", &path, Some(refused)).await;
        assert_eq!(
            (status, body["code"].as_str(), body["message"].as_str()),
            (
                400,
                Some("validation"),
                Some("an idempotency key must not start with mcp: (reserved for MCP clients)")
            ),
            "{path}"
        );
        assert_eq!(
            (rows(h, "stories").await, rows(h, "comments").await),
            before,
            "{path}: nothing written"
        );
        let (status, body) = call(&h.app, Some(&h.admin_token), "POST", &path, Some(control)).await;
        assert_eq!(status, 200, "the control on {path}: {body}");
    }
}

/// The activity entries about `key`, oldest first, as the feed's route answers them.
async fn entries(h: &Harness, key: &str) -> Vec<Value> {
    let mut entries = api(
        h,
        &h.admin_token,
        &format!("/api/v1/activity?entity_key={key}"),
    )
    .await
    .as_array()
    .unwrap()
    .clone();
    entries.sort_by_key(|entry| entry["id"].as_i64());
    entries
}

/// `(action, actor, via, client)` of each entry about `key`.
async fn trail(h: &Harness, key: &str) -> Vec<(String, String, String, Value)> {
    entries(h, key)
        .await
        .iter()
        .map(|e| {
            let text = |name: &str| e[name].as_str().unwrap().to_string();
            (
                text("action"),
                text("actor"),
                text("via"),
                e["payload"]["client"].clone(),
            )
        })
        .collect()
}

/// A story as the API shows it.
async fn story(h: &Harness, key: &str) -> Value {
    api(h, &h.admin_token, &format!("/api/v1/stories/{key}")).await
}

fn names(tools: &Value, read_only: bool) -> Vec<String> {
    tools
        .as_array()
        .unwrap()
        .iter()
        .filter(|tool| tool["annotations"]["readOnlyHint"] == read_only)
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect()
}

/// `tools/list` over 2026-07-28 with `token`: the tools and the answer's `cacheScope`.
async fn listed(h: &Harness, token: &str) -> (Value, Value) {
    let (status, _, body) = send(&h.app, mcp_modern(token, 1, "tools/list", json!({}))).await;
    assert_eq!(status, 200, "{body}");
    let result = serde_json::from_str::<Value>(&body).unwrap()["result"].clone();
    (result["tools"].clone(), result["cacheScope"].clone())
}

/// What a repeated create is told.
fn repeat_note(key: &str, title: &str) -> String {
    format!(
        "Nothing was written: this idempotency key already made {key} (\"{title}\"). If you did not mean to retry that create, send it again with a new key."
    )
}

#[tokio::test]
async fn create_story_writes_what_the_route_writes_and_answers_a_receipt() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let epic = ready_epic(h, "cs-e1").await;
    let receipt = answer(
        h,
        &dan,
        "create_story",
        json!({"idempotency_key": "k1", "title": "  Pick a slot ", "kind": "bug", "epic": epic,
               "description": "As a customer.", "criteria": ["a slot can be picked", "it is held"],
               "labels": ["web"], "owner": BOB}),
    )
    .await;
    let made = receipt["key"].as_str().unwrap().to_string();
    let stored = story(h, &made).await;
    // The receipt is the stored story's own values, and nothing more than a receipt.
    assert_eq!(
        receipt,
        json!({"key": made, "title": "Pick a slot", "delivery": "todo", "resolution": null,
               "version": stored["version"], "created": true})
    );
    // Every field the tool takes arrived, through the route's own validation (the title is trimmed).
    assert_eq!(
        json!([
            stored["title"],
            stored["kind"],
            stored["epic"],
            stored["description"],
            stored["criteria"],
            stored["labels"],
            stored["owner"],
            stored["external_ref"]
        ]),
        json!(["Pick a slot", "bug", epic, "As a customer.",
               [{"text": "a slot can be picked", "done": false}, {"text": "it is held", "done": false}],
               ["web"], BOB, null])
    );
    // kb/ADR-0029 R9: the entry names the person, the channel and the client. The client is the host of the
    // flow's client id, which the server took from the token's connection.
    assert!(
        f.client_id.starts_with("http://127.0.0.1:"),
        "the flow's client: {}",
        f.client_id
    );
    assert_eq!(
        trail(h, &made).await,
        vec![(
            "created".to_string(),
            DAN.to_string(),
            "interactive".to_string(),
            json!("127.0.0.1")
        )]
    );
    // The control: the same create through the API has no client. And a token from the developer command names
    // that command.
    let by_api = key(&post(
        h,
        "/api/v1/stories",
        json!({"idempotency_key": "cs-api", "title": "By hand", "kind": "story", "criteria": [], "labels": []}),
    )
    .await);
    assert_eq!(trail(h, &by_api).await[0].3, Value::Null);
    let developer = issue_mcp_token(h, DAN).await.secret;
    let by_command = answer(
        h,
        &developer,
        "create_story",
        json!({"idempotency_key": "k2", "title": "By command"}),
    )
    .await;
    assert_eq!(
        trail(h, by_command["key"].as_str().unwrap()).await[0].3,
        json!("urn:reeve:developer-command")
    );
    // Left out: kind is a story, and there is no epic, description, criterion, label or owner.
    let plain = story(h, by_command["key"].as_str().unwrap()).await;
    assert_eq!(
        json!([
            plain["kind"],
            plain["epic"],
            plain["description"],
            plain["criteria"],
            plain["labels"],
            plain["owner"]
        ]),
        json!(["story", null, "", [], [], null])
    );
}

#[tokio::test]
async fn a_repeated_create_writes_nothing_and_says_so_and_keys_are_each_persons_own() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let bob = token(&f, &h.victim_token, "write").await;
    let call = |title: &str| json!({"idempotency_key": "k1", "title": title});
    let first = answer(h, &dan, "create_story", call("Pick a slot")).await;
    let made = first["key"].as_str().unwrap().to_string();
    let count = rows(h, "stories").await;

    // The same person, the same key, from another of their connections: nothing is written, and the answer says
    // which story the key made, whatever the second call asked for.
    let dan_again = token(&f, &h.admin_token, "write").await;
    let repeat = answer(h, &dan_again, "create_story", call("Something else")).await;
    assert_eq!(
        repeat,
        json!({"key": made, "title": "Pick a slot", "delivery": "todo", "resolution": null,
               "version": first["version"], "created": false,
               "note": repeat_note(&made, "Pick a slot")})
    );
    assert_eq!(rows(h, "stories").await, count);
    assert_eq!(trail(h, &made).await.len(), 1, "one `created` entry");

    // Another person's same key is another key.
    let bobs = answer(h, &bob, "create_story", call("Bob's own")).await;
    assert_eq!(bobs["created"], true);
    assert_ne!(bobs["key"], first["key"]);
    // How it is stored: under the caller's email, which the call cannot choose.
    let keys: Vec<String> =
        sqlx::query_scalar("SELECT idempotency_key FROM stories ORDER BY idempotency_key")
            .fetch_all(&h.state.pool)
            .await
            .unwrap();
    assert_eq!(
        keys,
        vec![
            "mcp:bob@example.com:k1".to_string(),
            "mcp:dan@example.com:k1".to_string()
        ]
    );

    // A key that is empty, too long or has a control character is refused, and nothing is written.
    let count = rows(h, "stories").await;
    for bad in [String::new(), "k".repeat(101), "k\n1".to_string()] {
        let said = refusal(
            h,
            &dan,
            "create_story",
            json!({"idempotency_key": bad, "title": "Refused"}),
        )
        .await;
        assert_eq!(
            said,
            "idempotency_key must be 1 to 100 characters, without control characters"
        );
    }
    // The control: a key of exactly 100 characters is accepted.
    let longest = answer(
        h,
        &dan,
        "create_story",
        json!({"idempotency_key": "k".repeat(100), "title": "Accepted"}),
    )
    .await;
    assert_eq!(longest["created"], true);
    assert_eq!(rows(h, "stories").await, count + 1);
}

/// One key sent at once over two connections of one person: one story, and exactly one of the two calls is told
/// it created it. `created` is the insert's own answer inside the handler's transaction, so this holds in every
/// interleaving; nothing here forces a particular one.
#[tokio::test]
async fn one_key_sent_at_once_over_two_connections_of_one_person_makes_one_story() {
    let f = flow().await;
    let h = &f.h;
    let (one, two) = (
        token(&f, &h.admin_token, "write").await,
        token(&f, &h.admin_token, "write").await,
    );
    let count = rows(h, "stories").await;
    let call = || json!({"idempotency_key": "both", "title": "Sent twice"});
    let (first, second) = tokio::join!(
        answer(h, &one, "create_story", call()),
        answer(h, &two, "create_story", call())
    );
    assert_eq!(first["key"], second["key"], "one story");
    let created = [&first, &second]
        .iter()
        .filter(|receipt| receipt["created"] == true)
        .count();
    assert_eq!(created, 1, "exactly one call created it: {first} {second}");
    assert_eq!(rows(h, "stories").await, count + 1);
    assert_eq!(
        trail(h, first["key"].as_str().unwrap()).await.len(),
        1,
        "one `created` entry"
    );
}

/// The guard never turns a committed write into an error: a description far over the largest answer is written
/// whole, and the answer is a receipt.
#[tokio::test]
async fn a_write_with_a_very_long_description_succeeds_with_a_small_receipt() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    // 100 000 characters of two bytes each: 200 000 bytes, nearly three times the largest answer (70 000 bytes).
    let long = "å".repeat(100_000);
    let receipt = answer(
        h,
        &dan,
        "create_story",
        json!({"idempotency_key": "k1", "title": "Long", "description": long}),
    )
    .await;
    assert!(
        receipt.to_string().len() < 300,
        "a receipt, not the story: {} bytes",
        receipt.to_string().len()
    );
    let made = receipt["key"].as_str().unwrap();
    assert_eq!(
        story(h, made).await["description"]
            .as_str()
            .map(|d| d.chars().count()),
        Some(100_000)
    );
    // The control: reading the same story back over MCP is where the text is cut.
    let read = answer(h, &dan, "get_story", json!({"key": made})).await;
    assert!(
        read["story"]["description"]
            .as_str()
            .unwrap()
            .ends_with("[cut: 90000 more characters]")
    );
}

#[tokio::test]
async fn create_story_tells_the_model_what_the_planner_refused() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let count = rows(h, "stories").await;
    let said = |arguments: Value| refusal(h, &dan, "create_story", arguments);
    assert_eq!(
        said(json!({"idempotency_key": "r1", "title": "Refused", "epic": "TST-9999"})).await,
        "No epic with key \"TST-9999\". Keys look like TST-123."
    );
    assert_eq!(
        said(json!({"idempotency_key": "r2", "title": "Refused", "epic": "bookings"})).await,
        "epic must be a key such as TST-12, not \"bookings\"."
    );
    assert_eq!(
        said(json!({"idempotency_key": "r3", "title": "Refused", "owner": "eve@elsewhere.org"}))
            .await,
        "eve@elsewhere.org is not an allowed user"
    );
    assert!(
        said(json!({"idempotency_key": "r4", "title": "  "}))
            .await
            .contains("title")
    );
    // What the tool withholds is refused by name, with the schema.
    for withheld in ["external_ref", "attachments", "rank"] {
        let mut call = json!({"idempotency_key": "r5", "title": "Refused"});
        call[withheld] = json!("x");
        let said = said(call).await;
        assert!(
            said.starts_with("Invalid arguments:") && said.contains(withheld),
            "{said}"
        );
    }
    assert_eq!(rows(h, "stories").await, count, "nothing was written");
}

/// kb/ADR-0029 R7, through the router, for every tool that changes anything: a read connection is not shown it,
/// and calling it is 403 with the challenge, in both revisions, with nothing written.
#[tokio::test]
async fn a_read_connection_sees_no_write_tool_and_is_refused_each_with_insufficient_scope() {
    let f = flow().await;
    let h = &f.h;
    let reader = token(&f, &h.admin_token, "read").await;
    let writer = token(&f, &h.admin_token, "write").await;
    let (all, scope) = listed(h, &writer).await;
    let (shown, reader_scope) = listed(h, &reader).await;
    assert_eq!(
        (&scope, &reader_scope),
        (&json!("private"), &json!("private"))
    );
    let writes = names(&all, false);
    assert!(
        writes.contains(&"create_story".to_string()),
        "the control: {writes:?}"
    );
    assert_eq!(names(&shown, true), names(&all, true), "every read tool");
    assert_eq!(names(&shown, false), Vec::<String>::new(), "no write tool");
    // The same list over 2025-11-25.
    let (_, _, body) = send(
        &h.app,
        mcp_legacy(&reader, Some(2), "tools/list", json!({})),
    )
    .await;
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["result"]["tools"],
        shown
    );

    let count = rows(h, "stories").await;
    let challenge = "Bearer error=\"insufficient_scope\", scope=\"write\", resource_metadata=\"http://127.0.0.1:7474/.well-known/oauth-protected-resource/mcp\", error_description=\"This tool changes the planner and needs a connection that may read and change\"";
    for name in &writes {
        // Arguments `create_story` would accept: without the check a story would be made.
        let params = json!({"name": name, "arguments": {"idempotency_key": "scope-1", "title": "Not allowed"}});
        for request in [
            mcp_modern(&reader, 7, "tools/call", params.clone()),
            mcp_legacy(&reader, Some(7), "tools/call", params.clone()),
        ] {
            let (status, headers, body) = send(&h.app, request).await;
            assert_eq!(status, 403, "{name}: {body}");
            assert_eq!(headers["www-authenticate"], challenge, "{name}");
            assert_eq!(
                serde_json::from_str::<Value>(&body).unwrap(),
                json!({"jsonrpc": "2.0", "id": 7, "error": {"code": -32003, "message": "Insufficient scope: this tool needs the write scope"}}),
                "{name}"
            );
        }
    }
    assert_eq!(rows(h, "stories").await, count, "nothing was written");
    // The control: the same call with the write connection creates the story, and a read tool is the reader's.
    assert_eq!(
        answer(
            h,
            &writer,
            "create_story",
            json!({"idempotency_key": "scope-1", "title": "Allowed"})
        )
        .await["created"],
        true
    );
    answer(h, &reader, "list_stories", json!({})).await;
}

/// The limit through the router: per person across their connections, refused calls counted, the clock moved on.
#[tokio::test]
async fn the_thirty_first_write_call_in_ten_minutes_is_refused_with_a_time() {
    let f = flow().await;
    let h = &f.h;
    let (one, two) = (
        token(&f, &h.admin_token, "write").await,
        token(&f, &h.admin_token, "write").await,
    );
    let bob = token(&f, &h.victim_token, "write").await;
    // Thirty calls that are each refused for their arguments, over two connections of one person.
    for n in 0..30 {
        let said = refusal(
            h,
            if n % 2 == 0 { &one } else { &two },
            "create_story",
            json!({}),
        )
        .await;
        assert!(said.starts_with("Invalid arguments:"), "call {n}: {said}");
    }
    let good = |idem: &str| json!({"idempotency_key": idem, "title": "Over the limit"});
    let count = rows(h, "stories").await;
    assert_eq!(
        refusal(h, &one, "create_story", good("l1")).await,
        format!(
            "Too many changes: one person may make at most 30 calls to tools that change the planner in 10 minutes. Nothing was written. Try again in 600 seconds (at {} UTC).",
            (h.state.clock.now() + chrono::Duration::minutes(10)).format("%H:%M:%S")
        )
    );
    assert_eq!(rows(h, "stories").await, count, "nothing was written");
    // Reading is not limited, and another person is not this person.
    answer(h, &one, "list_stories", json!({})).await;
    assert_eq!(
        answer(h, &bob, "create_story", good("l1")).await["created"],
        true
    );
    // One second before the ten minutes are over, and then at them.
    h.state.clock.advance(chrono::Duration::seconds(599));
    assert!(
        refusal(h, &two, "create_story", good("l1"))
            .await
            .contains("Try again in 1 seconds")
    );
    h.state.clock.advance(chrono::Duration::seconds(1));
    assert_eq!(
        answer(h, &two, "create_story", good("l1")).await["created"],
        true
    );
}
