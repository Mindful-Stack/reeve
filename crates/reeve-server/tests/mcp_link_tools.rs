//! The item link and web link write tools of `/mcp` (GRA-902), through the router, with real tokens from the
//! sign-in flow: each writes what its API route writes, answers a small receipt, records the client, and gates
//! nothing. The scope and the write limit for every write tool are in `mcp_write_tools.rs`.
#![cfg(feature = "static-oidc")]
mod common;
use common::oauth::{Flow, connect, flow};
use common::*;
use serde_json::{Value, json};

const DAN: &str = "dan@example.com";

async fn token(f: &Flow, session: &str, scope: &str) -> String {
    connect(f, session, scope).await.access_token
}

struct Items {
    initiative: String,
    epic: String,
    a: String,
    b: String,
}

/// A story with one criterion in `epic`, made through the API; its key.
async fn a_story(h: &Harness, idem: &str, title: &str, epic: &str) -> String {
    key(&post(
        h,
        "/api/v1/stories",
        json!({"idempotency_key": idem, "title": title, "kind": "story", "epic": epic,
               "criteria": ["it works"], "labels": []}),
    )
    .await)
}

/// An initiative "Billing", an epic whose discovery is complete, and two ready stories in it: `a` "Billing
/// account" and `b` "Invoice template". Made through the API as the admin.
async fn items(h: &Harness) -> Items {
    let initiative = key(&post(
        h,
        "/api/v1/initiatives",
        json!({"idempotency_key": "ml-i", "title": "Billing", "horizon": "Now"}),
    )
    .await);
    let epic = ready_epic(h, "ml-e").await;
    let a = a_story(h, "ml-a", "Billing account", &epic).await;
    let b = a_story(h, "ml-b", "Invoice template", &epic).await;
    Items {
        initiative,
        epic,
        a,
        b,
    }
}

fn link(t: &str, direction: &str, label: &str, key: &str, level: &str, title: &str) -> Value {
    json!({"type": t, "direction": direction, "label": label,
           "item": {"key": key, "level": level, "title": title}})
}

async fn links_of(h: &Harness, key: &str) -> Value {
    api(h, &h.admin_token, &format!("/api/v1/items/{key}/links")).await["links"].clone()
}

async fn web_links_of(h: &Harness, key: &str) -> Value {
    api(h, &h.admin_token, &format!("/api/v1/items/{key}/links")).await["web_links"].clone()
}

/// The last entry about `key`, as `(action, actor, via, client)`.
async fn last_entry(h: &Harness, key: &str) -> (String, String, String, Value) {
    trail(h, key).await.pop().unwrap()
}

fn by_dan_through_the_flow(action: &str) -> (String, String, String, Value) {
    (
        action.to_string(),
        DAN.to_string(),
        "interactive".to_string(),
        json!("127.0.0.1"),
    )
}

/// The note after a `follows` between two stories: the shared sentence, then where a dependency is recorded
/// (decision C of 2026-11-24). Written out here as a literal, not built from the server's function, so a change
/// of either sentence shows.
fn informational(from: &str, to: &str) -> String {
    format!(
        "Informational: this does not hold {from} back. If the person meant that {from} must wait for {to}, tell them: that is a blocker, which a person adds in the CLI or the web client; this app cannot add one."
    )
}

