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
/// interleaving. A smoke test: it does not force the race, and a run may well take the two calls one after the
/// other.
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

/// An owner nobody is allowed as, of 200 000 characters: the planner's refusal repeats it.
fn oversized_owner() -> String {
    format!("{}@elsewhere.org", "e".repeat(200_000))
}

/// A refusal of `oversized_owner` as the model reads it: bounded, with the cut marker, still saying what was
/// refused.
fn assert_bounded(said: &str) {
    assert!(
        said.chars().count() < 10_000,
        "{} characters",
        said.chars().count()
    );
    assert!(said.contains("[cut: "), "{}", &said[..200]);
    assert!(
        said.ends_with("is not an allowed user"),
        "{}",
        &said[said.len() - 200..]
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
    assert_bounded(
        &said(json!({"idempotency_key": "r6", "title": "Refused", "owner": oversized_owner()}))
            .await,
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
            "Too many changes: one person may make at most 30 calls to tools that change the planner in 10 minutes. Nothing was written. Try again in 600 seconds (at {} UTC). Tell the person; do not call a tool that changes the planner again before then.",
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

/// One quota for all four write tools, checked after the scope: 31 write calls over a read connection are each
/// 403 and count for nothing, a mix of the four tools then fills the person's thirty, at which each of the four is
/// refused while the read tools still answer. A limit checked ahead of the scope, or for some tools only, goes red.
#[tokio::test]
async fn every_write_tool_counts_against_one_quota_and_only_after_the_scope_check() {
    let f = flow().await;
    let h = &f.h;
    let reader = token(&f, &h.admin_token, "read").await;
    let writer = token(&f, &h.admin_token, "write").await;
    let made = key(&post(
        h,
        "/api/v1/stories",
        json!({"idempotency_key": "q-s1", "title": "Pick a slot", "kind": "story", "criteria": [], "labels": []}),
    )
    .await);
    // Arguments each tool would accept and act on, for call `n`.
    let valid = |name: &str, n: usize| match name {
        "create_story" => {
            json!({"idempotency_key": format!("q-{n}"), "title": format!("Story {n}")})
        }
        "edit_story" => json!({"key": made, "version": 1, "title": format!("Edit {n}")}),
        "add_comment" => {
            json!({"idempotency_key": format!("q-{n}"), "item": made, "body": format!("Comment {n}")})
        }
        "move_story" => json!({"key": made, "to": "todo", "version": 1}),
        other => panic!("{other}"),
    };
    const WRITE: [&str; 4] = ["create_story", "edit_story", "add_comment", "move_story"];
    let over = "Too many changes:";

    for n in 0..31 {
        let name = WRITE[n % 4];
        let params = json!({"name": name, "arguments": valid(name, n)});
        let (status, _, body) = send(&h.app, mcp_modern(&reader, 1, "tools/call", params)).await;
        assert_eq!(status, 403, "read call {n} ({name}): {body}");
    }
    // The person's whole thirty, a mix of the four tools; whatever each answers, none is the limit's refusal.
    for n in 0..30 {
        let name = WRITE[n % 4];
        let result = tool(h, &writer, name, valid(name, n)).await;
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(!text.starts_with(over), "write call {n} ({name}): {text}");
    }
    let count = (rows(h, "stories").await, rows(h, "comments").await);
    for name in WRITE {
        let said = refusal(h, &writer, name, valid(name, 99)).await;
        assert!(said.starts_with(over), "{name}: {said}");
    }
    assert_eq!(
        (rows(h, "stories").await, rows(h, "comments").await),
        count,
        "nothing was written at exhaustion"
    );
    // Reading is not limited.
    answer(h, &writer, "list_stories", json!({})).await;
    answer(h, &reader, "get_story", json!({"key": made})).await;
}

/// A story in a ready epic with two criteria, two labels and an owner, made through the API; its key.
async fn editable(h: &Harness, idem: &str) -> String {
    let epic = ready_epic(h, &format!("{idem}-e")).await;
    key(&post(
        h,
        "/api/v1/stories",
        json!({"idempotency_key": idem, "title": "Pick a slot", "kind": "story", "epic": epic, "description": "As a customer.", "criteria": ["a slot can be picked", "it is held"], "labels": ["web", "old"], "owner": BOB}),
    )
    .await)
}

/// `PATCH` on the API as the admin; the new story.
async fn patch(h: &Harness, key: &str, body: Value) -> Value {
    let (status, answer) = call(
        &h.app,
        Some(&h.admin_token),
        "PATCH",
        &format!("/api/v1/stories/{key}"),
        Some(body),
    )
    .await;
    assert_eq!(status, 200, "{answer}");
    answer["value"].clone()
}

#[tokio::test]
async fn edit_story_changes_only_what_it_names_and_merges_criteria_and_labels() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let key = editable(h, "es-s1").await;
    // Both criteria are checked off, through the API.
    let before = patch(
        h,
        &key,
        json!({"version": 1, "criteria": [{"text": "a slot can be picked", "done": true}, {"text": "it is held", "done": true}]}),
    )
    .await;
    assert_eq!(before["version"], 2, "the control");

    let receipt = answer(
        h,
        &dan,
        "edit_story",
        json!({"key": key, "version": 2, "title": "Pick a time slot", "kind": "bug",
               "criteria": ["a slot can be picked", "it is held for ten minutes", "it can be released"],
               "add_labels": ["api", "web"], "remove_labels": ["old"]}),
    )
    .await;
    let after = story(h, &key).await;
    assert_eq!(
        receipt,
        json!({"key": key, "title": "Pick a time slot", "delivery": "todo", "resolution": null, "version": 3})
    );
    assert_eq!(after["version"], 3);
    assert_eq!(
        json!([
            after["title"],
            after["kind"],
            after["criteria"],
            after["labels"]
        ]),
        json!(["Pick a time slot", "bug",
               [{"text": "a slot can be picked", "done": true}, {"text": "it is held for ten minutes", "done": false},
                {"text": "it can be released", "done": false}],
               ["web", "api"]])
    );
    // What the call did not name is as it was.
    assert_eq!(
        json!([
            after["description"],
            after["owner"],
            after["epic"],
            after["rank"]
        ]),
        json!([
            before["description"],
            before["owner"],
            before["epic"],
            before["rank"]
        ])
    );
    assert_eq!(
        trail(h, &key).await.last().cloned(),
        Some((
            "edited".to_string(),
            DAN.to_string(),
            "interactive".to_string(),
            json!("127.0.0.1")
        ))
    );

    // Clearing is by flag; setting is by value.
    let cleared = answer(
        h,
        &dan,
        "edit_story",
        json!({"key": key, "version": 3, "detach_epic": true, "clear_owner": true}),
    )
    .await;
    assert_eq!(cleared["version"], 4);
    let after = story(h, &key).await;
    assert_eq!(json!([after["epic"], after["owner"]]), json!([null, null]));
    let set = answer(
        h,
        &dan,
        "edit_story",
        json!({"key": key, "version": 4, "epic": before["epic"], "owner": DAN, "description": "Rewritten."}),
    )
    .await;
    assert_eq!(set["version"], 5);
    let after = story(h, &key).await;
    assert_eq!(
        json!([after["epic"], after["owner"], after["description"]]),
        json!([before["epic"], DAN, "Rewritten."])
    );
}

#[tokio::test]
async fn edit_story_refuses_what_would_lose_or_overwrite_and_changes_nothing() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let key = editable(h, "er-s1").await;
    let before = story(h, &key).await;
    let said = |mut arguments: Value| {
        arguments["key"] = json!(key);
        arguments["version"] = json!(1);
        refusal(h, &dan, "edit_story", arguments)
    };
    let cases = [
        (
            json!({"detach_epic": true, "epic": before["epic"]}),
            "detach_epic and epic cannot be sent together.",
        ),
        (
            json!({"clear_owner": true, "owner": DAN}),
            "clear_owner and owner cannot be sent together.",
        ),
        (
            json!({"criteria": []}),
            "criteria cannot be emptied here: send every criterion the story should have.",
        ),
        (
            json!({}),
            "Nothing to change: send at least one field besides key and version.",
        ),
        (
            json!({"owner": "eve@elsewhere.org"}),
            "eve@elsewhere.org is not an allowed user",
        ),
        (
            json!({"description": "As a cust[cut: 9990 more characters]"}),
            "description ends with the marker of a text get_story cut. The whole text cannot be read here, so it cannot be replaced here: tell the person to edit it in the CLI or the web client.",
        ),
        (
            json!({"criteria": ["a slot can be picked", "it is he[cut: 5 more characters]"]}),
            "A criterion ends with the marker of a text get_story cut. The whole text cannot be read here, so it cannot be replaced here: tell the person to edit it in the CLI or the web client.",
        ),
    ];
    for (arguments, expected) in cases {
        assert_eq!(said(arguments.clone()).await, expected, "{arguments}");
    }
    assert_bounded(&said(json!({"owner": oversized_owner()})).await);
    assert!(
        said(json!({"criteria": ["a slot can be picked", "  "]}))
            .await
            .contains("criterion")
    );
    for withheld in ["rank", "external_ref", "attachments", "brief", "labels"] {
        let mut arguments = json!({"title": "Refused"});
        arguments[withheld] = json!("x");
        let said = said(arguments).await;
        assert!(
            said.starts_with("Invalid arguments:") && said.contains(withheld),
            "{said}"
        );
    }
    // A version that is no longer current, with and without a merge to read for: the same conflict, and what the
    // model is told names the version to read.
    patch(
        h,
        &key,
        json!({"version": 1, "labels": ["web", "old", "new"]}),
    )
    .await;
    let stale = "changed since you read it and is now at version 2. Read it again with get_story, check what changed, and send only the change the person asked for, with version 2.";
    assert_eq!(
        said(json!({"title": "Stale"})).await,
        format!("{key} {stale}")
    );
    assert_eq!(
        said(json!({"add_labels": ["x"]})).await,
        format!("{key} {stale}")
    );
    assert_eq!(
        refusal(
            h,
            &dan,
            "edit_story",
            json!({"key": "TST-9999", "version": 1, "title": "Nobody"})
        )
        .await,
        "No story with key \"TST-9999\". Keys look like TST-123."
    );
    // Nothing above changed the story: it is as the API's own edit left it.
    let after = story(h, &key).await;
    assert_eq!(after["version"], 2);
    assert_eq!(after["title"], before["title"]);
    assert_eq!(after["labels"], json!(["web", "old", "new"]));
    assert_eq!(
        trail(h, &key)
            .await
            .iter()
            .filter(|entry| entry.0 == "edited")
            .count(),
        1,
        "the API's edit only"
    );
}

/// The comments of an item, as the API lists them.
async fn thread(h: &Harness, path: &str) -> Vec<Value> {
    api(h, &h.admin_token, path)
        .await
        .as_array()
        .unwrap()
        .clone()
}

#[tokio::test]
async fn add_comment_comments_on_a_story_an_epic_and_a_decision_and_answers_a_receipt() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let story_key = editable(h, "ac-s1").await;
    let epic = story(h, &story_key).await["epic"]
        .as_str()
        .unwrap()
        .to_string();
    let decision = post(
        h,
        "/api/v1/decisions",
        json!({"idempotency_key": "ac-d1", "on": epic, "title": "Which queue", "decider": DAN}),
    )
    .await["id"]
        .as_i64()
        .unwrap();

    let targets = [
        (
            json!({"item": story_key}),
            format!("/api/v1/items/{story_key}/comments"),
            "story",
            story_key.clone(),
        ),
        (
            json!({"item": epic}),
            format!("/api/v1/items/{epic}/comments"),
            "epic",
            epic.clone(),
        ),
        (
            json!({"decision": decision}),
            format!("/api/v1/decisions/{decision}/comments"),
            "decision",
            decision.to_string(),
        ),
    ];
    for (n, (target, path, entity, entity_key)) in targets.into_iter().enumerate() {
        assert_eq!(
            thread(h, &path).await.len(),
            0,
            "the control: {path} starts empty"
        );
        let mut arguments = target.clone();
        arguments["idempotency_key"] = json!(format!("c{n}"));
        arguments["body"] = json!("  The queue is chosen. ");
        let receipt = answer(h, &dan, "add_comment", arguments.clone()).await;
        let comments = thread(h, &path).await;
        assert_eq!(comments.len(), 1, "{path}");
        // The comment is the caller's, trimmed as the route trims it, and the receipt names it.
        assert_eq!(
            json!([comments[0]["author"], comments[0]["body"]]),
            json!([DAN, "The queue is chosen."])
        );
        let mut expected = target.clone();
        expected["comment_id"] = comments[0]["id"].clone();
        expected["created"] = json!(true);
        assert_eq!(receipt, expected, "{path}");
        // The entry is on the thing commented on and names the client.
        let on_target = entries(h, &entity_key).await;
        let entry = on_target
            .iter()
            .rfind(|e| e["action"] == "commented")
            .unwrap_or_else(|| panic!("no entry on {entity_key}"));
        assert_eq!(
            json!([
                entry["entity_type"],
                entry["actor"],
                entry["via"],
                entry["payload"]["client"],
                entry["payload"]["comment_id"]
            ]),
            json!([entity, DAN, "interactive", "127.0.0.1", comments[0]["id"]])
        );
        // A repeat adds nothing and says which comment the key made. It does not say what that comment is on:
        // the key may have made it on another item.
        arguments["body"] = json!("Said again.");
        let repeat = answer(h, &dan, "add_comment", arguments).await;
        assert_eq!(
            repeat,
            json!({"comment_id": comments[0]["id"], "created": false,
                   "note": format!("Nothing was written: this idempotency key already made comment {}, which may be on another item. For another comment, use a new key.", comments[0]["id"])}),
            "{path}"
        );
        assert_eq!(thread(h, &path).await.len(), 1, "{path}: still one");
    }
    // The case the note is worded for: the story's key reused on the epic answers the story's comment, adds
    // nothing to the epic, and does not claim the comment is on the epic.
    let on_story = thread(h, &format!("/api/v1/items/{story_key}/comments")).await;
    let reused = answer(
        h,
        &dan,
        "add_comment",
        json!({"idempotency_key": "c0", "item": epic, "body": "For the epic."}),
    )
    .await;
    assert_eq!(
        json!([reused["comment_id"], reused["created"], reused.get("item")]),
        json!([on_story[0]["id"], false, null])
    );
    assert_eq!(
        thread(h, &format!("/api/v1/items/{epic}/comments"))
            .await
            .len(),
        1
    );
}

