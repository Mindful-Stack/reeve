//! The read tools of `/mcp` (GRA-660), through the router: each answers what its API route answers for the same
//! input and the same person, taken through the tool's documented summary, cap or cut, and each adapter does what
//! the CLI does on the client. Every comparison with a route is preceded by a control on the route's own answer.
mod common;
use common::*;
use serde_json::{Value, json};

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
        "No epic with key \"TST-999999\". Keys look like TST-123."
    );
    assert_eq!(
        refusal(&h, &bob, "get_epic", json!({"key": s.bobs})).await,
        format!("No epic with key {:?}. Keys look like TST-123.", s.bobs)
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
        "No initiative with key \"TST-999999\". Keys look like TST-123."
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

fn ids(decisions: &Value) -> Vec<i64> {
    decisions
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_i64().unwrap())
        .collect()
}

#[tokio::test]
async fn list_decisions_answers_the_route_s_decisions_and_mine_keeps_the_caller_s() {
    let h = mcp_harness().await;
    let s = seed(&h).await;
    let bob = mcp_token(&h, BOB).await;
    let dan = mcp_token(&h, DAN).await;

    // The tool's arguments, the route's query for them, and the control: the ids the route must list. Unscoped,
    // the closed decision is left out; scoped, it is listed: as `reeve decision ls` lists them.
    let cases = [
        (json!({}), String::new(), vec![s.open_decision]),
        (
            json!({"on": s.bobs}),
            format!("?on={}", s.bobs),
            vec![s.closed_decision],
        ),
        (
            json!({"initiative": s.initiative}),
            format!("?initiative={}", s.initiative),
            vec![s.open_decision, s.closed_decision],
        ),
    ];
    for (arguments, query, control) in cases {
        let rows = api(&h, &h.victim_token, &format!("/api/v1/decisions{query}")).await;
        let mut listed = ids(&rows);
        listed.sort();
        assert_eq!(listed, control, "the control for {arguments}");
        assert_eq!(
            answer(&h, &bob, "list_decisions", arguments.clone()).await,
            json!({"decisions": rows, "omitted": 0}),
            "{arguments}"
        );
    }

    // `mine` keeps the decisions the caller decides: bob the open one, dan none of the open ones, and dan's
    // closed one once the list is scoped so that closed ones are in it.
    let mine = json!({"mine": true});
    let bobs = answer(&h, &bob, "list_decisions", mine.clone()).await;
    assert_eq!(ids(&bobs["decisions"]), vec![s.open_decision]);
    let dans = answer(&h, &dan, "list_decisions", mine).await;
    assert_eq!(dans, json!({"decisions": [], "omitted": 0}));
    let dans_under = answer(
        &h,
        &dan,
        "list_decisions",
        json!({"mine": true, "initiative": s.initiative}),
    )
    .await;
    assert_eq!(ids(&dans_under["decisions"]), vec![s.closed_decision]);

    // Both scopes at once are refused by the action, in the route's words.
    assert_eq!(
        refusal(
            &h,
            &bob,
            "list_decisions",
            json!({"on": s.bobs, "initiative": s.initiative})
        )
        .await,
        "give on or initiative, not both"
    );
    // A key that names nothing lists nothing; text that is no key is said to be none; the queue is not offered.
    assert_eq!(
        answer(&h, &bob, "list_decisions", json!({"on": "TST-999999"})).await,
        json!({"decisions": [], "omitted": 0})
    );
    assert_eq!(
        refusal(&h, &bob, "list_decisions", json!({"on": "the epic"})).await,
        "on must be a key such as TST-12, not \"the epic\"."
    );
    let queue = refusal(&h, &bob, "list_decisions", json!({"queue": true})).await;
    assert!(queue.contains("unknown field `queue`"), "{queue}");
}

