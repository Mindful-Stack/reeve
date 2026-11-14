//! The read tools of `/mcp` (GRA-660), through the router: each answers what its API route answers for the same
//! input and the same person, taken through the tool's documented summary, cap or cut, and each adapter does what
//! the CLI does on the client. Every comparison with a route is preceded by a control on the route's own answer.
mod common;
use common::*;
use serde_json::{Value, json};

/// `tools/call` over 2026-07-28 as the token's person: the JSON-RPC `result`. The HTTP status must be 200.
async fn tool(h: &Harness, token: &str, name: &str, arguments: Value) -> Value {
    let params = json!({"name": name, "arguments": arguments});
    let (status, _, body) = send(&h.app, mcp_modern(token, 1, "tools/call", params)).await;
    assert_eq!(status, 200, "{name}: {body}");
    serde_json::from_str::<Value>(&body).unwrap()["result"].clone()
}

/// A successful call's `structuredContent`. The text block must carry the same JSON.
async fn answer(h: &Harness, token: &str, name: &str, arguments: Value) -> Value {
    let result = tool(h, token, name, arguments).await;
    assert_eq!(result["isError"], false, "{name}: {result}");
    let text = result["content"][0]["text"].as_str().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(text).unwrap(),
        result["structuredContent"],
        "{name}: the text block is the same JSON"
    );
    result["structuredContent"].clone()
}

/// A failed call's message: a result with `isError: true` and no `structuredContent`.
async fn refusal(h: &Harness, token: &str, name: &str, arguments: Value) -> String {
    let result = tool(h, token, name, arguments).await;
    assert_eq!(result["isError"], true, "{name}: {result}");
    assert!(
        result.get("structuredContent").is_none(),
        "{name}: {result}"
    );
    result["content"][0]["text"].as_str().unwrap().to_string()
}

/// The `value` of a `GET` on the API as the token's person.
async fn api(h: &Harness, token: &str, path: &str) -> Value {
    let (status, body) = call(&h.app, Some(token), "GET", path, None).await;
    assert_eq!(status, 200, "{path}: {body}");
    body["value"].clone()
}

/// The `value` of a `POST` on the API as the admin.
async fn post(h: &Harness, path: &str, body: Value) -> Value {
    let (status, answer) = call(&h.app, Some(&h.admin_token), "POST", path, Some(body)).await;
    assert_eq!(status, 200, "{path}: {answer}");
    answer["value"].clone()
}

fn key(item: &Value) -> String {
    item["key"].as_str().unwrap().to_string()
}

async fn new_story(h: &Harness, body: Value) -> String {
    key(&post(h, "/api/v1/stories", body).await)
}

const BOB: &str = "bob@example.com";
const DAN: &str = "dan@example.com";

/// An MCP token for `email`. The harness's API tokens are `h.admin_token` (dan) and `h.victim_token` (bob).
async fn mcp_token(h: &Harness, email: &str) -> String {
    issue_mcp_token(h, email).await.secret
}

/// What `seed` made.
#[allow(dead_code)] // not every test names every item
struct Seed {
    /// Discovery not complete, so the stories under it are not ready.
    initiative: String,
    /// Under `initiative`; discovery complete.
    epic: String,
    /// Under no initiative; discovery complete.
    loose_epic: String,
    /// In `epic`, owned by bob, todo, labelled `web`, promised in 30 days.
    bobs: String,
    /// In `epic`, owned by dan, a bug, todo, urgent.
    dans: String,
    /// No epic, no owner, a chore.
    untriaged: String,
    /// In `loose_epic`, no owner: the one ready story.
    ready: String,
    /// In `loose_epic`, owned by bob, done (dropped).
    dropped: String,
    /// On `epic`, decider bob, open.
    open_decision: i64,
    /// On `bobs`, decider dan, closed.
    closed_decision: i64,
}

