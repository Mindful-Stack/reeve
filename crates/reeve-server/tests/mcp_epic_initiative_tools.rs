//! The epic and initiative write tools of `/mcp` (GRA-901), through the router, with real tokens from the sign-in
//! flow: each writes what its API route writes, answers a small receipt, records the client, and has no way to
//! the discovery mark. The scope and the write limit for every write tool are in `mcp_write_tools.rs`.
#![cfg(feature = "static-oidc")]
mod common;
use common::oauth::{Flow, connect, flow};
use common::*;
use serde_json::{Value, json};

const BOB: &str = "bob@example.com";
const DAN: &str = "dan@example.com";

/// A real access token of `scope` for the person whose API token `session` is. `f.h.admin_token` is dan's
/// session and `f.h.victim_token` is bob's.
async fn token(f: &Flow, session: &str, scope: &str) -> String {
    connect(f, session, scope).await.access_token
}

async fn epic(h: &Harness, key: &str) -> Value {
    api(h, &h.admin_token, &format!("/api/v1/epics/{key}")).await
}

async fn initiative(h: &Harness, key: &str) -> Value {
    api(h, &h.admin_token, &format!("/api/v1/initiatives/{key}")).await
}

/// An initiative made through the API as the admin; its key.
async fn an_initiative(h: &Harness, idem: &str, title: &str) -> String {
    key(&post(
        h,
        "/api/v1/initiatives",
        json!({"idempotency_key": idem, "title": title, "horizon": "Now"}),
    )
    .await)
}

/// The payload of the newest activity entry about `key`.
async fn last_payload(h: &Harness, key: &str) -> Value {
    entries(h, key).await.pop().unwrap()["payload"].clone()
}

/// `item`'s values for `names`, in order: what two items made the same way must agree on.
fn fields(item: &Value, names: &[&str]) -> Value {
    Value::Array(names.iter().map(|name| item[*name].clone()).collect())
}

/// What a repeated create is told.
fn repeat_note(key: &str, title: &str) -> String {
    format!(
        "Nothing was written: this idempotency key already made {key} (\"{title}\"). If you did not mean to retry that create, send it again with a new key."
    )
}

fn created_by_dan_through_the_flow() -> (String, String, String, Value) {
    (
        "created".to_string(),
        DAN.to_string(),
        "interactive".to_string(),
        json!("127.0.0.1"),
    )
}

const EPIC_FIELDS: [&str; 6] = [
    "title",
    "initiative",
    "brief",
    "discovery_complete",
    "external_ref",
    "version",
];

#[tokio::test]
async fn create_epic_writes_what_the_route_writes_and_answers_a_receipt() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let parent = an_initiative(h, "ce-i1", "Bookings").await;
    let arguments = |idem: &str| json!({"idempotency_key": idem, "title": "  Checkout ", "initiative": parent, "brief": "What it delivers."});
    let receipt = answer(h, &dan, "create_epic", arguments("k1")).await;
    let made = key(&receipt);
    let stored = epic(h, &made).await;
    // A receipt, and nothing more than one.
    assert_eq!(
        receipt,
        json!({"key": made, "title": "Checkout", "initiative": parent, "version": stored["version"], "created": true})
    );
    // The same create through the route stores the same epic: one action behind both. The title is trimmed by
    // the handler's own validation, and nothing marks discovery.
    let by_route = post(h, "/api/v1/epics", arguments("ce-api")).await;
    assert_eq!(
        fields(&stored, &EPIC_FIELDS),
        fields(&by_route, &EPIC_FIELDS)
    );
    assert_eq!(
        fields(&stored, &EPIC_FIELDS),
        json!(["Checkout", parent, "What it delivers.", null, null, 1])
    );
    // kb/ADR-0029 R9: the entry names the person, the channel and the client; the route's own has no client.
    assert_eq!(
        trail(h, &made).await,
        vec![created_by_dan_through_the_flow()]
    );
    assert_eq!(trail(h, &key(&by_route)).await[0].3, Value::Null);
    // Left out: no initiative and an empty brief.
    let plain = answer(
        h,
        &dan,
        "create_epic",
        json!({"idempotency_key": "k2", "title": "Loose ends"}),
    )
    .await;
    let plain_stored = epic(h, &key(&plain)).await;
    assert_eq!(
        json!([
            plain["initiative"],
            plain_stored["initiative"],
            plain_stored["brief"]
        ]),
        json!([null, null, ""])
    );
}

#[tokio::test]
async fn a_repeated_epic_create_writes_nothing_and_keys_are_each_persons_own() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let bob = token(&f, &h.victim_token, "write").await;
    let call = |title: &str| json!({"idempotency_key": "k1", "title": title});
    let first = answer(h, &dan, "create_epic", call("Checkout")).await;
    let made = key(&first);
    let count = rows(h, "epics").await;
    // The same person, the same key, from another of their connections.
    let dan_again = token(&f, &h.admin_token, "write").await;
    let repeat = answer(h, &dan_again, "create_epic", call("Something else")).await;
    assert_eq!(
        repeat,
        json!({"key": made, "title": "Checkout", "initiative": null, "version": first["version"],
               "created": false, "note": repeat_note(&made, "Checkout")})
    );
    assert_eq!(rows(h, "epics").await, count);
    assert_eq!(trail(h, &made).await.len(), 1, "one `created` entry");
    // Another person's same key is another key, stored under the caller's email.
    let bobs = answer(h, &bob, "create_epic", call("Bob's own")).await;
    assert_eq!(bobs["created"], true);
    assert_ne!(bobs["key"], first["key"]);
    let keys: Vec<String> =
        sqlx::query_scalar("SELECT idempotency_key FROM epics ORDER BY idempotency_key")
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
}