/// One row of `GET /stories` as `list_stories` summarises it, written out by hand from the API's JSON, not
/// through `StorySummary::from`.
fn story_summary(row: &Value) -> Value {
    let story = &row["story"];
    let criteria = story["criteria"].as_array().unwrap();
    json!({
        "key": story["key"],
        "title": story["title"],
        "kind": story["kind"],
        "epic": story["epic"],
        "owner": story["owner"],
        "delivery": story["delivery"],
        "resolution": story["resolution"],
        "labels": story["labels"],
        "version": story["version"],
        "blocked_by": story["blocked_by"],
        "criteria_done": criteria.iter().filter(|c| c["done"] == true).count(),
        "criteria_total": criteria.len(),
        "ready": row["readiness"]["ready"],
        "reasons": row["readiness"]["reasons"],
        "urgent": !story["urgent_sources"].as_array().unwrap().is_empty(),
        "promised_on": story["promise"]["on"],
    })
}

fn story_keys(page: &Value) -> Vec<String> {
    page["stories"]
        .as_array()
        .unwrap()
        .iter()
        .map(key)
        .collect()
}

#[tokio::test]
async fn list_stories_answers_the_route_s_page_as_summaries_for_every_filter() {
    let h = mcp_harness().await;
    let s = seed(&h).await;
    let bob = mcp_token(&h, BOB).await;
    let all = [&s.bobs, &s.dans, &s.untriaged, &s.ready, &s.dropped];
    // The tool's arguments, the route's query for them, and the control: the stories the route must list.
    let cases: Vec<(Value, String, Vec<&String>)> = vec![
        (json!({}), String::new(), all.to_vec()),
        (
            json!({"open": true}),
            "&done=none".into(),
            vec![&s.bobs, &s.dans, &s.untriaged, &s.ready],
        ),
        (
            json!({"owner": DAN}),
            format!("&owner={DAN}"),
            vec![&s.dans],
        ),
        (
            json!({"epic": s.epic}),
            format!("&epic={}", s.epic),
            vec![&s.bobs, &s.dans],
        ),
        (
            json!({"initiative": s.initiative}),
            format!("&initiative={}", s.initiative),
            vec![&s.bobs, &s.dans],
        ),
        (
            json!({"untriaged": true}),
            "&untriaged=true".into(),
            vec![&s.untriaged],
        ),
        (json!({"ready": true}), "&ready=true".into(), vec![&s.ready]),
        (
            json!({"urgent": true}),
            "&urgent=true".into(),
            vec![&s.dans],
        ),
        (json!({"kind": "bug"}), "&kind=bug".into(), vec![&s.dans]),
        (json!({"label": "web"}), "&label=web".into(), vec![&s.bobs]),
        (json!({"text": "slot"}), "&q=slot".into(), vec![&s.bobs]),
        (
            json!({"delivery": "done"}),
            "&delivery=done".into(),
            vec![&s.dropped],
        ),
        (
            json!({"promised": "any"}),
            "&promised=any".into(),
            vec![&s.bobs],
        ),
        // Filters combine.
        (
            json!({"epic": s.epic, "kind": "bug", "open": true}),
            format!("&epic={}&kind=bug&done=none", s.epic),
            vec![&s.dans],
        ),
    ];
    for (arguments, query, control) in cases {
        let rows = api(
            &h,
            &h.victim_token,
            &format!("/api/v1/stories?limit=25&offset=0{query}"),
        )
        .await;
        let rows = rows.as_array().unwrap();
        let mut listed: Vec<String> = rows.iter().map(|row| key(&row["story"])).collect();
        listed.sort();
        let mut control: Vec<String> = control.into_iter().cloned().collect();
        control.sort();
        assert_eq!(listed, control, "the control for {arguments}");
        let summaries: Vec<Value> = rows.iter().map(story_summary).collect();
        assert_eq!(
            answer(&h, &bob, "list_stories", arguments.clone()).await,
            json!({"stories": summaries, "next_offset": null}),
            "{arguments}"
        );
    }

    // The derived fields where they are not the default.
    let ready = answer(&h, &bob, "list_stories", json!({"ready": true})).await;
    assert_eq!(
        (
            &ready["stories"][0]["ready"],
            &ready["stories"][0]["reasons"]
        ),
        (&json!(true), &json!([]))
    );
    let urgent = answer(&h, &bob, "list_stories", json!({"urgent": true})).await;
    assert_eq!(urgent["stories"][0]["urgent"], true);
    let done = answer(&h, &bob, "list_stories", json!({"delivery": "done"})).await;
    assert_eq!(done["stories"][0]["resolution"], "dropped");
}