/// The fixture the list and read tests share, made through the API as the admin. Its seven properties are listed
/// in the plan's Task 5; each test states the control it relies on.
async fn seed(h: &Harness) -> Seed {
    let initiative = key(&post(
        h,
        "/api/v1/initiatives",
        json!({"idempotency_key": "rt-i1", "title": "Bookings", "horizon": "Now", "brief": "Why bookings matter."}),
    )
    .await);
    let made = post(
        h,
        "/api/v1/epics",
        json!({"idempotency_key": "rt-e1", "title": "Booking flow", "initiative": initiative, "brief": "Pick, pay, confirm."}),
    )
    .await;
    let epic = key(&made);
    post(
        h,
        &format!("/api/v1/epics/{epic}/discovery"),
        json!({"version": made["version"], "complete": true}),
    )
    .await;
    let loose_epic = ready_epic(h, "rt-e2").await;
    let bobs = new_story(
        h,
        json!({"idempotency_key": "rt-s1", "title": "Pick a slot", "kind": "story", "epic": epic, "criteria": ["a slot can be picked"], "labels": ["web"], "owner": BOB}),
    )
    .await;
    let dans = new_story(
        h,
        json!({"idempotency_key": "rt-s2", "title": "Confirm by email", "kind": "bug", "epic": epic, "criteria": ["an email arrives"], "labels": [], "owner": DAN}),
    )
    .await;
    let untriaged = new_story(
        h,
        json!({"idempotency_key": "rt-s3", "title": "Tidy the footer", "kind": "chore", "criteria": [], "labels": []}),
    )
    .await;
    let ready = new_story(
        h,
        json!({"idempotency_key": "rt-s4", "title": "Print the receipt", "kind": "story", "epic": loose_epic, "criteria": ["a receipt prints"], "labels": []}),
    )
    .await;
    let dropped = new_story(
        h,
        json!({"idempotency_key": "rt-s5", "title": "Fax the confirmation", "kind": "story", "epic": loose_epic, "criteria": ["a fax arrives"], "labels": [], "owner": BOB}),
    )
    .await;
    post(
        h,
        &format!("/api/v1/stories/{dropped}/move"),
        json!({"version": 1, "to": "done", "resolution": "dropped"}),
    )
    .await;
    post(
        h,
        &format!("/api/v1/stories/{dans}/urgent"),
        json!({"reason": "a customer is waiting"}),
    )
    .await;
    let in_a_month = chrono::Utc::now().date_naive() + chrono::Days::new(30);
    post(
        h,
        &format!("/api/v1/stories/{bobs}/promise"),
        json!({"on": in_a_month}),
    )
    .await;
    let open_decision = post(
        h,
        "/api/v1/decisions",
        json!({"idempotency_key": "rt-d1", "on": epic, "title": "Which payment provider", "decider": BOB}),
    )
    .await["id"]
        .as_i64()
        .unwrap();
    let closed_decision = post(
        h,
        "/api/v1/decisions",
        json!({"idempotency_key": "rt-d2", "on": bobs, "title": "How long a booking is", "decider": DAN}),
    )
    .await["id"]
        .as_i64()
        .unwrap();
    post(
        h,
        &format!("/api/v1/decisions/{closed_decision}/close"),
        json!({"kind": "decided", "reason": "Thirty minutes."}),
    )
    .await;
    Seed {
        initiative,
        epic,
        loose_epic,
        bobs,
        dans,
        untriaged,
        ready,
        dropped,
        open_decision,
        closed_decision,
    }
}

#[tokio::test]
async fn search_items_answers_the_route_s_hits() {
    let h = mcp_harness().await;
    let s = seed(&h).await;
    let bob = mcp_token(&h, BOB).await;

    // The control: one item has "slot" in its title. (A decision is not an item, and criteria are not searched.)
    let hits = api(&h, &h.victim_token, "/api/v1/search?q=slot").await;
    assert_eq!(hits.as_array().unwrap().len(), 1, "{hits}");
    assert_eq!(hits[0]["key"], json!(s.bobs));
    assert_eq!(
        answer(&h, &bob, "search_items", json!({"text": "slot"})).await,
        json!({"hits": hits, "omitted": 0})
    );

    // A key finds its item, marked exact.
    let by_key = api(&h, &h.victim_token, &format!("/api/v1/search?q={}", s.epic)).await;
    assert_eq!(by_key[0]["exact"], true, "the control: {by_key}");
    assert_eq!(
        answer(&h, &bob, "search_items", json!({"text": s.epic})).await,
        json!({"hits": by_key, "omitted": 0})
    );

    // Nothing found is an empty list, not an error.
    assert_eq!(
        answer(&h, &bob, "search_items", json!({"text": "zebra"})).await,
        json!({"hits": [], "omitted": 0})
    );
    // The route's validation runs for the tool, and its message is what the model reads.
    assert_eq!(
        refusal(&h, &bob, "search_items", json!({"text": "   "})).await,
        "search text must be 1..=200 characters"
    );
    // The route's parameter name is not a tool argument: an unknown argument is refused, not ignored.
    let unknown = refusal(&h, &bob, "search_items", json!({"q": "slot"})).await;
    assert!(unknown.contains("unknown field `q`"), "{unknown}");
}