#[tokio::test]
async fn create_epic_tells_the_model_what_the_planner_refused() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    // An epic's key is no initiative's.
    let an_epic = ready_epic(h, "cr-e1").await;
    let count = rows(h, "epics").await;
    let said = |arguments: Value| refusal(h, &dan, "create_epic", arguments);
    assert_eq!(
        said(json!({"idempotency_key": "r1", "title": "Refused", "initiative": "TST-9999"})).await,
        "No initiative with key \"TST-9999\". Keys look like TST-123."
    );
    assert_eq!(
        said(json!({"idempotency_key": "r2", "title": "Refused", "initiative": an_epic})).await,
        format!("No initiative with key \"{an_epic}\". Keys look like TST-123.")
    );
    assert_eq!(
        said(json!({"idempotency_key": "r3", "title": "Refused", "initiative": "bookings"})).await,
        "initiative must be a key such as TST-12, not \"bookings\"."
    );
    assert!(
        said(json!({"idempotency_key": "r4", "title": "  "}))
            .await
            .contains("title")
    );
    assert_eq!(
        said(json!({"idempotency_key": "", "title": "Refused"})).await,
        "idempotency_key must be 1 to 100 characters, without control characters"
    );
    // What the tool withholds is refused by name, with the schema.
    for withheld in ["external_ref", "attachments", "discovery_complete", "owner"] {
        let mut call = json!({"idempotency_key": "r5", "title": "Refused"});
        call[withheld] = json!("x");
        let said = said(call).await;
        assert!(
            said.starts_with("Invalid arguments:") && said.contains(withheld),
            "{said}"
        );
    }
    assert_eq!(rows(h, "epics").await, count, "nothing was written");
}

#[tokio::test]
async fn edit_epic_changes_only_what_it_names_and_keeps_the_discovery_mark() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    // Its discovery is marked, so a mark the edit drops or rewrites would show.
    let made = ready_epic(h, "ee-e1").await;
    let (a, b) = (
        an_initiative(h, "ee-i1", "Bookings").await,
        an_initiative(h, "ee-i2", "Payments").await,
    );
    let before = epic(h, &made).await;
    assert!(
        before["discovery_complete"].is_object(),
        "the control: {before}"
    );
    let v = before["version"].as_i64().unwrap();

    let receipt = answer(
        h,
        &dan,
        "edit_epic",
        json!({"key": made, "version": v, "title": " Checkout flow "}),
    )
    .await;
    assert_eq!(
        receipt,
        json!({"key": made, "title": "Checkout flow", "initiative": null, "version": v + 1})
    );
    assert_eq!(
        fields(
            &epic(h, &made).await,
            &["title", "brief", "initiative", "discovery_complete"]
        ),
        json!([
            "Checkout flow",
            before["brief"],
            null,
            before["discovery_complete"]
        ])
    );

    let moved_in = answer(
        h,
        &dan,
        "edit_epic",
        json!({"key": made, "version": v + 1, "initiative": a, "brief": "New brief."}),
    )
    .await;
    // Decision A: the receipt says the epic moved and whose mark and decisions its stories' readiness depends on.
    assert_eq!(
        moved_in["note"],
        json!(format!(
            "Moved to {a}: its stories' readiness now also depends on {a}'s discovery mark and required decisions."
        ))
    );
    assert_eq!(
        fields(&epic(h, &made).await, &["title", "brief", "initiative"]),
        json!(["Checkout flow", "New brief.", a])
    );
    // Decision A: the move is in the entry, with the client it was made through. The whole payload is compared, so
    // a key in the wrong place shows.
    let moved = |fields: Value, from: Value, to: Value| {
        json!({"client": "127.0.0.1", "fields": fields,
               "from": {"initiative": from}, "to": {"initiative": to}})
    };
    assert_eq!(
        last_payload(h, &made).await,
        moved(json!(["brief", "initiative"]), json!(null), json!(a))
    );
    let moved_on = answer(
        h,
        &dan,
        "edit_epic",
        json!({"key": made, "version": v + 2, "initiative": b}),
    )
    .await;
    assert_eq!(
        moved_on["note"],
        json!(format!(
            "Moved from {a} to {b}: its stories' readiness now depends on {b}'s discovery mark and required decisions, and no longer on {a}'s."
        ))
    );
    assert_eq!(epic(h, &made).await["initiative"], json!(b));
    assert_eq!(
        last_payload(h, &made).await,
        moved(json!(["initiative"]), json!(a), json!(b))
    );
    let detached = answer(
        h,
        &dan,
        "edit_epic",
        json!({"key": made, "version": v + 3, "detach_initiative": true}),
    )
    .await;
    assert_eq!(detached["initiative"], Value::Null);
    assert_eq!(
        detached["note"],
        json!(format!(
            "Taken out of {b}: its stories' readiness no longer depends on {b}'s discovery mark and required decisions."
        ))
    );
    assert_eq!(
        last_payload(h, &made).await,
        moved(json!(["initiative"]), json!(b), json!(null))
    );
    let after = epic(h, &made).await;
    assert_eq!(
        fields(
            &after,
            &["initiative", "brief", "discovery_complete", "version"]
        ),
        json!([null, "New brief.", before["discovery_complete"], v + 4])
    );
    // The first edit of this test changed only the title: its entry names the field and says nothing of an
    // initiative.
    let first_edit = entries(h, &made)
        .await
        .into_iter()
        .find(|entry| entry["action"] == "edited")
        .unwrap();
    assert_eq!(
        first_edit["payload"],
        json!({"client": "127.0.0.1", "fields": ["title"]})
    );
    // kb/ADR-0029 R9.
    assert_eq!(
        trail(h, &made).await.pop().unwrap(),
        (
            "edited".to_string(),
            DAN.to_string(),
            "interactive".to_string(),
            json!("127.0.0.1")
        )
    );

    // The same edit through the route leaves the same epic: one action behind both.
    let change = json!({"title": " Renamed ", "brief": "Same brief.", "initiative": a});
    let (by_tool, by_route) = (ready_epic(h, "ee-e2").await, ready_epic(h, "ee-e3").await);
    let mut arguments = change.clone();
    arguments["key"] = json!(by_tool);
    arguments["version"] = epic(h, &by_tool).await["version"].clone();
    answer(h, &dan, "edit_epic", arguments).await;
    let mut body = change;
    body["version"] = epic(h, &by_route).await["version"].clone();
    let (status, patched) = call(
        &h.app,
        Some(&h.admin_token),
        "PATCH",
        &format!("/api/v1/epics/{by_route}"),
        Some(body),
    )
    .await;
    assert_eq!(status, 200, "{patched}");
    let same = ["title", "brief", "initiative", "external_ref", "version"];
    assert_eq!(
        fields(&epic(h, &by_tool).await, &same),
        fields(&patched["value"], &same)
    );
    assert_eq!(epic(h, &by_tool).await["title"], "Renamed");
}