#[tokio::test]
async fn add_comment_needs_exactly_one_target_and_a_body_and_a_long_comment_gets_a_small_receipt() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let story_key = editable(h, "an-s1").await;
    let count = rows(h, "comments").await;
    let said = |arguments: Value| refusal(h, &dan, "add_comment", arguments);
    let one = "Send exactly one of item and decision.";
    assert_eq!(
        said(json!({"idempotency_key": "n1", "body": "x"})).await,
        one
    );
    assert_eq!(
        said(json!({"idempotency_key": "n2", "body": "x", "item": story_key, "decision": 1})).await,
        one
    );
    assert_eq!(
        said(json!({"idempotency_key": "n3", "body": "  ", "item": story_key})).await,
        "empty comment"
    );
    assert_eq!(
        said(json!({"idempotency_key": "n4", "body": "x", "item": "TST-9999"})).await,
        "No item with key \"TST-9999\". Keys look like TST-123."
    );
    assert_eq!(
        said(json!({"idempotency_key": "n5", "body": "x", "decision": 987654})).await,
        "No required decision with id 987654."
    );
    assert_eq!(
        said(json!({"idempotency_key": "", "body": "x", "item": story_key})).await,
        "idempotency_key must be 1 to 100 characters, without control characters"
    );
    let withheld =
        said(json!({"idempotency_key": "n6", "body": "x", "item": story_key, "attachments": []}))
            .await;
    assert!(
        withheld.starts_with("Invalid arguments:") && withheld.contains("attachments"),
        "{withheld}"
    );
    assert_eq!(rows(h, "comments").await, count, "nothing was written");

    // 100 000 characters of two bytes: written whole, answered with a receipt.
    let receipt = answer(
        h,
        &dan,
        "add_comment",
        json!({"idempotency_key": "n7", "item": story_key, "body": "å".repeat(100_000)}),
    )
    .await;
    assert!(receipt.to_string().len() < 200, "{receipt}");
    let stored = thread(h, &format!("/api/v1/items/{story_key}/comments")).await;
    assert_eq!(
        stored[0]["body"].as_str().map(|body| body.chars().count()),
        Some(100_000)
    );
}

