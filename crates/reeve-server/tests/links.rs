//! Item links over the API (GRA-877, kb/ADR-0031): each route's happy path and each error, the forward form on
//! writes, and which reads carry `links`.
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
    // The shape only: no route writes a web link yet, so every read of one item answers an empty list.
    for path in [
        format!("/api/v1/stories/{}", it.b),
        format!("/api/v1/epics/{}", it.epic),
        format!("/api/v1/initiatives/{}", it.initiative),
    ] {
        assert_eq!(get(path.clone()).await["web_links"], json!([]), "{path}");
    }
    let context = get(format!("/api/v1/stories/{}/context", it.b)).await;
    assert_eq!(context["story"]["links"][0]["item"]["key"], json!(it.a));
    assert_eq!(context["story"]["web_links"], json!([]));
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