#[tokio::test]
async fn edit_epic_refuses_what_would_lose_or_overwrite_and_changes_nothing() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let made = ready_epic(h, "er-e1").await;
    let before = epic(h, &made).await;
    let v = before["version"].as_i64().unwrap();
    let entries_before = trail(h, &made).await.len();
    let said = |arguments: Value| refusal(h, &dan, "edit_epic", arguments);

    assert_eq!(
        said(json!({"key": made, "version": v - 1, "title": "Stale"})).await,
        format!(
            "{made} changed since you read it and is now at version {v}. Read it again with get_epic, check what changed, and send only the change the person asked for, with version {v}."
        )
    );
    // A version ahead of the stored one is the same conflict, through the tool and through the route.
    assert_eq!(
        said(json!({"key": made, "version": v + 1, "title": "Ahead"})).await,
        format!(
            "{made} changed since you read it and is now at version {v}. Read it again with get_epic, check what changed, and send only the change the person asked for, with version {v}."
        )
    );
    let (st, ahead) = call(
        &h.app,
        Some(&h.admin_token),
        "PATCH",
        &format!("/api/v1/epics/{made}"),
        Some(json!({"version": v + 1, "title": "Ahead"})),
    )
    .await;
    assert_eq!(
        (st, &ahead["code"], &ahead["current"]),
        (409, &json!("conflict"), &json!(v)),
        "{ahead}"
    );
    assert_eq!(
        said(json!({"key": made, "version": v, "initiative": "TST-1", "detach_initiative": true}))
            .await,
        "detach_initiative and initiative cannot be sent together."
    );
    assert_eq!(
        said(json!({"key": made, "version": v})).await,
        "Nothing to change: send at least one field besides key and version."
    );
    assert_eq!(
        said(json!({"key": made, "version": v, "brief": "The start[cut: 9000 more characters]"}))
            .await,
        "brief ends with the marker of a text get_epic cut. The whole text cannot be read here, so it cannot be replaced here: tell the person to edit it in the CLI or the web client."
    );
    assert_eq!(
        said(json!({"key": "TST-9999", "version": 1, "title": "Nobody"})).await,
        "No epic with key \"TST-9999\". Keys look like TST-123."
    );
    assert_eq!(
        said(json!({"key": made, "version": v, "initiative": "TST-9999"})).await,
        format!(
            "No epic with key \"{made}\", or no initiative with key \"TST-9999\". Keys look like TST-123."
        )
    );
    assert!(
        said(json!({"key": made, "version": v, "title": " "}))
            .await
            .contains("title")
    );

    assert_eq!(epic(h, &made).await, before, "the epic is as it was");
    assert_eq!(
        trail(h, &made).await.len(),
        entries_before,
        "no entry was written"
    );
    // The control: the same call with the current version and a real change is accepted.
    answer(
        h,
        &dan,
        "edit_epic",
        json!({"key": made, "version": v, "title": "Accepted"}),
    )
    .await;
}

const INITIATIVE_FIELDS: [&str; 8] = [
    "title",
    "brief",
    "horizon",
    "target_month",
    "owner",
    "discovery_complete",
    "external_ref",
    "version",
];

