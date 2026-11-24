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