/// How many activity entries have `action`, in the whole log.
async fn logged(h: &Harness, action: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM activity WHERE action = $1")
        .bind(action)
        .fetch_one(&h.state.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn move_story_moves_a_ready_story_as_the_route_does() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let epic = ready_epic(h, "mv-e1").await;
    let ready = key(&post(
        h,
        "/api/v1/stories",
        json!({"idempotency_key": "mv-s1", "title": "Print the receipt", "kind": "story", "epic": epic, "criteria": ["a receipt prints"], "labels": []}),
    )
    .await);
    assert_eq!(
        api(h, &h.admin_token, &format!("/api/v1/stories/{ready}/ready")).await["ready"],
        true,
        "the control"
    );

    let receipt = answer(
        h,
        &dan,
        "move_story",
        json!({"key": ready, "to": "in_progress", "version": 1}),
    )
    .await;
    let after = story(h, &ready).await;
    assert_eq!(
        receipt,
        json!({"key": ready, "title": "Print the receipt", "delivery": "in_progress", "resolution": null, "version": after["version"]})
    );
    // The route's rule, not the tool's: a story with no owner becomes the mover's (kb/ADR-0027 R6).
    assert_eq!(
        json!([after["delivery"], after["owner"]]),
        json!(["in_progress", DAN])
    );
    assert_eq!(
        trail(h, &ready).await.last().cloned(),
        Some((
            "moved".to_string(),
            DAN.to_string(),
            "interactive".to_string(),
            json!("127.0.0.1")
        ))
    );
    // Done needs a resolution: without one nothing moves, and nothing here is the gate's to override.
    let said = refusal(
        h,
        &dan,
        "move_story",
        json!({"key": ready, "to": "done", "version": after["version"]}),
    )
    .await;
    assert_eq!(
        said,
        format!(
            "{ready} was not moved. Reasons, as get_story lists them: [{{\"reason\":\"resolution_required\"}}]. Nothing changed."
        )
    );
    let done = answer(
        h,
        &dan,
        "move_story",
        json!({"key": ready, "to": "done", "version": after["version"], "resolution": "completed"}),
    )
    .await;
    assert_eq!(
        json!([done["delivery"], done["resolution"]]),
        json!(["done", "completed"])
    );
    // A stale version is told with the version to read.
    assert_eq!(
        refusal(
            h,
            &dan,
            "move_story",
            json!({"key": ready, "to": "in_review", "version": 1})
        )
        .await,
        format!(
            "{ready} changed since you read it and is now at version {v}. Read it again with get_story, check what changed, and send only the change the person asked for, with version {v}.",
            v = done["version"]
        )
    );
    assert_eq!(logged(h, "override").await, 0);
}

/// kb/ADR-0029 R10, kb/ADR-0027 R2: nothing a chat can send moves a story that is not ready into active work.
#[tokio::test]
async fn move_story_never_overrides_the_gate() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    // No epic: untriaged, so not ready.
    let blocked = key(&post(
        h,
        "/api/v1/stories",
        json!({"idempotency_key": "ov-s1", "title": "Tidy the footer", "kind": "chore", "criteria": ["tidy"], "labels": []}),
    )
    .await);
    let before = story(h, &blocked).await;
    assert_eq!(
        api(
            h,
            &h.admin_token,
            &format!("/api/v1/stories/{blocked}/ready")
        )
        .await["ready"],
        false,
        "the control"
    );
    let entries_before = trail(h, &blocked).await.len();

    // The plain move is refused with the planner's reasons and where a person can override.
    let said = refusal(
        h,
        &dan,
        "move_story",
        json!({"key": blocked, "to": "in_progress", "version": 1}),
    )
    .await;
    assert_eq!(
        said,
        format!(
            "{blocked} was not moved: it is not ready. Reasons, as get_story lists them: [{{\"reason\":\"untriaged\"}}]. A person can override this in the CLI or the web client; this tool cannot. Nothing changed."
        )
    );
    // Every way of sending a reason is refused before anything runs.
    for sent in [
        json!({"override_reason": "the person said so"}),
        json!({"override": true}),
        json!({"reason": "the person said so"}),
    ] {
        let mut arguments = json!({"key": blocked, "to": "in_progress", "version": 1});
        for (name, value) in sent.as_object().unwrap() {
            arguments[name] = value.clone();
        }
        let said = refusal(h, &dan, "move_story", arguments).await;
        assert!(
            said.starts_with("Invalid arguments: unknown field"),
            "{said}"
        );
    }
    // In 2025-11-25 too.
    let params = json!({"name": "move_story", "arguments": {"key": blocked, "to": "in_progress", "version": 1, "override_reason": "x"}});
    let (status, _, body) = send(&h.app, mcp_legacy(&dan, Some(3), "tools/call", params)).await;
    let result = serde_json::from_str::<Value>(&body).unwrap()["result"].clone();
    assert_eq!((status, &result["isError"]), (200, &json!(true)), "{body}");

    let after = story(h, &blocked).await;
    assert_eq!(
        json!([after["delivery"], after["version"], after["owner"]]),
        json!([before["delivery"], before["version"], before["owner"]])
    );
    assert_eq!(
        trail(h, &blocked).await.len(),
        entries_before,
        "no entry was written"
    );
    assert_eq!(
        (logged(h, "override").await, logged(h, "moved").await),
        (0, 0)
    );

    // The control, and the reason the rule is the tool's to keep: the same person's API token, with a reason, does
    // override. An MCP caller is interactive too; only the missing field (and the action's own refusal of a
    // reason from a caller with a client) keeps it out.
    let (status, moved) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        &format!("/api/v1/stories/{blocked}/move"),
        Some(json!({"version": 1, "to": "in_progress", "override_reason": "agreed in standup"})),
    )
    .await;
    assert_eq!(
        (status, &moved["value"]["delivery"]),
        (200, &json!("in_progress")),
        "{moved}"
    );
    assert_eq!(logged(h, "override").await, 1);
}