#[tokio::test]
async fn add_item_link_writes_what_the_route_writes_and_says_a_follows_is_informational() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let it = items(h).await;
    let follows = json!({"key": it.b, "type": "follows", "to": it.a});
    let receipt = answer(h, &dan, "add_item_link", follows.clone()).await;
    // Two stories and `follows`: the receipt says it is informational.
    assert_eq!(
        receipt,
        json!({"key": it.b, "type": "follows", "to": it.a, "changed": true,
               "note": informational(&it.b, &it.a)})
    );
    // Stored as the route stores it, read from either end through the API.
    assert_eq!(
        links_of(h, &it.b).await,
        json!([link(
            "follows",
            "out",
            "follows",
            &it.a,
            "story",
            "Billing account"
        )])
    );
    assert_eq!(
        links_of(h, &it.a).await,
        json!([link(
            "follows",
            "in",
            "is followed by",
            &it.b,
            "story",
            "Invoice template"
        )])
    );
    // kb/ADR-0031 R7 and kb/ADR-0029 R9: logged on both items, with the client.
    for item in [&it.a, &it.b] {
        assert_eq!(
            last_entry(h, item).await,
            by_dan_through_the_flow("item_link_added"),
            "{item}"
        );
    }
    // A repeat changes nothing, says so, and writes no entry.
    let entries_before = trail(h, &it.b).await.len();
    let repeat = answer(h, &dan, "add_item_link", follows).await;
    assert_eq!(
        repeat,
        json!({"key": it.b, "type": "follows", "to": it.a, "changed": false,
               "note": format!("Nothing was written: {} already follows {}. {}", it.b, it.a, informational(&it.b, &it.a))})
    );
    assert_eq!(trail(h, &it.b).await.len(), entries_before);
    assert_eq!(rows(h, "item_links").await, 1);

    // No note when the name is not `follows`, or an end is not a story.
    let relates = answer(
        h,
        &dan,
        "add_item_link",
        json!({"key": it.a, "type": "relates_to", "to": it.b}),
    )
    .await;
    assert_eq!(
        relates,
        json!({"key": it.a, "type": "relates_to", "to": it.b, "changed": true})
    );
    let to_epic = answer(
        h,
        &dan,
        "add_item_link",
        json!({"key": it.b, "type": "follows", "to": it.epic}),
    )
    .await;
    assert_eq!(
        to_epic,
        json!({"key": it.b, "type": "follows", "to": it.epic, "changed": true})
    );
    // `relates_to` is one link whichever end names it (kb/ADR-0031 R4).
    let reversed = answer(
        h,
        &dan,
        "add_item_link",
        json!({"key": it.b, "type": "relates_to", "to": it.a}),
    )
    .await;
    assert_eq!(
        reversed,
        json!({"key": it.b, "type": "relates_to", "to": it.a, "changed": false,
               "note": format!("Nothing was written: {} already relates to {}.", it.b, it.a)})
    );

    // The same link through the route reads the same: one action behind both.
    let (status, by_route) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        &format!("/api/v1/items/{}/links", it.epic),
        Some(json!({"type": "duplicates", "to": it.initiative})),
    )
    .await;
    assert_eq!(status, 200, "{by_route}");
    answer(
        h,
        &dan,
        "add_item_link",
        json!({"key": it.a, "type": "duplicates", "to": it.initiative}),
    )
    .await;
    let duplicate = link(
        "duplicates",
        "out",
        "duplicates",
        &it.initiative,
        "initiative",
        "Billing",
    );
    assert!(
        by_route["value"].as_array().unwrap().contains(&duplicate),
        "{by_route}"
    );
    assert!(
        links_of(h, &it.a)
            .await
            .as_array()
            .unwrap()
            .contains(&duplicate)
    );
}