#[tokio::test]
async fn create_initiative_writes_what_the_route_writes_and_answers_a_receipt() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let arguments = |idem: &str| {
        json!({"idempotency_key": idem, "title": " Bookings ", "horizon": "Next",
        "brief": "Why it exists.", "target_month": "2027-03", "owner": "Bob@Example.com"})
    };
    let receipt = answer(h, &dan, "create_initiative", arguments("k1")).await;
    let made = key(&receipt);
    let stored = initiative(h, &made).await;
    assert_eq!(
        receipt,
        json!({"key": made, "title": "Bookings", "version": stored["version"], "created": true})
    );
    // The same create through the route stores the same initiative: one action behind both. The handler trims
    // the title and normalises the owner; nothing marks discovery.
    let by_route = post(h, "/api/v1/initiatives", arguments("ci-api")).await;
    assert_eq!(
        fields(&stored, &INITIATIVE_FIELDS),
        fields(&by_route, &INITIATIVE_FIELDS)
    );
    assert_eq!(
        fields(&stored, &INITIATIVE_FIELDS),
        json!([
            "Bookings",
            "Why it exists.",
            "Next",
            "2027-03",
            BOB,
            null,
            null,
            1
        ])
    );
    assert_eq!(
        trail(h, &made).await,
        vec![created_by_dan_through_the_flow()]
    );
    // A repeat writes nothing and says which initiative the key made.
    let count = rows(h, "initiatives").await;
    let repeat = answer(
        h,
        &dan,
        "create_initiative",
        json!({"idempotency_key": "k1", "title": "Something else", "horizon": "Now"}),
    )
    .await;
    assert_eq!(
        repeat,
        json!({"key": made, "title": "Bookings", "version": stored["version"],
               "created": false, "note": repeat_note(&made, "Bookings")})
    );
    assert_eq!(rows(h, "initiatives").await, count);
    let stored_key: String =
        sqlx::query_scalar("SELECT idempotency_key FROM initiatives WHERE key = $1")
            .bind(&made)
            .fetch_one(&h.state.pool)
            .await
            .unwrap();
    assert_eq!(stored_key, "mcp:dan@example.com:k1");
}

#[tokio::test]
async fn create_initiative_tells_the_model_what_the_planner_refused() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let count = rows(h, "initiatives").await;
    let said = |arguments: Value| refusal(h, &dan, "create_initiative", arguments);
    let base = |idem: &str| json!({"idempotency_key": idem, "title": "Refused", "horizon": "Now"});
    let with = |idem: &str, name: &str, value: Value| {
        let mut call = base(idem);
        call[name] = value;
        call
    };
    assert_eq!(
        said(with("r1", "horizon", json!("Someday"))).await,
        "horizon \"Someday\" is not one of the configured buckets"
    );
    assert_eq!(
        said(with("r2", "owner", json!("eve@elsewhere.org"))).await,
        "eve@elsewhere.org is not an allowed user"
    );
    assert!(
        said(with("r3", "title", json!("  ")))
            .await
            .contains("title")
    );
    // A month as a chat model is likely to write it: refused in the action's words, which say what to send.
    assert_eq!(
        said(with("r6", "target_month", json!("March 2027"))).await,
        "target month must be YYYY-MM, for example 2027-03, not \"March 2027\""
    );
    let missing = said(json!({"idempotency_key": "r4", "title": "Refused"})).await;
    assert!(
        missing.starts_with("Invalid arguments:") && missing.contains("horizon"),
        "{missing}"
    );
    for withheld in ["external_ref", "attachments", "discovery_complete"] {
        let said = said(with("r5", withheld, json!("x"))).await;
        assert!(
            said.starts_with("Invalid arguments:") && said.contains(withheld),
            "{said}"
        );
    }
    assert_eq!(rows(h, "initiatives").await, count, "nothing was written");
}