#[tokio::test]
async fn get_epic_answers_the_route_s_epic_with_a_long_brief_cut() {
    let h = mcp_harness().await;
    let s = seed(&h).await;
    let bob = mcp_token(&h, BOB).await;

    let want = api(&h, &h.victim_token, &format!("/api/v1/epics/{}", s.epic)).await;
    // The control: the epic has what the answer is supposed to carry.
    assert_eq!(want["brief"], "Pick, pay, confirm.");
    assert_eq!(want["initiative"], json!(s.initiative));
    assert!(want["discovery_complete"].is_object());
    assert_eq!(
        answer(&h, &bob, "get_epic", json!({"key": s.epic})).await,
        want
    );

    // A brief over the limit: the API answers it whole, the tool cuts it and says by how much. Two-byte
    // characters: a cut that counted bytes would stop at half of them.
    let brief = "å".repeat(10_003);
    let long = key(&post(
        &h,
        "/api/v1/epics",
        json!({"idempotency_key": "rt-e3", "title": "Long", "brief": brief}),
    )
    .await);
    let mut want = api(&h, &h.victim_token, &format!("/api/v1/epics/{long}")).await;
    assert_eq!(
        want["brief"],
        json!(brief),
        "the control: the API route answers it whole"
    );
    want["brief"] = json!(format!("{}[cut: 3 more characters]", "å".repeat(10_000)));
    assert_eq!(
        answer(&h, &bob, "get_epic", json!({"key": long})).await,
        want
    );

    // An unknown key, and a key that is a story's, are told in a tool caller's words.
    assert_eq!(
        refusal(&h, &bob, "get_epic", json!({"key": "TST-999999"})).await,
        "No epic with key TST-999999. Keys look like TST-123."
    );
    assert_eq!(
        refusal(&h, &bob, "get_epic", json!({"key": s.bobs})).await,
        format!("No epic with key {}. Keys look like TST-123.", s.bobs)
    );
}

#[tokio::test]
async fn get_initiative_answers_the_route_s_initiative_with_a_long_brief_cut() {
    let h = mcp_harness().await;
    let s = seed(&h).await;
    let bob = mcp_token(&h, BOB).await;

    let want = api(
        &h,
        &h.victim_token,
        &format!("/api/v1/initiatives/{}", s.initiative),
    )
    .await;
    // The control: the initiative has a brief and a progress to carry (two stories count under it, neither done).
    assert_eq!(want["brief"], "Why bookings matter.");
    assert_eq!(want["progress"], 0.0, "{want}");
    assert_eq!(
        answer(&h, &bob, "get_initiative", json!({"key": s.initiative})).await,
        want
    );

    let brief = "é".repeat(10_002);
    let long = key(&post(
        &h,
        "/api/v1/initiatives",
        json!({"idempotency_key": "rt-i2", "title": "Long", "horizon": "Now", "brief": brief}),
    )
    .await);
    let mut want = api(&h, &h.victim_token, &format!("/api/v1/initiatives/{long}")).await;
    assert_eq!(
        want["brief"],
        json!(brief),
        "the control: the API route answers it whole"
    );
    want["brief"] = json!(format!("{}[cut: 2 more characters]", "é".repeat(10_000)));
    assert_eq!(
        answer(&h, &bob, "get_initiative", json!({"key": long})).await,
        want
    );

    assert_eq!(
        refusal(&h, &bob, "get_initiative", json!({"key": "TST-999999"})).await,
        "No initiative with key TST-999999. Keys look like TST-123."
    );
}

/// An epic as `list_epics` summarises it, written out from the API's JSON by hand, not through
/// `EpicSummary::from`: mapping both sides through the conversion would compare it with itself.
fn epic_summary(epic: &Value) -> Value {
    json!({
        "key": epic["key"],
        "title": epic["title"],
        "initiative": epic["initiative"],
        "version": epic["version"],
        "discovery_complete": epic["discovery_complete"].is_object(),
        "promised_on": epic["promise"]["fact"]["on"],
    })
}