#[tokio::test]
async fn the_item_link_tools_tell_the_model_what_the_planner_refused() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let it = items(h).await;
    for name in ["add_item_link", "remove_item_link"] {
        let said = |arguments: Value| refusal(h, &dan, name, arguments);
        assert_eq!(
            said(json!({"key": it.a, "type": "relates_to", "to": it.a})).await,
            "an item cannot have an item link to itself",
            "{name}"
        );
        assert_eq!(
            said(json!({"key": it.b, "type": "follows", "to": "TST-999"})).await,
            format!(
                "No item with key \"{}\", or no item with key \"TST-999\". Keys look like TST-123.",
                it.b
            ),
            "{name}"
        );
        assert_eq!(
            said(json!({"key": "TST-999", "type": "follows", "to": it.a})).await,
            format!(
                "No item with key \"TST-999\", or no item with key \"{}\". Keys look like TST-123.",
                it.a
            ),
            "{name}"
        );
        assert_eq!(
            said(json!({"key": it.b, "type": "follows", "to": "bookings"})).await,
            "to must be a key such as TST-12, not \"bookings\".",
            "{name}"
        );
        // kb/ADR-0031 R2: no name means holding work back.
        for blocking in ["blocks", "depends_on"] {
            let said = said(json!({"key": it.b, "type": blocking, "to": it.a})).await;
            assert!(
                said.starts_with("Invalid arguments: unknown variant"),
                "{name} {blocking}: {said}"
            );
        }
        for extra in ["blocker", "url", "override_reason"] {
            let mut arguments = json!({"key": it.b, "type": "follows", "to": it.a});
            arguments[extra] = json!("x");
            let said = said(arguments).await;
            assert!(
                said.starts_with("Invalid arguments: unknown field"),
                "{name} {extra}: {said}"
            );
        }
    }
    assert_eq!(rows(h, "item_links").await, 0, "nothing was written");
}

#[tokio::test]
async fn remove_item_link_removes_the_link_named_as_it_was_added() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let it = items(h).await;
    answer(
        h,
        &dan,
        "add_item_link",
        json!({"key": it.b, "type": "follows", "to": it.a}),
    )
    .await;
    let entries_before = (trail(h, &it.a).await.len(), trail(h, &it.b).await.len());

    // Named the other way round, it is another link, which is not there: nothing is removed.
    let reversed = answer(
        h,
        &dan,
        "remove_item_link",
        json!({"key": it.a, "type": "follows", "to": it.b}),
    )
    .await;
    assert_eq!(
        reversed,
        json!({"key": it.a, "type": "follows", "to": it.b, "changed": false,
               "note": format!("Nothing was removed: there is no item link \"{} follows {}\". An item link is removed as it was added: for one that an item shows with `direction: in`, send the other item as `key`.", it.a, it.b)})
    );
    assert_eq!(rows(h, "item_links").await, 1);
    assert_eq!(
        (trail(h, &it.a).await.len(), trail(h, &it.b).await.len()),
        entries_before,
        "no entry for a removal that removed nothing"
    );

    let removed = answer(
        h,
        &dan,
        "remove_item_link",
        json!({"key": it.b, "type": "follows", "to": it.a}),
    )
    .await;
    assert_eq!(
        removed,
        json!({"key": it.b, "type": "follows", "to": it.a, "changed": true})
    );
    assert_eq!(links_of(h, &it.a).await, json!([]));
    assert_eq!(links_of(h, &it.b).await, json!([]));
    for item in [&it.a, &it.b] {
        assert_eq!(
            last_entry(h, item).await,
            by_dan_through_the_flow("item_link_removed"),
            "{item}"
        );
    }
    // Again: nothing to remove.
    let again = answer(
        h,
        &dan,
        "remove_item_link",
        json!({"key": it.b, "type": "follows", "to": it.a}),
    )
    .await;
    assert_eq!(again["changed"], false);
}