#[tokio::test]
async fn edit_initiative_changes_only_what_it_names_and_keeps_the_discovery_mark() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let made = an_initiative(h, "ei-i1", "Bookings").await;
    // Mark its discovery, so a mark the edit drops or rewrites would show.
    post(
        h,
        &format!("/api/v1/initiatives/{made}/discovery"),
        json!({"version": 1, "complete": true}),
    )
    .await;
    let before = initiative(h, &made).await;
    assert!(
        before["discovery_complete"].is_object(),
        "the control: {before}"
    );
    let v = before["version"].as_i64().unwrap();
    let edit = |version: i64, change: Value| {
        let mut arguments = change;
        arguments["key"] = json!(made);
        arguments["version"] = json!(version);
        answer(h, &dan, "edit_initiative", arguments)
    };

    let receipt = edit(v, json!({"title": " Bookings 2027 "})).await;
    assert_eq!(
        receipt,
        json!({"key": made, "title": "Bookings 2027", "version": v + 1})
    );
    assert_eq!(
        fields(&initiative(h, &made).await, &INITIATIVE_FIELDS[1..6]),
        fields(&before, &INITIATIVE_FIELDS[1..6]),
        "nothing but the title moved"
    );
    // The first six fields as the API shows them now.
    let shown = || async { fields(&initiative(h, &made).await, &INITIATIVE_FIELDS[..6]) };
    let mark = before["discovery_complete"].clone();
    edit(
        v + 1,
        json!({"brief": "New brief.", "horizon": "Later", "target_month": "2027-06", "owner": BOB}),
    )
    .await;
    assert_eq!(
        shown().await,
        json!(["Bookings 2027", "New brief.", "Later", "2027-06", BOB, mark])
    );
    // Left out is not cleared, now that the owner and the target month hold something: a title-only edit keeps
    // both. (The title-only edit above could not show this: both were null before it.)
    edit(v + 2, json!({"title": "Bookings 2028"})).await;
    assert_eq!(
        shown().await,
        json!(["Bookings 2028", "New brief.", "Later", "2027-06", BOB, mark]),
        "a title-only edit keeps the owner and the target month"
    );
    // Each is cleared on its own, and the other survives.
    edit(v + 3, json!({"clear_target_month": true})).await;
    assert_eq!(
        shown().await,
        json!(["Bookings 2028", "New brief.", "Later", null, BOB, mark]),
        "clearing the target month keeps the owner"
    );
    edit(v + 4, json!({"target_month": "2027-09"})).await;
    edit(v + 5, json!({"clear_owner": true})).await;
    assert_eq!(
        shown().await,
        json!([
            "Bookings 2028",
            "New brief.",
            "Later",
            "2027-09",
            null,
            mark
        ]),
        "clearing the owner keeps the target month"
    );
    let after = initiative(h, &made).await;
    assert_eq!(after["version"], json!(v + 6));
    assert_eq!(
        trail(h, &made).await.pop().unwrap(),
        (
            "edited".to_string(),
            DAN.to_string(),
            "interactive".to_string(),
            json!("127.0.0.1")
        )
    );

    // The same edit through the route leaves the same initiative.
    let change = json!({"title": " Renamed ", "brief": "Same.", "horizon": "Next", "target_month": "2027-09", "owner": "Bob@Example.com"});
    let (by_tool, by_route) = (
        an_initiative(h, "ei-i2", "One").await,
        an_initiative(h, "ei-i3", "Two").await,
    );
    let mut arguments = change.clone();
    arguments["key"] = json!(by_tool);
    arguments["version"] = json!(1);
    answer(h, &dan, "edit_initiative", arguments).await;
    let mut body = change;
    body["version"] = json!(1);
    let (status, patched) = call(
        &h.app,
        Some(&h.admin_token),
        "PATCH",
        &format!("/api/v1/initiatives/{by_route}"),
        Some(body),
    )
    .await;
    assert_eq!(status, 200, "{patched}");
    assert_eq!(
        fields(&initiative(h, &by_tool).await, &INITIATIVE_FIELDS),
        fields(&patched["value"], &INITIATIVE_FIELDS)
    );
    assert_eq!(initiative(h, &by_tool).await["owner"], BOB);
}

#[tokio::test]
async fn edit_initiative_refuses_what_would_lose_or_overwrite_and_changes_nothing() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let made = an_initiative(h, "ir-i1", "Bookings").await;
    let before = initiative(h, &made).await;
    let entries_before = trail(h, &made).await.len();
    let said = |change: Value| {
        let mut arguments = change;
        if arguments.get("key").is_none() {
            arguments["key"] = json!(made);
        }
        if arguments.get("version").is_none() {
            arguments["version"] = json!(1);
        }
        refusal(h, &dan, "edit_initiative", arguments)
    };
    assert_eq!(
        said(json!({"version": 0, "title": "Stale"})).await,
        format!(
            "{made} changed since you read it and is now at version 1. Read it again with get_initiative, check what changed, and send only the change the person asked for, with version 1."
        )
    );
    // A version ahead of the stored one is the same conflict, through the tool and through the route.
    assert_eq!(
        said(json!({"version": 2, "title": "Ahead"})).await,
        format!(
            "{made} changed since you read it and is now at version 1. Read it again with get_initiative, check what changed, and send only the change the person asked for, with version 1."
        )
    );
    let (st, ahead) = call(
        &h.app,
        Some(&h.admin_token),
        "PATCH",
        &format!("/api/v1/initiatives/{made}"),
        Some(json!({"version": 2, "title": "Ahead"})),
    )
    .await;
    assert_eq!(
        (st, &ahead["code"], &ahead["current"]),
        (409, &json!("conflict"), &json!(1)),
        "{ahead}"
    );
    assert_eq!(
        said(json!({"owner": BOB, "clear_owner": true})).await,
        "clear_owner and owner cannot be sent together."
    );
    assert_eq!(
        said(json!({"target_month": "2027-03", "clear_target_month": true})).await,
        "clear_target_month and target_month cannot be sent together."
    );
    assert_eq!(
        said(json!({})).await,
        "Nothing to change: send at least one field besides key and version."
    );
    assert_eq!(
        said(json!({"brief": "The start[cut: 9000 more characters]"})).await,
        "brief ends with the marker of a text get_initiative cut. The whole text cannot be read here, so it cannot be replaced here: tell the person to edit it in the CLI or the web client."
    );
    assert_eq!(
        said(json!({"owner": "eve@elsewhere.org"})).await,
        "eve@elsewhere.org is not an allowed user"
    );
    assert_eq!(
        said(json!({"target_month": "2027-3"})).await,
        "target month must be YYYY-MM, for example 2027-03, not \"2027-3\""
    );
    assert_eq!(
        said(json!({"horizon": "Someday"})).await,
        "horizon \"Someday\" is not one of the configured buckets"
    );
    assert_eq!(
        said(json!({"key": "TST-9999", "title": "Nobody"})).await,
        "No initiative with key \"TST-9999\". Keys look like TST-123."
    );
    assert_eq!(
        initiative(h, &made).await,
        before,
        "the initiative is as it was"
    );
    assert_eq!(
        trail(h, &made).await.len(),
        entries_before,
        "no entry was written"
    );
    // The control.
    answer(
        h,
        &dan,
        "edit_initiative",
        json!({"key": made, "version": 1, "title": "Accepted"}),
    )
    .await;
}