/// The `goal` filter needs an active sprint: a goal's id lists the stories linked to that goal, `any` the stories
/// serving some goal, `none` the rest.
#[tokio::test]
async fn list_stories_goal_filters_by_the_active_sprint_s_goals() {
    let h = mcp_harness().await;
    let dan = mcp_token(&h, DAN).await;
    let story = |idem: &str| json!({"idempotency_key": idem, "title": idem, "kind": "story", "criteria": [], "labels": []});
    let first = new_story(&h, story("gl-s1")).await;
    let second = new_story(&h, story("gl-s2")).await;
    let upkeep = new_story(&h, story("gl-s3")).await;
    let mut goals = Vec::new();
    for (idem, linked) in [("gl-g1", &first), ("gl-g2", &second)] {
        let goal = post(
            &h,
            "/api/v1/goals",
            json!({"idempotency_key": idem, "outcome": idem, "check": "c"}),
        )
        .await["goal"]["id"]
            .as_i64()
            .unwrap();
        post(
            &h,
            &format!("/api/v1/goals/{goal}/links"),
            json!({"items": [linked]}),
        )
        .await;
        goals.push(goal);
    }
    let draft = api(&h, &h.admin_token, "/api/v1/sprints").await["draft"].clone();
    post(
        &h,
        &format!("/api/v1/sprints/{}/open", draft["id"]),
        json!({"version": draft["version"]}),
    )
    .await;

    // The tool's `goal`, and the control: the stories the route lists for it. Four different lists.
    let cases = [
        (goals[0].to_string(), vec![&first]),
        (goals[1].to_string(), vec![&second]),
        ("any".to_string(), vec![&first, &second]),
        ("none".to_string(), vec![&upkeep]),
    ];
    for (goal, control) in cases {
        let rows = api(
            &h,
            &h.admin_token,
            &format!("/api/v1/stories?limit=25&offset=0&goal={goal}"),
        )
        .await;
        let mut listed: Vec<String> = rows
            .as_array()
            .unwrap()
            .iter()
            .map(|row| key(&row["story"]))
            .collect();
        listed.sort();
        let mut control: Vec<String> = control.into_iter().cloned().collect();
        control.sort();
        assert_eq!(listed, control, "the control for goal={goal}");
        let page = answer(&h, &dan, "list_stories", json!({"goal": goal})).await;
        let mut got = story_keys(&page);
        got.sort();
        assert_eq!(got, control, "goal={goal}");
    }
}

#[tokio::test]
async fn list_stories_mine_is_the_caller_as_owner_as_the_cli_sends_it() {
    let h = mcp_harness().await;
    let s = seed(&h).await;
    let bob = mcp_token(&h, BOB).await;
    let dan = mcp_token(&h, DAN).await;

    // "Mine and open" is the caller's plate: what the CLI's `story ls --mine --open` asks the route for.
    let plate = api(
        &h,
        &h.victim_token,
        &format!("/api/v1/stories?owner={BOB}&done=none&limit=25&offset=0"),
    )
    .await;
    let plate = plate.as_array().unwrap();
    assert_eq!(plate.len(), 1, "the control: bob has one open story");
    assert_eq!(plate[0]["story"]["key"], json!(s.bobs));
    let summaries: Vec<Value> = plate.iter().map(story_summary).collect();
    let mine_and_open = json!({"mine": true, "open": true});
    assert_eq!(
        answer(&h, &bob, "list_stories", mine_and_open.clone()).await,
        json!({"stories": summaries, "next_offset": null})
    );
    // The same arguments from another person list that person's work.
    let dans = answer(&h, &dan, "list_stories", mine_and_open).await;
    assert_eq!(story_keys(&dans), vec![s.dans.clone()]);
    // Without `open`, bob's done story is his too.
    let mut all_of_bobs =
        story_keys(&answer(&h, &bob, "list_stories", json!({"mine": true})).await);
    all_of_bobs.sort();
    let mut want = vec![s.bobs.clone(), s.dropped.clone()];
    want.sort();
    assert_eq!(all_of_bobs, want);
    // `mine` and `owner` say two different things: refused, not guessed.
    assert_eq!(
        refusal(
            &h,
            &bob,
            "list_stories",
            json!({"mine": true, "owner": DAN})
        )
        .await,
        "Give mine or owner, not both."
    );
}