#[tokio::test]
async fn add_web_link_writes_what_the_route_writes_and_answers_its_id() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let it = items(h).await;
    let scenario = "https://example.com/wiki/scenario";
    let receipt = answer(
        h,
        &dan,
        "add_web_link",
        json!({"key": it.epic, "title": "  Scenario page ", "url": "HTTPS://Example.com/wiki/scenario"}),
    )
    .await;
    let id = receipt["id"].as_i64().unwrap();
    // The receipt carries the title and the URL as stored, not as sent.
    assert_eq!(
        receipt,
        json!({"item": it.epic, "id": id, "title": "Scenario page", "url": scenario, "changed": true})
    );
    assert_eq!(
        web_links_of(h, &it.epic).await,
        json!([{"id": id, "title": "Scenario page", "url": scenario}])
    );
    assert_eq!(
        last_entry(h, &it.epic).await,
        by_dan_through_the_flow("web_link_added")
    );
    // The same web link through the route, on another item, is stored the same: one action behind both.
    let (status, by_route) = call(
        &h.app,
        Some(&h.admin_token),
        "POST",
        &format!("/api/v1/items/{}/web-links", it.a),
        Some(json!({"title": "  Scenario page ", "url": "HTTPS://Example.com/wiki/scenario"})),
    )
    .await;
    assert_eq!(status, 200, "{by_route}");
    assert_eq!(
        json!([by_route["value"][0]["title"], by_route["value"][0]["url"]]),
        json!([receipt["title"], receipt["url"]])
    );

    // A repeat, in another spelling of the URL: the same id, nothing written.
    let entries_before = trail(h, &it.epic).await.len();
    let repeat = answer(
        h,
        &dan,
        "add_web_link",
        json!({"key": it.epic, "title": "Scenario page", "url": "https://EXAMPLE.com/wiki/scenario"}),
    )
    .await;
    assert_eq!(
        repeat,
        json!({"item": it.epic, "id": id, "title": "Scenario page", "url": scenario, "changed": false,
               "note": "Nothing was written: the item already has this web link."})
    );
    assert_eq!(trail(h, &it.epic).await.len(), entries_before);

    // What the planner refuses is told in its words, and nothing is written.
    let said = |arguments: Value| refusal(h, &dan, "add_web_link", arguments);
    assert_eq!(
        said(json!({"key": it.epic, "title": "Scenario", "url": scenario})).await,
        "this item already has a web link to that URL, titled \"Scenario page\": remove it first, then add it with the new title"
    );
    let url_rule = "a web link's URL must start with http:// or https://, be at most 1000 characters and hold no user name or password";
    for bad in [
        "ftp://example.com/file",
        "https://dan:secret@example.com/",
        "example.com",
    ] {
        assert_eq!(
            said(json!({"key": it.epic, "title": "Bad", "url": bad})).await,
            url_rule,
            "{bad}"
        );
    }
    assert_eq!(
        said(json!({"key": it.epic, "title": " ", "url": "https://example.com/x"})).await,
        "a web link's title must be 1 to 200 characters on one line, with no tab or other control character"
    );
    assert_eq!(
        said(json!({"key": "TST-999", "title": "Page", "url": "https://example.com/x"})).await,
        "No item with key \"TST-999\". Keys look like TST-123."
    );
    for extra in ["type", "to", "id"] {
        let mut arguments =
            json!({"key": it.epic, "title": "Page", "url": "https://example.com/x"});
        arguments[extra] = json!("x");
        assert!(
            said(arguments)
                .await
                .starts_with("Invalid arguments: unknown field")
        );
    }
    assert_eq!(
        rows(h, "web_links").await,
        2,
        "the epic's and the story's, and no other"
    );
}