/// kb/ADR-0014 R4: moving a story never depends on goals. With a sprint open and a goal that another story
/// serves, a ready story that serves no goal moves exactly as it does with no sprint.
#[tokio::test]
async fn move_story_does_not_depend_on_goals() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let epic = ready_epic(h, "mg-e1").await;
    let new = |idem: &str| json!({"idempotency_key": idem, "title": idem, "kind": "story", "epic": epic, "criteria": ["works"], "labels": []});
    let serving = key(&post(h, "/api/v1/stories", new("mg-s1")).await);
    let upkeep = key(&post(h, "/api/v1/stories", new("mg-s2")).await);
    let goal = post(
        h,
        "/api/v1/goals",
        json!({"idempotency_key": "mg-g1", "outcome": "Customers can book", "check": "one did"}),
    )
    .await["goal"]["id"]
        .as_i64()
        .unwrap();
    post(
        h,
        &format!("/api/v1/goals/{goal}/links"),
        json!({"items": [serving]}),
    )
    .await;
    let draft = api(h, &h.admin_token, "/api/v1/sprints").await["draft"].clone();
    post(
        h,
        &format!("/api/v1/sprints/{}/open", draft["id"]),
        json!({"version": draft["version"]}),
    )
    .await;
    // The control: the sprint is active and the two stories differ in the goals they serve.
    assert_eq!(
        story(h, &serving).await["goals"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        story(h, &upkeep).await["goals"].as_array().map(Vec::len),
        Some(0)
    );

    for key in [&serving, &upkeep] {
        let receipt = answer(
            h,
            &dan,
            "move_story",
            json!({"key": key, "to": "in_progress", "version": 1}),
        )
        .await;
        assert_eq!(receipt["delivery"], "in_progress", "{key}");
    }
}