/// The last guard never turns a committed write into an error (as `a_write_with_a_very_long_description_succeeds_
/// with_a_small_receipt` holds for a story): whatever an initiative holds, a create and an edit answer a receipt of
/// a few hundred bytes. The brief is sent through the tool. The horizon and the target month are written into the
/// row, because no request can store a long one: a horizon must be a configured bucket and a target month a month,
/// but a row may hold either from before (a bucket that was renamed away, a month from before it was checked), and
/// an edit that leaves them alone is valid.
#[tokio::test]
async fn an_initiative_write_answers_a_small_receipt_whatever_the_initiative_holds() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    // 100 000 characters of two bytes each: 200 000 bytes, nearly three times the largest answer (70 000 bytes).
    let long = "å".repeat(100_000);
    let created = answer(
        h,
        &dan,
        "create_initiative",
        json!({"idempotency_key": "big-1", "title": "Big", "horizon": "Now", "brief": long}),
    )
    .await;
    let made = key(&created);
    assert_eq!(
        created,
        json!({"key": made, "title": "Big", "version": 1, "created": true})
    );

    sqlx::query("UPDATE initiatives SET horizon = $1, target_month = $1 WHERE key = $2")
        .bind(&long)
        .bind(&made)
        .execute(&h.state.pool)
        .await
        .unwrap();
    let chars = |item: &Value, name: &str| item[name].as_str().map(|text| text.chars().count());
    let stored = initiative(h, &made).await;
    assert_eq!(
        (
            chars(&stored, "horizon"),
            chars(&stored, "target_month"),
            chars(&stored, "brief")
        ),
        (Some(100_000), Some(100_000), Some(100_000)),
        "the control: the row holds all three, each far over the largest answer"
    );

    // An edit that names neither the horizon nor the month is valid, is committed, and answers a receipt.
    let edited = answer(
        h,
        &dan,
        "edit_initiative",
        json!({"key": made, "version": 1, "title": "Still big", "brief": format!("{long}.")}),
    )
    .await;
    assert_eq!(
        edited,
        json!({"key": made, "title": "Still big", "version": 2})
    );
    assert!(
        edited.to_string().len() < 300,
        "{} bytes",
        edited.to_string().len()
    );
    let after = initiative(h, &made).await;
    assert_eq!(
        (
            chars(&after, "horizon"),
            chars(&after, "target_month"),
            chars(&after, "brief")
        ),
        (Some(100_000), Some(100_000), Some(100_001)),
        "the edit was written, and what it did not name is as it was"
    );
    // A repeated create answers the same small receipt, with its note.
    let repeat = answer(
        h,
        &dan,
        "create_initiative",
        json!({"idempotency_key": "big-1", "title": "Big", "horizon": "Now"}),
    )
    .await;
    assert_eq!(repeat["created"], false);
    assert!(
        repeat.to_string().len() < 500,
        "{} bytes",
        repeat.to_string().len()
    );
}

