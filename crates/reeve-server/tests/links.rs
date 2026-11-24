//! Item links and web links over the API (GRA-877, GRA-878, kb/ADR-0031): each route's happy path and each error,
//! the forward form on item link writes, and which reads carry `links` and `web_links`.
mod common;
use common::*;
use serde_json::{Value, json};

struct Items {
    initiative: String,
    epic: String,
    a: String,
    b: String,
}

/// An initiative "Billing", its epic "Invoices", and two stories in it: `a` "Billing account" and `b` "Invoice
/// template". Made through the API as the admin.
async fn items(h: &Harness) -> Items {
    let initiative = key(&post(
        h,
        "/api/v1/initiatives",
        json!({"idempotency_key": "l-i", "title": "Billing", "horizon": "Now"}),
    )
    .await);
    let epic = key(&post(
        h,
        "/api/v1/epics",
        json!({"idempotency_key": "l-e", "title": "Invoices", "initiative": initiative}),
    )
    .await);
    let a = key(&post(
        h,
        "/api/v1/stories",
        json!({"idempotency_key": "l-a", "title": "Billing account", "kind": "story", "epic": epic, "criteria": [], "labels": []}),
    )
    .await);
    let b = key(&post(
        h,
        "/api/v1/stories",
        json!({"idempotency_key": "l-b", "title": "Invoice template", "kind": "story", "epic": epic, "criteria": [], "labels": []}),
    )
    .await);
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

/// One item's activity entries of one action, oldest first.
async fn entries(h: &Harness, key: &str, action: &str) -> Vec<Value> {
    let mut rows = api(
        h,
        &h.admin_token,
        &format!("/api/v1/activity?entity_key={key}&action={action}"),
    )
    .await
    .as_array()
    .unwrap()
    .clone();
    rows.sort_by_key(|e| e["id"].as_i64());
    rows
}

#[tokio::test]
async fn item_links_are_added_listed_and_removed_in_their_forward_form() {
    let h = harness().await;
    let it = items(&h).await;
    let admin = Some(h.admin_token.as_str());
    let bot = Some(h.bot_token.as_str());
    let on_b = format!("/api/v1/items/{}/links", it.b);

    // b follows a, written with an automation token: any caller who may edit an item may link it.
    let follows = json!({"type": "follows", "to": it.a});
    let (st, added) = call(&h.app, bot, "POST", &on_b, Some(follows.clone())).await;
    assert_eq!(st, 200, "{added}");
    assert_eq!(
        added,
        json!({"outcome": "ok", "value": [link("follows", "out", "follows", &it.a, "story", "Billing account")]}),
        "the answer is the item's links"
    );
    assert_eq!(
        links_of(&h, &it.a).await,
        json!([link(
            "follows",
            "in",
            "is followed by",
            &it.b,
            "story",
            "Invoice template"
        )]),
        "the other item reads it with the reverse wording"
    );
    // A member reads them too.
    assert!(api(&h, &h.victim_token, &on_b).await["links"].is_array());

    // Again: ok, the same answer, nothing more logged.
    let (st, again) = call(&h.app, bot, "POST", &on_b, Some(follows)).await;
    assert_eq!((st, &again), (200, &added));
    for k in [&it.a, &it.b] {
        let logged = entries(&h, k, "item_link_added").await;
        assert_eq!(logged.len(), 1, "{k}: one entry, none for the repeat");
        assert_eq!(logged[0]["entity_type"], "story");
        assert_eq!(logged[0]["via"], "automation");
        assert_eq!(
            logged[0]["payload"],
            json!({"type": "follows", "from": it.b, "to": it.a})
        );
    }

    // The reverse wording cannot be written: naming `a` first names another link, which does not exist.
    let reverse = format!("/api/v1/items/{}/links/follows/{}", it.a, it.b);
    let (st, v) = call(&h.app, admin, "DELETE", &reverse, None).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(
        links_of(&h, &it.a).await.as_array().unwrap().len(),
        1,
        "the `in` link is still there"
    );
    // The link shown on `a` as "is followed by b" is removed by its forward form: `b` first.
    let forward = format!("/api/v1/items/{}/links/follows/{}", it.b, it.a);
    let (st, v) = call(&h.app, admin, "DELETE", &forward, None).await;
    assert_eq!((st, &v), (200, &json!({"outcome": "ok", "value": []})));
    assert_eq!(links_of(&h, &it.a).await, json!([]));
    let (st, _) = call(&h.app, admin, "DELETE", &forward, None).await;
    assert_eq!(st, 200, "removing a link that does not exist is ok");
    for k in [&it.a, &it.b] {
        assert_eq!(entries(&h, k, "item_link_removed").await.len(), 1, "{k}");
    }

    // relates_to takes either order on add and on remove, between an epic and an initiative.
    let relates = |from: &str, to: &str| {
        let (path, body) = (
            format!("/api/v1/items/{from}/links"),
            json!({"type": "relates_to", "to": to}),
        );
        let app = h.app.clone();
        let token = h.admin_token.clone();
        async move { call(&app, Some(&token), "POST", &path, Some(body)).await.0 }
    };
    assert_eq!(relates(&it.epic, &it.initiative).await, 200);
    assert_eq!(relates(&it.initiative, &it.epic).await, 200);
    assert_eq!(
        links_of(&h, &it.epic).await,
        json!([link(
            "relates_to",
            "out",
            "relates to",
            &it.initiative,
            "initiative",
            "Billing"
        )])
    );
    assert_eq!(
        links_of(&h, &it.initiative).await,
        json!([link(
            "relates_to",
            "out",
            "relates to",
            &it.epic,
            "epic",
            "Invoices"
        )])
    );
    assert_eq!(
        entries(&h, &it.epic, "item_link_added").await.len(),
        1,
        "one link, whichever end named it"
    );
    let from_the_other_end = format!(
        "/api/v1/items/{}/links/relates_to/{}",
        it.initiative, it.epic
    );
    let (st, v) = call(&h.app, admin, "DELETE", &from_the_other_end, None).await;
    assert_eq!((st, &v), (200, &json!({"outcome": "ok", "value": []})));
    assert_eq!(links_of(&h, &it.epic).await, json!([]));

    // An external reference names either end, in the body and in both path segments.
    sqlx::query("UPDATE stories SET external_ref = 'SCRUM-31' WHERE key = $1")
        .bind(&it.a)
        .execute(&h.state.pool)
        .await
        .unwrap();
    let (st, v) = call(
        &h.app,
        admin,
        "POST",
        &on_b,
        Some(json!({"type": "duplicates", "to": "SCRUM-31"})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(
        v["value"],
        json!([link(
            "duplicates",
            "out",
            "duplicates",
            &it.a,
            "story",
            "Billing account"
        )]),
        "the answer names the native key"
    );
    assert_eq!(
        links_of(&h, "SCRUM-31").await,
        json!([link(
            "duplicates",
            "in",
            "is duplicated by",
            &it.b,
            "story",
            "Invoice template"
        )])
    );
    let by_ref = format!("/api/v1/items/{}/links/duplicates/SCRUM-31", it.b);
    let (st, v) = call(&h.app, admin, "DELETE", &by_ref, None).await;
    assert_eq!((st, &v), (200, &json!({"outcome": "ok", "value": []})));
}

#[tokio::test]
async fn item_link_errors_are_not_found_or_validation() {
    let h = harness().await;
    let it = items(&h).await;
    let t = Some(h.admin_token.as_str());
    let code = |(st, v): (u16, Value)| (st, v["code"].as_str().map(str::to_string));
    let not_found = (404, Some("not_found".to_string()));
    let validation = (400, Some("validation".to_string()));
    let on = |k: &str| format!("/api/v1/items/{k}/links");

    assert_eq!(
        code(call(&h.app, None, "GET", &on(&it.a), None).await).0,
        401
    );

    // An unknown key at either end.
    assert_eq!(
        code(call(&h.app, t, "GET", &on("TST-999"), None).await),
        not_found
    );
    let to_a = json!({"type": "follows", "to": it.a});
    assert_eq!(
        code(call(&h.app, t, "POST", &on("TST-999"), Some(to_a)).await),
        not_found
    );
    let to_ghost = json!({"type": "follows", "to": "TST-999"});
    assert_eq!(
        code(call(&h.app, t, "POST", &on(&it.a), Some(to_ghost)).await),
        not_found
    );
    for path in [
        format!("{}/follows/TST-999", on(&it.a)),
        format!("{}/follows/{}", on("TST-999"), it.a),
    ] {
        assert_eq!(
            code(call(&h.app, t, "DELETE", &path, None).await),
            not_found,
            "{path}"
        );
    }

    // A name outside the closed set, in the body and in the path. Blocking is not an item link.
    for name in ["blocks", "depends_on", "relates-to"] {
        let body = json!({"type": name, "to": it.b});
        assert_eq!(
            code(call(&h.app, t, "POST", &on(&it.a), Some(body)).await),
            validation,
            "{name}"
        );
        let path = format!("{}/{name}/{}", on(&it.a), it.b);
        assert_eq!(
            code(call(&h.app, t, "DELETE", &path, None).await),
            validation,
            "{name}"
        );
    }
    let (_, v) = call(
        &h.app,
        t,
        "DELETE",
        &format!("{}/blocks/{}", on(&it.a), it.b),
        None,
    )
    .await;
    assert_eq!(
        v["message"],
        "unknown item link name \"blocks\": one of follows, duplicates, relates_to"
    );

    // A link from an item to itself, and a body with a field the type does not have.
    let (st, v) = call(
        &h.app,
        t,
        "POST",
        &on(&it.a),
        Some(json!({"type": "follows", "to": it.a})),
    )
    .await;
    assert_eq!(
        (st, v["message"].as_str()),
        (400, Some("an item cannot have an item link to itself"))
    );
    let extra = json!({"type": "follows", "to": it.b, "direction": "in"});
    assert_eq!(
        code(call(&h.app, t, "POST", &on(&it.a), Some(extra)).await),
        validation
    );

    // None of it wrote anything.
    assert_eq!(links_of(&h, &it.a).await, json!([]));
    assert_eq!(links_of(&h, &it.b).await, json!([]));
}

/// Which answers carry `links` and `web_links`: every read of one item, and never a read of several.
#[tokio::test]
async fn reads_of_one_item_carry_links_over_the_api_and_lists_do_not() {
    let h = harness().await;
    let it = items(&h).await;
    post(
        &h,
        &format!("/api/v1/items/{}/links", it.b),
        json!({"type": "follows", "to": it.a}),
    )
    .await;
    post(
        &h,
        &format!("/api/v1/items/{}/links", it.epic),
        json!({"type": "relates_to", "to": it.initiative}),
    )
    .await;
    post(
        &h,
        &format!("/api/v1/items/{}/web-links", it.b),
        json!({"title": "Page", "url": "https://example.com/"}),
    )
    .await;
    let get = |path: String| {
        let h = &h;
        async move { api(h, &h.admin_token, &path).await }
    };

    assert_eq!(
        get(format!("/api/v1/stories/{}", it.b)).await["links"][0]["label"],
        "follows"
    );
    assert_eq!(
        get(format!("/api/v1/stories/{}", it.a)).await["links"][0]["label"],
        "is followed by"
    );
    assert_eq!(
        get(format!("/api/v1/epics/{}", it.epic)).await["links"][0]["label"],
        "relates to"
    );
    assert_eq!(
        get(format!("/api/v1/initiatives/{}", it.initiative)).await["links"][0]["item"]["key"],
        json!(it.epic)
    );
    assert_eq!(
        get(format!("/api/v1/stories/{}", it.b)).await["web_links"][0]["title"],
        "Page"
    );
    // An item without web links answers an empty list, never a missing field.
    for path in [
        format!("/api/v1/stories/{}", it.a),
        format!("/api/v1/epics/{}", it.epic),
        format!("/api/v1/initiatives/{}", it.initiative),
    ] {
        assert_eq!(get(path.clone()).await["web_links"], json!([]), "{path}");
    }
    let context = get(format!("/api/v1/stories/{}/context", it.b)).await;
    assert_eq!(context["story"]["links"][0]["item"]["key"], json!(it.a));
    assert_eq!(context["story"]["web_links"][0]["title"], "Page");
    assert!(context["epic"].get("web_links").is_none(), "{context}");
    assert!(
        context["initiative"].get("web_links").is_none(),
        "{context}"
    );
    assert!(context["epic"].get("links").is_none(), "{context}");
    assert!(context["initiative"].get("links").is_none(), "{context}");

    for (path, row) in [
        ("/api/v1/stories", "story"),
        ("/api/v1/epics", ""),
        ("/api/v1/initiatives", ""),
    ] {
        let listed = get(path.to_string()).await;
        let rows = listed.as_array().unwrap();
        assert!(
            !rows.is_empty(),
            "the control: {path} lists the linked items"
        );
        for r in rows {
            let item = if row.is_empty() { r } else { &r[row] };
            assert!(item.get("links").is_none(), "{path}: {item}");
            assert!(item.get("web_links").is_none(), "{path}: {item}");
        }
    }
}

async fn web_links_of(h: &Harness, key: &str) -> Value {
    api(h, &h.admin_token, &format!("/api/v1/items/{key}/links")).await["web_links"].clone()
}

#[tokio::test]
async fn web_links_are_added_listed_and_removed() {
    let h = harness().await;
    let it = items(&h).await;
    let admin = Some(h.admin_token.as_str());
    let bot = Some(h.bot_token.as_str());
    let on_epic = format!("/api/v1/items/{}/web-links", it.epic);
    let scenario = "https://example.com/wiki/scenario";

    // Added with an automation token; stored trimmed and in the parser's form; the answer is the item's web links.
    let (st, v) = call(
        &h.app,
        bot,
        "POST",
        &on_epic,
        Some(json!({"title": " Scenario page ", "url": "HTTPS://Example.com/wiki/scenario"})),
    )
    .await;
    assert_eq!(st, 200, "{v}");
    let id = v["value"][0]["id"].as_i64().expect("an id");
    assert_eq!(
        v,
        json!({"outcome": "ok", "value": [{"id": id, "title": "Scenario page", "url": scenario}]})
    );
    assert_eq!(web_links_of(&h, &it.epic).await, v["value"]);
    assert_eq!(
        api(&h, &h.admin_token, &format!("/api/v1/epics/{}", it.epic)).await["web_links"],
        v["value"],
        "the read of the item carries them"
    );
    let added = entries(&h, &it.epic, "web_link_added").await;
    assert_eq!(added.len(), 1);
    assert_eq!(added[0]["entity_type"], "epic");
    assert_eq!(added[0]["via"], "automation");
    assert_eq!(
        added[0]["payload"],
        json!({"id": id, "title": "Scenario page", "url": scenario})
    );

    // The same URL in another spelling with the same title: ok, nothing changes, nothing logged.
    let same = json!({"title": "Scenario page", "url": "https://EXAMPLE.com/wiki/scenario"});
    let (st, again) = call(&h.app, admin, "POST", &on_epic, Some(same)).await;
    assert_eq!((st, &again["value"]), (200, &v["value"]));
    // With another title: a validation error that says what to do.
    let retitled = json!({"title": "Scenario", "url": scenario});
    let (st, refused) = call(&h.app, admin, "POST", &on_epic, Some(retitled)).await;
    assert_eq!(
        (st, refused["message"].as_str()),
        (
            400,
            Some(
                "this item already has a web link to that URL, titled \"Scenario page\": remove it first, then add it with the new title"
            )
        )
    );
    assert_eq!(entries(&h, &it.epic, "web_link_added").await.len(), 1);

    // A second one lists after the first; the same URL may sit on another item, by its external reference.
    let (st, two) = call(
        &h.app,
        admin,
        "POST",
        &on_epic,
        Some(json!({"title": "Board", "url": "http://example.com/board"})),
    )
    .await;
    assert_eq!(st, 200, "{two}");
    assert_eq!(two["value"][0]["id"], id);
    assert_eq!(two["value"][1]["title"], "Board");
    sqlx::query("UPDATE stories SET external_ref = 'SCRUM-31' WHERE key = $1")
        .bind(&it.a)
        .execute(&h.state.pool)
        .await
        .unwrap();
    let (st, on_story) = call(
        &h.app,
        admin,
        "POST",
        "/api/v1/items/SCRUM-31/web-links",
        Some(json!({"title": "Scenario page", "url": scenario})),
    )
    .await;
    assert_eq!(st, 200, "{on_story}");
    let story_id = on_story["value"][0]["id"].as_i64().unwrap();
    assert_eq!(web_links_of(&h, &it.a).await, on_story["value"]);

    // Removed by id on its own item: the story's id on the epic removes nothing.
    let (st, kept) = call(
        &h.app,
        admin,
        "DELETE",
        &format!("{on_epic}/{story_id}"),
        None,
    )
    .await;
    assert_eq!(st, 200, "{kept}");
    assert_eq!(
        kept["value"].as_array().unwrap().len(),
        2,
        "nothing removed"
    );
    let (st, left) = call(&h.app, bot, "DELETE", &format!("{on_epic}/{id}"), None).await;
    assert_eq!(st, 200, "{left}");
    assert_eq!(left["value"].as_array().unwrap().len(), 1);
    assert_eq!(left["value"][0]["title"], "Board");
    let (st, _) = call(&h.app, admin, "DELETE", &format!("{on_epic}/{id}"), None).await;
    assert_eq!(st, 200, "removing one that does not exist is ok");
    let removed = entries(&h, &it.epic, "web_link_removed").await;
    assert_eq!(removed.len(), 1);
    assert_eq!(
        removed[0]["payload"],
        json!({"id": id, "title": "Scenario page", "url": scenario})
    );
    assert_eq!(web_links_of(&h, &it.a).await.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn web_link_errors_are_not_found_or_validation() {
    let h = harness().await;
    let it = items(&h).await;
    let t = Some(h.admin_token.as_str());
    let code = |(st, v): (u16, Value)| (st, v["code"].as_str().map(str::to_string));
    let not_found = (404, Some("not_found".to_string()));
    let validation = (400, Some("validation".to_string()));
    let on = |k: &str| format!("/api/v1/items/{k}/web-links");
    let good = json!({"title": "Page", "url": "https://example.com/"});

    assert_eq!(
        code(call(&h.app, None, "POST", &on(&it.a), Some(good.clone())).await).0,
        401
    );
    assert_eq!(
        code(call(&h.app, t, "POST", &on("TST-999"), Some(good.clone())).await),
        not_found
    );
    assert_eq!(
        code(call(&h.app, t, "DELETE", &format!("{}/1", on("TST-999")), None).await),
        not_found
    );

    let long_url = format!("https://example.com/{}", "a".repeat(981));
    for bad in [
        json!({"title": "", "url": "https://example.com/"}),
        json!({"title": "   ", "url": "https://example.com/"}),
        json!({"title": "t".repeat(201), "url": "https://example.com/"}),
        json!({"title": "Page", "url": "ftp://example.com/file"}),
        json!({"title": "Page", "url": "javascript:alert(1)"}),
        json!({"title": "Page", "url": "example.com/wiki"}),
        json!({"title": "Page", "url": long_url}),
        json!({"title": "Page", "url": "https://mia:secret@example.com/wiki"}),
        json!({"title": "Page", "url": "https://mia@example.com/wiki"}),
        json!({"title": "Page"}),
        json!({"title": "Page", "url": "https://example.com/", "id": 7}),
    ] {
        assert_eq!(
            code(call(&h.app, t, "POST", &on(&it.a), Some(bad.clone())).await),
            validation,
            "{bad}"
        );
    }
    // An id that is not a number is a validation error, not a missing route.
    assert_eq!(
        code(call(&h.app, t, "DELETE", &format!("{}/first", on(&it.a)), None).await),
        validation
    );
    assert_eq!(
        web_links_of(&h, &it.a).await,
        json!([]),
        "none of it wrote anything"
    );
    // Nor logged anything: a refused URL (one with a user name and a password among them) leaves no entry.
    assert!(
        entries(&h, &it.a, "web_link_added").await.is_empty(),
        "no activity entry for a refused web link"
    );
    // A control character or a line separator in a title is a validation error before any write: a NUL never
    // reaches PostgreSQL (which refuses it, and that was a 500), and an escape sequence never reaches a row a
    // terminal prints.
    for bad in [
        "Report\0",
        "Report\u{1b}[2J\u{1b}[Hfake output",
        "Report\nsecond line",
        "Report\u{2028}second line",
        "Report\tpage",
    ] {
        let (st, v) = call(
            &h.app,
            t,
            "POST",
            &on(&it.a),
            Some(json!({"title": bad, "url": "https://example.com/"})),
        )
        .await;
        assert_eq!(
            (st, v["code"].as_str(), v["message"].as_str()),
            (
                400,
                Some("validation"),
                Some(
                    "a web link's title must be 1 to 200 characters on one line, with no tab or other control character"
                )
            ),
            "{bad:?}: {v}"
        );
    }
    assert_eq!(web_links_of(&h, &it.a).await, json!([]), "no row");
    assert!(
        entries(&h, &it.a, "web_link_added").await.is_empty(),
        "no activity entry"
    );
    let (_, v) = call(
        &h.app,
        t,
        "POST",
        &on(&it.a),
        Some(json!({"title": "Page", "url": "https://mia:secret@example.com/wiki"})),
    )
    .await;
    assert_eq!(
        v["message"],
        "a web link's URL must start with http:// or https://, be at most 1000 characters and hold no user name or password"
    );
}