#[tokio::test]
async fn list_epics_answers_the_route_s_epics_as_summaries() {
    let h = mcp_harness().await;
    let s = seed(&h).await;
    let bob = mcp_token(&h, BOB).await;
    // A third epic whose discovery is not complete, and a promise on the first: the summary's two derived fields
    // then differ between rows.
    let undiscovered = key(&post(
        &h,
        "/api/v1/epics",
        json!({"idempotency_key": "rt-e4", "title": "Not looked into"}),
    )
    .await);
    let in_a_week = chrono::Utc::now().date_naive() + chrono::Days::new(7);
    post(
        &h,
        &format!("/api/v1/epics/{}/promise", s.epic),
        json!({"on": in_a_week}),
    )
    .await;

    let all = api(&h, &h.victim_token, "/api/v1/epics").await;
    let keys: Vec<String> = all.as_array().unwrap().iter().map(key).collect();
    assert_eq!(
        keys,
        vec![s.epic.clone(), s.loose_epic.clone(), undiscovered],
        "the control: three epics, oldest first"
    );
    let summaries: Vec<Value> = all.as_array().unwrap().iter().map(epic_summary).collect();
    let got = answer(&h, &bob, "list_epics", json!({})).await;
    assert_eq!(got, json!({"epics": summaries, "omitted": 0}));
    // The two derived fields differ between rows.
    assert_eq!(got["epics"][0]["discovery_complete"], true);
    assert_eq!(got["epics"][0]["promised_on"], json!(in_a_week));
    assert_eq!(got["epics"][2]["discovery_complete"], false);
    assert!(got["epics"][2]["promised_on"].is_null());

    let of_initiative = api(
        &h,
        &h.victim_token,
        &format!("/api/v1/epics?initiative={}", s.initiative),
    )
    .await;
    assert_eq!(of_initiative.as_array().unwrap().len(), 1, "the control");
    assert_eq!(
        answer(&h, &bob, "list_epics", json!({"initiative": s.initiative})).await,
        json!({"epics": [epic_summary(&of_initiative[0])], "omitted": 0})
    );
    // A key that names nothing lists nothing, as for the route; text that is no key is said to be none.
    assert_eq!(
        answer(&h, &bob, "list_epics", json!({"initiative": "TST-999999"})).await,
        json!({"epics": [], "omitted": 0})
    );
    assert_eq!(
        refusal(&h, &bob, "list_epics", json!({"initiative": "bookings"})).await,
        "initiative must be a key such as TST-12, not \"bookings\"."
    );
}

/// An initiative as `list_initiatives` summarises it, written out by hand from the API's JSON.
fn initiative_summary(initiative: &Value) -> Value {
    json!({
        "key": initiative["key"],
        "title": initiative["title"],
        "horizon": initiative["horizon"],
        "target_month": initiative["target_month"],
        "owner": initiative["owner"],
        "version": initiative["version"],
        "discovery_complete": initiative["discovery_complete"].is_object(),
        "progress": initiative["progress"],
    })
}

#[tokio::test]
async fn list_initiatives_answers_the_route_s_initiatives_as_summaries() {
    let h = mcp_harness().await;
    let s = seed(&h).await;
    let bob = mcp_token(&h, BOB).await;
    // A second initiative, with an owner, a target month and its discovery complete, so rows differ.
    let made = post(
        &h,
        "/api/v1/initiatives",
        json!({"idempotency_key": "rt-i3", "title": "Payments", "horizon": "Now", "target_month": "2027-01", "owner": DAN}),
    )
    .await;
    let payments = key(&made);
    post(
        &h,
        &format!("/api/v1/initiatives/{payments}/discovery"),
        json!({"version": made["version"], "complete": true}),
    )
    .await;

    let all = api(&h, &h.victim_token, "/api/v1/initiatives").await;
    let keys: Vec<String> = all.as_array().unwrap().iter().map(key).collect();
    assert_eq!(
        keys,
        vec![s.initiative.clone(), payments.clone()],
        "the control"
    );
    let summaries: Vec<Value> = all
        .as_array()
        .unwrap()
        .iter()
        .map(initiative_summary)
        .collect();
    let got = answer(&h, &bob, "list_initiatives", json!({})).await;
    assert_eq!(got, json!({"initiatives": summaries, "omitted": 0}));
    // The derived field differs between rows.
    assert_eq!(got["initiatives"][1]["discovery_complete"], true);
    assert_eq!(got["initiatives"][0]["discovery_complete"], false);

    // It takes no arguments: one is refused, not ignored.
    let unknown = refusal(&h, &bob, "list_initiatives", json!({"horizon": "Now"})).await;
    assert!(unknown.contains("unknown field `horizon`"), "{unknown}");
}