/// The keys of the items whose missing discovery mark holds `story` back, sorted.
async fn held_by_discovery(h: &Harness, story: &str) -> Vec<String> {
    let readiness = api(h, &h.admin_token, &format!("/api/v1/stories/{story}/ready")).await;
    let mut items: Vec<String> = readiness["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|reason| reason["reason"] == "ancestor_discovery_incomplete")
        .map(|reason| reason["item"].as_str().unwrap().to_string())
        .collect();
    items.sort();
    items
}

/// The owner's rule for GRA-901 (kb/ADR-0027: discovery marks gate; kb/ADR-0002 R3: only a mark says discovery
/// is done): nothing a chat can send to the epic and initiative tools marks discovery or reopens it, on a create
/// or an edit. Shown where it matters: a story under an epic and an initiative the tools made stays held by both,
/// whatever the tools are sent, until the marks are set through the API's own route; and once they are set, it
/// stays ready whatever the tools are sent. The observation on kb/ADR-0029 names this test.
#[tokio::test]
async fn no_epic_or_initiative_tool_can_mark_or_reopen_discovery() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let ini = key(&answer(
        h,
        &dan,
        "create_initiative",
        json!({"idempotency_key": "d-i", "title": "Bookings", "horizon": "Now"}),
    )
    .await);
    let ep = key(&answer(
        h,
        &dan,
        "create_epic",
        json!({"idempotency_key": "d-e", "title": "Checkout", "initiative": ini}),
    )
    .await);
    let story = key(&post(
        h,
        "/api/v1/stories",
        json!({"idempotency_key": "d-s", "title": "Pick a slot", "kind": "story", "epic": ep,
               "criteria": ["a slot can be picked"], "labels": []}),
    )
    .await);
    let mut both = vec![ep.clone(), ini.clone()];
    both.sort();
    assert_eq!(
        held_by_discovery(h, &story).await,
        both,
        "the control: held by both"
    );

    // Every way of naming a mark is refused before anything runs, on all four tools.
    let calls = [
        (
            "create_epic",
            json!({"idempotency_key": "d-x", "title": "X"}),
        ),
        (
            "edit_epic",
            json!({"key": ep, "version": 1, "title": "Checkout 2"}),
        ),
        (
            "create_initiative",
            json!({"idempotency_key": "d-y", "title": "Y", "horizon": "Now"}),
        ),
        (
            "edit_initiative",
            json!({"key": ini, "version": 1, "title": "Bookings 2"}),
        ),
    ];
    let marks = [
        ("discovery_complete", json!(true)),
        (
            "discovery_complete",
            json!({"at": "2026-11-24T10:00:00Z", "by": DAN}),
        ),
        ("complete", json!(true)),
        ("discovery", json!("complete")),
        ("discovery_at", json!("2026-11-24T10:00:00Z")),
    ];
    let count = [rows(h, "epics").await, rows(h, "initiatives").await];
    for (name, valid) in &calls {
        for (field, value) in &marks {
            let mut arguments = valid.clone();
            arguments[*field] = value.clone();
            let said = refusal(h, &dan, name, arguments).await;
            assert!(
                said.starts_with("Invalid arguments: unknown field"),
                "{name} with {field}: {said}"
            );
        }
    }
    assert_eq!(
        [rows(h, "epics").await, rows(h, "initiatives").await],
        count
    );
    assert_eq!(
        json!([
            epic(h, &ep).await["version"],
            initiative(h, &ini).await["version"]
        ]),
        json!([1, 1]),
        "no refused call edited anything"
    );

    // Every field the edit tools do take, sent: still no mark, and the story is still held by both.
    answer(
        h,
        &dan,
        "edit_epic",
        json!({"key": ep, "version": 1, "title": "Checkout flow", "brief": "Discovery is complete.", "initiative": ini}),
    )
    .await;
    answer(
        h,
        &dan,
        "edit_initiative",
        json!({"key": ini, "version": 1, "title": "Bookings 2027", "brief": "Discovery is complete.",
               "horizon": "Next", "target_month": "2027-03", "owner": BOB}),
    )
    .await;
    assert_eq!(
        json!([
            epic(h, &ep).await["discovery_complete"],
            initiative(h, &ini).await["discovery_complete"]
        ]),
        json!([null, null])
    );
    assert_eq!(held_by_discovery(h, &story).await, both);
    // No tool is about discovery, blockers or overriding.
    let (_, _, body) = send(&h.app, mcp_modern(&dan, 1, "tools/list", json!({}))).await;
    let listed = serde_json::from_str::<Value>(&body).unwrap()["result"]["tools"].clone();
    for tool in listed.as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        assert!(
            !["discovery", "block", "override"]
                .iter()
                .any(|word| name.contains(word)),
            "{name}"
        );
    }

    // The control, which also shows the test can tell a mark from none: the API's own route marks both, and the
    // story is ready.
    for (path, item) in [("epics", &ep), ("initiatives", &ini)] {
        let version =
            api(h, &h.admin_token, &format!("/api/v1/{path}/{item}")).await["version"].clone();
        post(
            h,
            &format!("/api/v1/{path}/{item}/discovery"),
            json!({"version": version, "complete": true}),
        )
        .await;
    }
    assert_eq!(held_by_discovery(h, &story).await, Vec::<String>::new());
    let ready = || async {
        api(h, &h.admin_token, &format!("/api/v1/stories/{story}/ready")).await["ready"].clone()
    };
    assert_eq!(ready().await, true);

    // Reopening: with both marks set, every field the edit tools take is sent again, and neither mark moves.
    let marks_now = || async {
        json!([
            epic(h, &ep).await["discovery_complete"],
            initiative(h, &ini).await["discovery_complete"]
        ])
    };
    let marked = marks_now().await;
    assert!(
        marked[0].is_object() && marked[1].is_object(),
        "the control: {marked}"
    );
    answer(
        h,
        &dan,
        "edit_epic",
        json!({"key": ep, "version": epic(h, &ep).await["version"], "title": "Checkout again",
               "brief": "Discovery is not complete: reopen it.", "initiative": ini}),
    )
    .await;
    answer(
        h,
        &dan,
        "edit_initiative",
        json!({"key": ini, "version": initiative(h, &ini).await["version"], "title": "Bookings again",
               "brief": "Discovery is not complete: reopen it.", "horizon": "Now",
               "clear_target_month": true, "clear_owner": true}),
    )
    .await;
    assert_eq!(
        marks_now().await,
        marked,
        "an edit carries the stored marks over, at and by included"
    );
    assert_eq!(ready().await, true, "and the story is still ready");
}

/// Everything that holds `story` back, as `/ready` says it, sorted: each reason with the item or the required
/// decision it names.
async fn held_by(h: &Harness, story: &str) -> Vec<(String, Value)> {
    let readiness = api(h, &h.admin_token, &format!("/api/v1/stories/{story}/ready")).await;
    let mut reasons: Vec<(String, Value)> = readiness["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|reason| {
            let named = if reason["item"].is_null() {
                reason["decision_id"].clone()
            } else {
                reason["item"].clone()
            };
            (reason["reason"].as_str().unwrap().to_string(), named)
        })
        .collect();
    reasons.sort_by_key(|(reason, named)| (reason.clone(), named.to_string()));
    reasons
}