#[tokio::test]
async fn remove_web_link_removes_by_id_on_its_own_item_only() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let it = items(h).await;
    let add = |item: &str, title: &str, url: &str| {
        answer(
            h,
            &dan,
            "add_web_link",
            json!({"key": item, "title": title, "url": url}),
        )
    };
    let on_epic = add(&it.epic, "Spec", "https://example.com/spec").await["id"]
        .as_i64()
        .unwrap();
    let on_story = add(&it.a, "Board", "https://example.com/board").await["id"]
        .as_i64()
        .unwrap();

    // Another item's id: nothing is removed, and the receipt says where ids are shown.
    let entries_before = trail(h, &it.epic).await.len();
    let wrong = answer(
        h,
        &dan,
        "remove_web_link",
        json!({"key": it.epic, "id": on_story}),
    )
    .await;
    assert_eq!(
        wrong,
        json!({"item": it.epic, "id": on_story, "changed": false,
               "note": format!("Nothing was removed: {} has no web link with id {on_story}. get_story, get_epic and get_initiative show each web link's `id`.", it.epic)})
    );
    assert_eq!(rows(h, "web_links").await, 2);
    assert_eq!(trail(h, &it.epic).await.len(), entries_before);

    let removed = answer(
        h,
        &dan,
        "remove_web_link",
        json!({"key": it.epic, "id": on_epic}),
    )
    .await;
    assert_eq!(
        removed,
        json!({"item": it.epic, "id": on_epic, "changed": true})
    );
    assert_eq!(web_links_of(h, &it.epic).await, json!([]));
    assert_eq!(
        web_links_of(h, &it.a).await[0]["id"],
        json!(on_story),
        "the other item's web link is untouched"
    );
    assert_eq!(
        last_entry(h, &it.epic).await,
        by_dan_through_the_flow("web_link_removed")
    );
    // Again, and on an item that does not exist.
    assert_eq!(
        answer(
            h,
            &dan,
            "remove_web_link",
            json!({"key": it.epic, "id": on_epic})
        )
        .await["changed"],
        false
    );
    assert_eq!(
        refusal(
            h,
            &dan,
            "remove_web_link",
            json!({"key": "TST-999", "id": on_story})
        )
        .await,
        "No item with key \"TST-999\". Keys look like TST-123."
    );
    // By id only: a URL is not an argument.
    let said = refusal(
        h,
        &dan,
        "remove_web_link",
        json!({"key": it.a, "url": "https://example.com/board"}),
    )
    .await;
    assert!(said.starts_with("Invalid arguments:"), "{said}");
    assert_eq!(rows(h, "web_links").await, 1);
}

/// A receipt carries the longest web link the planner stores, whole: a title of 200 characters and a URL of
/// 1 000. Nothing in a web link's receipt is cut (a cut URL is another address), and the receipt of the largest
/// one is still a small answer.
#[tokio::test]
async fn a_web_link_receipt_carries_the_longest_title_and_url_whole() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let it = items(h).await;
    // Exactly at both limits, in the form the planner stores (lower-case host, nothing to encode), so what is sent
    // is what is stored. The title is 200 two-byte characters: characters are counted, not bytes.
    let prefix = "https://example.com/";
    let url = format!("{prefix}{}", "a".repeat(1_000 - prefix.len()));
    let title = "å".repeat(200);
    assert_eq!(
        (url.chars().count(), title.chars().count()),
        (1_000, 200),
        "the control"
    );
    let receipt = answer(
        h,
        &dan,
        "add_web_link",
        json!({"key": it.epic, "title": title, "url": url}),
    )
    .await;
    assert_eq!(
        receipt,
        json!({"item": it.epic, "id": receipt["id"], "title": title, "url": url, "changed": true})
    );
    assert!(
        !receipt.to_string().contains("[cut:"),
        "nothing in a web link's receipt is cut"
    );
    assert!(
        receipt.to_string().len() < 2_000,
        "{} bytes",
        receipt.to_string().len()
    );
    // The control: one character more is the planner's refusal, so 1 000 really is the most a receipt can hold.
    let said = refusal(
        h,
        &dan,
        "add_web_link",
        json!({"key": it.epic, "title": "Too long", "url": format!("{url}a")}),
    )
    .await;
    assert_eq!(
        said,
        "a web link's URL must start with http:// or https://, be at most 1000 characters and hold no user name or password"
    );
    // A repeat answers the same whole URL.
    let repeat = answer(
        h,
        &dan,
        "add_web_link",
        json!({"key": it.epic, "title": title, "url": url}),
    )
    .await;
    assert_eq!(
        (repeat["changed"].clone(), repeat["url"].clone()),
        (json!(false), json!(url))
    );
}

/// What `GET /stories/{key}/ready` answers for `story`: whether it is ready, and every reason it is not.
async fn readiness(h: &Harness, story: &str) -> Value {
    api(h, &h.admin_token, &format!("/api/v1/stories/{story}/ready")).await
}