#[tokio::test]
async fn list_stories_pages_with_next_offset_and_defaults_to_twenty_five() {
    let h = mcp_harness().await;
    let dan = mcp_token(&h, DAN).await;
    for n in 1..=26 {
        new_story(
            &h,
            json!({"idempotency_key": format!("pg-{n}"), "title": format!("Story {n}"), "kind": "story", "criteria": [], "labels": []}),
        )
        .await;
    }
    let all = api(&h, &h.admin_token, "/api/v1/stories").await;
    let all: Vec<String> = all
        .as_array()
        .unwrap()
        .iter()
        .map(|row| key(&row["story"]))
        .collect();
    assert_eq!(
        all.len(),
        26,
        "the control: the whole list, in backlog order"
    );

    // No limit: 25 stories, and where the next page starts.
    let first = answer(&h, &dan, "list_stories", json!({})).await;
    assert_eq!(story_keys(&first), &all[..25]);
    assert_eq!(first["next_offset"], 25);
    let rest = answer(&h, &dan, "list_stories", json!({"offset": 25})).await;
    assert_eq!(story_keys(&rest), &all[25..]);
    assert!(rest["next_offset"].is_null(), "{rest}");

    // Following `next_offset` covers the list exactly once, in order: 10, 10, 6.
    let mut seen = Vec::new();
    let mut offset = json!(0);
    for _ in 0..4 {
        let page = answer(
            &h,
            &dan,
            "list_stories",
            json!({"limit": 10, "offset": offset}),
        )
        .await;
        seen.extend(story_keys(&page));
        offset = page["next_offset"].clone();
        if offset.is_null() {
            break;
        }
    }
    assert!(offset.is_null(), "three pages of ten end the list");
    assert_eq!(seen, all);

    // A full last page still names a next offset; the page after it is empty and ends the list.
    let last_full = answer(&h, &dan, "list_stories", json!({"limit": 13, "offset": 13})).await;
    assert_eq!(story_keys(&last_full), &all[13..]);
    assert_eq!(last_full["next_offset"], 26);
    assert_eq!(
        answer(&h, &dan, "list_stories", json!({"limit": 13, "offset": 26})).await,
        json!({"stories": [], "next_offset": null})
    );

    // A page is the route's page for the same limit and offset.
    let route = api(&h, &h.admin_token, "/api/v1/stories?limit=10&offset=20").await;
    let summaries: Vec<Value> = route
        .as_array()
        .unwrap()
        .iter()
        .map(story_summary)
        .collect();
    assert_eq!(summaries.len(), 6, "the control");
    assert_eq!(
        answer(&h, &dan, "list_stories", json!({"limit": 10, "offset": 20})).await["stories"],
        json!(summaries)
    );

    // The largest page a chat may ask for is accepted, and holds all 26.
    let fifty = answer(&h, &dan, "list_stories", json!({"limit": 50})).await;
    assert_eq!(story_keys(&fifty), all);
    assert!(fifty["next_offset"].is_null(), "{fifty}");

    // The tool's page is at most 50, whatever the route would allow.
    for limit in [0, 51] {
        assert_eq!(
            refusal(&h, &dan, "list_stories", json!({"limit": limit})).await,
            "limit must be 1 to 50."
        );
    }
}