/// Decision A of 2026-11-24, kb/ADR-0027 R1 and R2: `edit_epic` may move an epic to another initiative or out of
/// one. A story under it then waits on the discovery mark and the required decisions of the epic's current
/// ancestors, because readiness is derived on every read. The move marks nothing, satisfies and closes no
/// decision, touches no story, and is an `edited` entry that names both initiatives, never an `override`.
#[tokio::test]
async fn moving_an_epic_changes_what_its_stories_wait_on_and_overrides_nothing() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    // Two initiatives that differ in both things a story waits on. `unmarked`: no discovery mark, one open
    // required decision. `marked`: its discovery complete, another open required decision.
    let unmarked = an_initiative(h, "mv-i1", "Bookings").await;
    let marked = an_initiative(h, "mv-i2", "Payments").await;
    post(
        h,
        &format!("/api/v1/initiatives/{marked}/discovery"),
        json!({"version": 1, "complete": true}),
    )
    .await;
    let mut decisions = Vec::new();
    for (idem, on) in [("mv-d1", &unmarked), ("mv-d2", &marked)] {
        let decision = post(
            h,
            "/api/v1/decisions",
            json!({"idempotency_key": idem, "on": on, "title": format!("Decide for {on}"), "decider": DAN}),
        )
        .await;
        decisions.push(decision["id"].clone());
    }
    let (on_unmarked, on_marked) = (decisions[0].clone(), decisions[1].clone());
    // An epic whose own discovery is complete, under no initiative, and a story in it with a criterion.
    let ep = ready_epic(h, "mv-e").await;
    let story = key(&post(
        h,
        "/api/v1/stories",
        json!({"idempotency_key": "mv-s", "title": "Pick a slot", "kind": "story", "epic": ep,
               "criteria": ["a slot can be picked"], "labels": []}),
    )
    .await);
    assert_eq!(
        held_by(h, &story).await,
        vec![],
        "the control: ready before any move"
    );

    // What a move must leave as it is: the three marks, both decisions whole, and the story itself.
    let untouched = || async {
        json!({
            "marks": [
                epic(h, &ep).await["discovery_complete"],
                initiative(h, &unmarked).await["discovery_complete"],
                initiative(h, &marked).await["discovery_complete"],
            ],
            "decisions": [
                api(h, &h.admin_token, &format!("/api/v1/decisions/{on_unmarked}")).await,
                api(h, &h.admin_token, &format!("/api/v1/decisions/{on_marked}")).await,
            ],
            "story": api(h, &h.admin_token, &format!("/api/v1/stories/{story}")).await,
        })
    };
    let before = untouched().await;
    let story_entries = trail(h, &story).await;
    assert!(
        before["marks"][0].is_object()
            && before["marks"][1].is_null()
            && before["marks"][2].is_object(),
        "the control: the epic and one initiative are marked, the other is not: {before}"
    );
    let reason = |name: &str, named: &Value| (name.to_string(), named.clone());
    let move_epic = |change: Value| async {
        let mut arguments = change;
        arguments["key"] = json!(ep);
        arguments["version"] = epic(h, &ep).await["version"].clone();
        answer(h, &dan, "edit_epic", arguments).await
    };

    // Under the unmarked initiative: held by its missing mark and by its decision.
    move_epic(json!({"initiative": unmarked})).await;
    assert_eq!(
        held_by(h, &story).await,
        vec![
            reason("ancestor_discovery_incomplete", &json!(unmarked)),
            reason("decision_no_record", &on_unmarked),
        ]
    );
    assert_eq!(untouched().await, before);
    // Under the marked one: the first initiative's mark and decision no longer hold it; the second's decision does.
    move_epic(json!({"initiative": marked})).await;
    assert_eq!(
        held_by(h, &story).await,
        vec![reason("decision_no_record", &on_marked)]
    );
    assert_eq!(untouched().await, before);
    // Out of any initiative: ready again, as before the first move.
    move_epic(json!({"detach_initiative": true})).await;
    assert_eq!(held_by(h, &story).await, vec![]);
    assert_eq!(
        api(h, &h.admin_token, &format!("/api/v1/stories/{story}/ready")).await["ready"],
        true
    );
    assert_eq!(
        untouched().await,
        before,
        "no mark, no decision and nothing of the story changed"
    );

    // The log: three edits of the epic, each naming where it went, with the client; and no override anywhere.
    let moves: Vec<Value> = entries(h, &ep)
        .await
        .iter()
        .filter(|entry| entry["action"] == "edited")
        .map(|entry| {
            json!([
                entry["payload"]["from"]["initiative"],
                entry["payload"]["to"]["initiative"],
                entry["payload"]["client"]
            ])
        })
        .collect();
    assert_eq!(
        moves,
        vec![
            json!([null, unmarked, "127.0.0.1"]),
            json!([unmarked, marked, "127.0.0.1"]),
            json!([marked, null, "127.0.0.1"]),
        ]
    );
    let overrides: i64 =
        sqlx::query_scalar("SELECT count(*) FROM activity WHERE action = 'override'")
            .fetch_one(&h.state.pool)
            .await
            .unwrap();
    assert_eq!(overrides, 0, "a move is an edit, never an override");
    assert_eq!(
        trail(h, &story).await,
        story_entries,
        "nothing was logged on the story"
    );
}