/// What no link tool may change, read from the database and through the API: every blocker row, and for each of
/// `stories` its readiness with its reasons, its delivery state, its version and its `blocked_by`.
async fn blockers_and_state(h: &Harness, stories: &[&String]) -> Value {
    let blocks: Vec<(String, String)> = sqlx::query_as(
        "SELECT story_key, blocker_key FROM story_blocks ORDER BY story_key, blocker_key",
    )
    .fetch_all(&h.state.pool)
    .await
    .unwrap();
    let mut state = Vec::new();
    for key in stories {
        let story = api(h, &h.admin_token, &format!("/api/v1/stories/{key}")).await;
        state.push(json!({
            "key": key,
            "readiness": readiness(h, key).await,
            "delivery": story["delivery"],
            "version": story["version"],
            "blocked_by": story["blocked_by"],
        }));
    }
    json!({"blocks": blocks, "stories": state})
}

/// kb/ADR-0031 R3 through the router, and the owner's rule for GRA-902 (the observation on kb/ADR-0029 names this
/// test): an item link or a web link a tool writes changes no readiness and no version, and a story that `follows`
/// another starts before it. And where a blocker does hold a story back, every successful add and removal by each
/// of the four link tools, on the blocked story and on its blocker, under all three item link names, leaves the
/// blocker rows, both stories' readiness, their delivery states and their versions exactly as they were.
#[tokio::test]
async fn the_link_tools_gate_nothing_and_no_tool_touches_a_blocker() {
    let f = flow().await;
    let h = &f.h;
    let dan = token(&f, &h.admin_token, "write").await;
    let it = items(h).await;
    let story = |key: String| async move {
        api(h, &h.admin_token, &format!("/api/v1/stories/{key}")).await
    };
    let before = (
        readiness(h, &it.b).await,
        story(it.b.clone()).await["version"].clone(),
        story(it.a.clone()).await["version"].clone(),
    );
    assert_eq!(before.0["ready"], true, "the control: {}", before.0);

    answer(
        h,
        &dan,
        "add_item_link",
        json!({"key": it.b, "type": "follows", "to": it.a}),
    )
    .await;
    answer(
        h,
        &dan,
        "add_web_link",
        json!({"key": it.b, "title": "Spec", "url": "https://example.com/spec"}),
    )
    .await;
    let b = story(it.b.clone()).await;
    assert_eq!(
        (
            readiness(h, &it.b).await,
            b["version"].clone(),
            story(it.a.clone()).await["version"].clone()
        ),
        before,
        "no readiness and no version moved"
    );
    assert_eq!(b["blocked_by"], json!([]), "an item link is not a blocker");
    // `b` follows `a`, `a` is still in todo, and `b` starts all the same.
    let moved = answer(
        h,
        &dan,
        "move_story",
        json!({"key": it.b, "to": "in_progress", "version": b["version"]}),
    )
    .await;
    assert_eq!(moved["delivery"], "in_progress");

    // Where a blocker exists: `c` is blocked by `d`, both in todo, set through the API's own route, so `c` is held.
    let c = a_story(h, "ml-c", "Invoice email", &it.epic).await;
    let d = a_story(h, "ml-d", "Mail provider", &it.epic).await;
    post(
        h,
        &format!("/api/v1/stories/{c}/blocks"),
        json!({"blocker": d}),
    )
    .await;
    let fixed = blockers_and_state(h, &[&c, &d]).await;
    assert_eq!(
        fixed["blocks"],
        json!([[c, d]]),
        "the control: exactly one blocker row"
    );
    assert_eq!(
        json!([
            fixed["stories"][0]["readiness"]["ready"],
            fixed["stories"][0]["readiness"]["reasons"][0]["reason"],
            fixed["stories"][1]["readiness"]["ready"],
        ]),
        json!([false, "blocked_by", true]),
        "the control: `c` is held by its blocker alone and `d` is ready: {fixed}"
    );

    // The rest runs over bob's connection: a person's limit is 30 write calls in ten minutes, refused ones
    // included, and dan has used some.
    let bob = token(&f, &h.victim_token, "write").await;
    // Every name from the blocked story to its blocker, and the two directed names the other way round
    // (`relates_to` the other way round is the same item link).
    let links = [
        (&c, "follows", &d),
        (&c, "duplicates", &d),
        (&c, "relates_to", &d),
        (&d, "follows", &c),
        (&d, "duplicates", &c),
    ];
    let named =
        |(key, name, to): (&String, &str, &String)| json!({"key": key, "type": name, "to": to});
    // Each step: the tool, its arguments, and whether the call must change something.
    let mut steps: Vec<(&str, Value, bool)> = Vec::new();
    // Before any item link joins them, a removal under every name, either way round, finds nothing: the blocker
    // edge is not an item link under any name.
    for link in links {
        steps.push(("remove_item_link", named(link), false));
    }
    steps.push(("remove_item_link", named((&d, "relates_to", &c)), false));
    // Then every add succeeds.
    for link in links {
        steps.push(("add_item_link", named(link), true));
    }
    for (n, (tool, arguments, changes)) in steps.into_iter().enumerate() {
        let receipt = answer(h, &bob, tool, arguments.clone()).await;
        assert_eq!(receipt["changed"], changes, "step {n}: {tool} {arguments}");
        assert_eq!(
            blockers_and_state(h, &[&c, &d]).await,
            fixed,
            "after step {n}: {tool} {arguments}"
        );
    }
    assert_eq!(
        rows(h, "item_links").await,
        6,
        "the five just added and `b follows a`: the adds really wrote"
    );

    // A web link on each of the two stories.
    let mut web_links = Vec::new();
    for key in [&c, &d] {
        let receipt = answer(
            h,
            &bob,
            "add_web_link",
            json!({"key": key, "title": "Runbook", "url": format!("https://example.com/runbook/{key}")}),
        )
        .await;
        assert_eq!(receipt["changed"], true, "add_web_link on {key}");
        assert_eq!(
            blockers_and_state(h, &[&c, &d]).await,
            fixed,
            "after add_web_link on {key}"
        );
        web_links.push((key, receipt["id"].clone()));
    }
    assert_eq!(
        rows(h, "web_links").await,
        3,
        "the two just added and the one on `b`"
    );

    // Then every removal succeeds, of both kinds.
    for link in links {
        let arguments = named(link);
        let receipt = answer(h, &bob, "remove_item_link", arguments.clone()).await;
        assert_eq!(receipt["changed"], true, "remove_item_link {arguments}");
        assert_eq!(
            blockers_and_state(h, &[&c, &d]).await,
            fixed,
            "after remove_item_link {arguments}"
        );
    }
    for (key, id) in web_links {
        let receipt = answer(h, &bob, "remove_web_link", json!({"key": key, "id": id})).await;
        assert_eq!(receipt["changed"], true, "remove_web_link on {key}");
        assert_eq!(
            blockers_and_state(h, &[&c, &d]).await,
            fixed,
            "after remove_web_link on {key}"
        );
    }
    assert_eq!(
        (rows(h, "item_links").await, rows(h, "web_links").await),
        (1, 1),
        "what was added here is gone, and nothing else"
    );

    // No name means blocking (kb/ADR-0031 R2): refused with the schema, on both item link tools.
    for tool in ["add_item_link", "remove_item_link"] {
        let said = refusal(h, &dan, tool, json!({"key": c, "type": "blocks", "to": d})).await;
        assert!(said.starts_with("Invalid arguments:"), "{tool}: {said}");
    }
    assert_eq!(
        blockers_and_state(h, &[&c, &d]).await,
        fixed,
        "still held by its blocker, by that alone"
    );
    // And no tool is about blockers, discovery or overriding.
    let (_, _, body) = send(&h.app, mcp_modern(&dan, 1, "tools/list", json!({}))).await;
    let listed = serde_json::from_str::<Value>(&body).unwrap()["result"]["tools"].clone();
    let names: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 21);
    assert!(
        !names.iter().any(|name| ["block", "discovery", "override"]
            .iter()
            .any(|word| name.contains(word))),
        "{names:?}"
    );
}