#[tokio::test]
async fn list_stories_says_what_does_not_fit() {
    let h = mcp_harness().await;
    let bob = mcp_token(&h, BOB).await;
    // The action's own validation, in its words.
    assert_eq!(
        refusal(&h, &bob, "list_stories", json!({"goal": "soon"})).await,
        "goal must be any, none or a goal id"
    );
    assert_eq!(
        refusal(&h, &bob, "list_stories", json!({"text": "x".repeat(201)})).await,
        "story text must be 1..=200 characters"
    );
    // A key that names nothing lists nothing; text that is no key is said to be none.
    assert_eq!(
        answer(&h, &bob, "list_stories", json!({"epic": "TST-999999"})).await,
        json!({"stories": [], "next_offset": null})
    );
    assert_eq!(
        refusal(
            &h,
            &bob,
            "list_stories",
            json!({"epic": "the booking epic"})
        )
        .await,
        "epic must be a key such as TST-12, not \"the booking epic\"."
    );
    // There is no `sort`.
    let sort = refusal(&h, &bob, "list_stories", json!({"sort": "relevance"})).await;
    assert!(sort.contains("unknown field `sort`"), "{sort}");
}

/// The three states `reeve sprint show` without an id meets, in the order a planner goes through them: no sprint,
/// only a draft, an active sprint beside the next draft.
#[tokio::test]
async fn get_sprint_reads_the_current_sprint_as_the_cli_picks_it() {
    let h = mcp_harness().await;
    let dan = mcp_token(&h, DAN).await;

    // None.
    assert_eq!(
        refusal(&h, &dan, "get_sprint", json!({})).await,
        "There is no sprint yet."
    );

    // Only a draft: the first goal creates it. The goal's notes are over the limit.
    let notes = "ö".repeat(2_001);
    let cut_notes = json!(format!("{}[cut: 1 more characters]", "ö".repeat(2_000)));
    post(
        &h,
        "/api/v1/goals",
        json!({"idempotency_key": "sp-g1", "outcome": "Customers can book", "check": "a customer booked one", "notes": notes}),
    )
    .await;
    let draft = api(&h, &h.admin_token, "/api/v1/sprints").await["draft"].clone();
    let draft_id = draft["id"].as_i64().unwrap();
    let mut want = api(&h, &h.admin_token, &format!("/api/v1/sprints/{draft_id}")).await;
    assert_eq!(want["sprint"]["state"], "draft", "the control");
    assert_eq!(
        want["sprint"]["goals"][0]["notes"],
        json!(notes),
        "the control: the API route answers the notes whole"
    );
    want["sprint"]["goals"][0]["notes"] = cut_notes.clone();
    assert_eq!(answer(&h, &dan, "get_sprint", json!({})).await, want);

    // An active sprint beside the next draft: the active one is read, and read by id, so its promises are there.
    let promised = new_story(
        &h,
        json!({"idempotency_key": "sp-s1", "title": "Promised this sprint", "kind": "story", "criteria": [], "labels": []}),
    )
    .await;
    let in_three_days = chrono::Utc::now().date_naive() + chrono::Days::new(3);
    post(
        &h,
        &format!("/api/v1/stories/{promised}/promise"),
        json!({"on": in_three_days}),
    )
    .await;
    post(
        &h,
        &format!("/api/v1/sprints/{draft_id}/open"),
        json!({"version": draft["version"]}),
    )
    .await;
    let listed = api(&h, &h.admin_token, "/api/v1/sprints").await;
    assert_eq!(
        listed["active"]["sprint"]["id"], draft_id,
        "the control: it is active"
    );
    assert!(
        listed["draft"].is_object(),
        "the control: opening made the next draft"
    );
    assert_eq!(
        listed["active"]["promises"],
        json!([]),
        "the control: the list carries no promises"
    );
    let mut want = api(&h, &h.admin_token, &format!("/api/v1/sprints/{draft_id}")).await;
    assert_eq!(
        want["promises"][0]["key"],
        json!(promised),
        "the control: the sprint read by id does"
    );
    want["sprint"]["goals"][0]["notes"] = cut_notes;
    assert_eq!(answer(&h, &dan, "get_sprint", json!({})).await, want);

    // With an id: that sprint, here the next draft; and an id that names none.
    let next = listed["draft"]["id"].as_i64().unwrap();
    assert_ne!(next, draft_id);
    assert_eq!(
        answer(&h, &dan, "get_sprint", json!({"id": next})).await,
        api(&h, &h.admin_token, &format!("/api/v1/sprints/{next}")).await
    );
    assert_eq!(
        refusal(&h, &dan, "get_sprint", json!({"id": 999_999})).await,
        "No sprint with id 999999."
    );
}
