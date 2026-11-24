//! Brief templates over the API (GRA-903): who may read and who may write, each route's happy path and each error,
//! which brief a new initiative or epic gets, and that a brief never follows its template afterwards.
mod common;
use common::*;
use serde_json::{Value, json};

const SCENARIO: &str = "# Scenario\n\n| Step | Who |\n|---|---|\n| 1 | |\n";
const DEBT: &str = "## Debt\n\nWhat it costs us today.\n";
const LIST: &str = "/api/v1/brief-templates";

/// Creates a brief template as the admin and answers it.
async fn template(h: &Harness, level: &str, name: &str, body: &str) -> Value {
    post(h, LIST, json!({"level": level, "name": name, "body": body})).await
}

fn one(t: &Value) -> String {
    format!("{LIST}/{}", t["id"])
}

/// The admin's request: its status and its whole answer.
async fn admin(h: &Harness, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
    call(&h.app, Some(&h.admin_token), method, path, body).await
}

/// A request that must be refused as a validation error whose message contains `says`.
async fn refused(h: &Harness, method: &str, path: &str, body: Value, says: &str) {
    let (st, b) = admin(h, method, path, Some(body.clone())).await;
    assert_eq!(st, 400, "{method} {path} {body}: {b}");
    assert_eq!(b["code"], "validation", "{b}");
    assert!(
        b["message"].as_str().unwrap().contains(says),
        "{method} {path} {body}: wanted {says:?} in {b}"
    );
}

#[tokio::test]
async fn a_member_reads_brief_templates_and_only_an_admin_writes_them() {
    let h = harness().await;
    let t = template(&h, "initiative", "Scenario", SCENARIO).await;
    let mark = format!("{}/default", one(&t));
    let member = Some(h.victim_token.as_str());

    // Every member lists and reads.
    let (st, list) = call(&h.app, member, "GET", LIST, None).await;
    assert_eq!((st, &list["value"]), (200, &json!([t])), "{list}");
    let (st, got) = call(&h.app, member, "GET", &one(&t), None).await;
    assert_eq!((st, &got["value"]), (200, &t), "{got}");

    // No member writes, by any of the five writing routes, and the refusal comes before anything is read: the
    // bodies are valid and the template exists, so only the role can be the reason.
    let writes = [
        (
            "POST",
            LIST.to_string(),
            Some(json!({"level": "epic", "name": "Mine", "body": "x"})),
        ),
        (
            "PATCH",
            one(&t),
            Some(json!({"version": 1, "name": "Mine"})),
        ),
        ("POST", mark.clone(), None),
        ("DELETE", mark.clone(), None),
        ("DELETE", one(&t), None),
    ];
    for (method, path, body) in &writes {
        let (st, b) = call(&h.app, member, method, path, body.clone()).await;
        assert_eq!(st, 403, "{method} {path}: {b}");
        assert_eq!(b["code"], "forbidden", "{method} {path}: {b}");
    }
    assert_eq!(
        api(&h, &h.admin_token, LIST).await,
        json!([t]),
        "a refused write changed nothing"
    );

    // Nobody reads without signing in.
    let (st, b) = call(&h.app, None, "GET", LIST, None).await;
    assert_eq!((st, &b["code"]), (401, &json!("unauthenticated")));

    // An admin writes with either kind of token: the bot token is the admin's automation token.
    let (st, b) = call(
        &h.app,
        Some(&h.bot_token),
        "POST",
        LIST,
        Some(json!({"level": "epic", "name": "Tech debt", "body": DEBT})),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    let (st, b) = call(&h.app, Some(&h.bot_token), "POST", &mark, None).await;
    assert_eq!((st, &b["value"]["is_default"]), (200, &json!(true)), "{b}");
}

#[tokio::test]
async fn a_brief_template_is_created_edited_marked_and_removed() {
    let h = harness().await;

    let t = template(&h, "initiative", "  Scenario ", SCENARIO).await;
    let id = t["id"].as_i64().unwrap();
    assert_eq!(
        t,
        json!({"id": id, "level": "initiative", "name": "Scenario", "body": SCENARIO, "is_default": false, "version": 1})
    );
    // The same content again answers the same template; the same name with another body is refused.
    assert_eq!(template(&h, "initiative", "scenario", SCENARIO).await, t);
    refused(
        &h,
        "POST",
        LIST,
        json!({"level": "initiative", "name": "SCENARIO", "body": "other"}),
        "already exists",
    )
    .await;
    assert_eq!(api(&h, &h.admin_token, LIST).await, json!([t]));

    // What a create refuses.
    let long = "n".repeat(101);
    for (body, says) in [
        (json!({"level": "story", "name": "S", "body": "b"}), "story"),
        (
            json!({"level": "epic", "name": " ", "body": "b"}),
            "name must be 1 to 100",
        ),
        (
            json!({"level": "epic", "name": long, "body": "b"}),
            "name must be 1 to 100",
        ),
        (
            json!({"level": "epic", "name": "two\nlines", "body": "b"}),
            "name must be 1 to 100",
        ),
        (
            json!({"level": "epic", "name": "S", "body": " \n"}),
            "body must not be blank",
        ),
        (
            json!({"level": "epic", "name": "S", "body": "b", "is_default": true}),
            "is_default",
        ),
    ] {
        refused(&h, "POST", LIST, body, says).await;
    }

    // An id nobody has is not found, by every route that takes one; a word where the id goes is a validation error.
    for (method, path, body) in [
        ("GET", format!("{LIST}/999999"), None),
        (
            "PATCH",
            format!("{LIST}/999999"),
            Some(json!({"version": 1, "name": "X"})),
        ),
        ("DELETE", format!("{LIST}/999999"), None),
        ("POST", format!("{LIST}/999999/default"), None),
        ("DELETE", format!("{LIST}/999999/default"), None),
    ] {
        let (st, b) = admin(&h, method, &path, body).await;
        assert_eq!(
            (st, &b["code"]),
            (404, &json!("not_found")),
            "{method} {path}: {b}"
        );
    }
    let (st, _) = admin(&h, "GET", &format!("{LIST}/scenario"), None).await;
    assert_eq!(st, 400);

    // An edit names the version it read, and changes only what it names.
    let (st, b) = admin(
        &h,
        "PATCH",
        &one(&t),
        Some(json!({"version": 9, "body": "x"})),
    )
    .await;
    assert_eq!(
        (st, &b["code"], &b["current"]),
        (409, &json!("conflict"), &json!(1)),
        "{b}"
    );
    let (st, b) = admin(
        &h,
        "PATCH",
        &one(&t),
        Some(json!({"version": 1, "body": "# New\n"})),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(
        b["value"],
        json!({"id": id, "level": "initiative", "name": "Scenario", "body": "# New\n", "is_default": false, "version": 2})
    );
    refused(
        &h,
        "PATCH",
        &one(&t),
        json!({"version": 2, "level": "epic"}),
        "level",
    )
    .await;

    // A rename onto a name the level has is refused; the other level's names are its own.
    let debt = template(&h, "initiative", "Tech debt", DEBT).await;
    let epic = template(&h, "epic", "Scenario", "E").await;
    refused(
        &h,
        "PATCH",
        &one(&debt),
        json!({"version": 1, "name": " scenario"}),
        "already exists",
    )
    .await;

    // The mark: one per level, moved by marking another, cleared by DELETE; both are no-ops when nothing changes.
    let defaults = |list: &Value| -> Vec<(String, String)> {
        list.as_array()
            .unwrap()
            .iter()
            .filter(|t| t["is_default"] == true)
            .map(|t| {
                (
                    t["level"].as_str().unwrap().into(),
                    t["name"].as_str().unwrap().into(),
                )
            })
            .collect()
    };
    let level = |l: &str, n: &str| (l.to_string(), n.to_string());
    for marked in [&t, &epic, &debt] {
        let (st, b) = admin(&h, "POST", &format!("{}/default", one(marked)), None).await;
        assert_eq!((st, &b["value"]["is_default"]), (200, &json!(true)), "{b}");
    }
    assert_eq!(
        defaults(&api(&h, &h.admin_token, LIST).await),
        vec![level("initiative", "Tech debt"), level("epic", "Scenario")]
    );
    let (st, b) = admin(&h, "POST", &format!("{}/default", one(&debt)), None).await;
    assert_eq!(
        (st, &b["value"]["version"]),
        (200, &json!(1)),
        "the mark changes no version: {b}"
    );
    let (st, b) = admin(&h, "DELETE", &format!("{}/default", one(&debt)), None).await;
    assert_eq!((st, &b["value"]["is_default"]), (200, &json!(false)), "{b}");
    let (st, _) = admin(&h, "DELETE", &format!("{}/default", one(&debt)), None).await;
    assert_eq!(st, 200, "clearing a mark that is not set changes nothing");
    assert_eq!(
        defaults(&api(&h, &h.admin_token, LIST).await),
        vec![level("epic", "Scenario")]
    );

    // The list: initiative first, then by name; one level on request; an unknown level is a validation error.
    let names = |list: &Value| -> Vec<String> {
        list.as_array()
            .unwrap()
            .iter()
            .map(|t| {
                format!(
                    "{} {}",
                    t["level"].as_str().unwrap(),
                    t["name"].as_str().unwrap()
                )
            })
            .collect()
    };
    assert_eq!(
        names(&api(&h, &h.admin_token, LIST).await),
        [
            "initiative Scenario",
            "initiative Tech debt",
            "epic Scenario"
        ]
    );
    assert_eq!(
        names(&api(&h, &h.admin_token, &format!("{LIST}?level=epic")).await),
        ["epic Scenario"]
    );
    let (st, _) = admin(&h, "GET", &format!("{LIST}?level=story"), None).await;
    assert_eq!(st, 400);

    // Removing: gone for every later request.
    let (st, b) = admin(&h, "DELETE", &one(&epic), None).await;
    assert_eq!(st, 200, "{b}");
    let (st, _) = admin(&h, "GET", &one(&epic), None).await;
    assert_eq!(st, 404);
    let (st, _) = admin(&h, "DELETE", &one(&epic), None).await;
    assert_eq!(st, 404);

    // Every change is in the activity log, on the template's id, and no entry holds a body.
    let mut log = api(
        &h,
        &h.admin_token,
        "/api/v1/activity?entity_type=brief_template&limit=500",
    )
    .await
    .as_array()
    .unwrap()
    .clone();
    log.sort_by_key(|e| e["id"].as_i64());
    let of = |key: &Value| -> Vec<String> {
        // The entry's key is the id as text.
        let key = key.to_string();
        log.iter()
            .filter(|e| e["entity_key"] == key)
            .map(|e| e["action"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        of(&t["id"]),
        [
            "brief_template_created",
            "brief_template_edited",
            "brief_template_default_set"
        ]
    );
    assert_eq!(
        of(&debt["id"]),
        [
            "brief_template_created",
            "brief_template_default_set",
            "brief_template_default_cleared"
        ]
    );
    assert_eq!(
        of(&epic["id"]),
        [
            "brief_template_created",
            "brief_template_default_set",
            "brief_template_removed"
        ]
    );
    let text = serde_json::to_string(&log).unwrap();
    assert!(
        !text.contains("| Step |") && !text.contains("What it costs"),
        "{text}"
    );
}

/// Creates an initiative or an epic as the admin with `fields` over a title and the idempotency key `idem`.
async fn create(h: &Harness, level: &str, idem: &str, fields: Value) -> (u16, Value) {
    let mut body = json!({"idempotency_key": idem, "title": format!("T {idem}")});
    if level == "initiative" {
        body["horizon"] = json!("Now");
    }
    body.as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    admin(h, "POST", &format!("/api/v1/{level}s"), Some(body)).await
}

/// The brief of the item such a create answers.
async fn brief_of(h: &Harness, level: &str, idem: &str, fields: Value) -> String {
    let (st, b) = create(h, level, idem, fields).await;
    assert_eq!(st, 200, "{idem}: {b}");
    b["value"]["brief"].as_str().unwrap().to_string()
}

async fn mark(h: &Harness, t: &Value) {
    post(h, &format!("{}/default", one(t)), json!({})).await;
}

/// How many initiatives or epics there are.
async fn items(h: &Harness, level: &str) -> usize {
    api(h, &h.admin_token, &format!("/api/v1/{level}s"))
        .await
        .as_array()
        .unwrap()
        .len()
}

#[tokio::test]
async fn a_new_brief_starts_from_a_template_only_when_the_create_asks() {
    let h = harness().await;
    for (level, other) in [("initiative", "epic"), ("epic", "initiative")] {
        let k = |n: &str| format!("{level}-{n}");
        let ask = || json!({"default_template": true});
        // No template at all: asking for the default gives an empty brief, not a refusal.
        assert_eq!(brief_of(&h, level, &k("none"), ask()).await, "");
        let scenario = template(&h, level, "Scenario", SCENARIO).await;
        let debt = template(&h, level, "Tech debt", DEBT).await;
        let elsewhere = format!("Only for {other}s");
        template(&h, other, &elsewhere, "OTHER").await;
        // Templates, but none marked: asking for the default still gives nothing.
        assert_eq!(brief_of(&h, level, &k("unmarked"), ask()).await, "");

        mark(&h, &scenario).await;
        // The control for everything below: the default is live for a create that asks.
        assert_eq!(brief_of(&h, level, &k("asked"), ask()).await, SCENARIO);
        // A caller that says nothing gets what it got before templates existed, in each way of saying nothing.
        for (n, fields) in [
            ("absent", json!({})),
            ("null", json!({"brief": null})),
            ("empty", json!({"brief": ""})),
            ("false", json!({"default_template": false})),
        ] {
            assert_eq!(brief_of(&h, level, &k(n), fields).await, "", "{level} {n}");
        }
        // Asked for, with "no brief" said in each way: the default. A blank brief counts as none given.
        for (n, fields) in [
            ("ask-null", json!({"brief": null, "default_template": true})),
            ("ask-empty", json!({"brief": "", "default_template": true})),
            (
                "ask-blank",
                json!({"brief": " \n", "default_template": true}),
            ),
        ] {
            assert_eq!(
                brief_of(&h, level, &k(n), fields).await,
                SCENARIO,
                "{level} {n}"
            );
        }
        // A named template, compared as names are, with a blank brief beside it or none.
        assert_eq!(
            brief_of(&h, level, &k("named"), json!({"template": " tech DEBT"})).await,
            DEBT
        );
        assert_eq!(
            brief_of(
                &h,
                level,
                &k("named-blank"),
                json!({"template": "Tech debt", "brief": " "})
            )
            .await,
            DEBT
        );
        // A brief the caller gives is the brief.
        assert_eq!(
            brief_of(&h, level, &k("mine"), json!({"brief": "Mine"})).await,
            "Mine"
        );

        // What is refused, and that a refusal creates nothing.
        let before = items(&h, level).await;
        for (n, fields, says) in [
            (
                "both",
                json!({"brief": "Mine", "template": "Scenario"}),
                "not both".to_string(),
            ),
            (
                "mine-default",
                json!({"brief": "Mine", "default_template": true}),
                "not both".to_string(),
            ),
            (
                "name-default",
                json!({"template": "Scenario", "default_template": true}),
                "exclude each other".to_string(),
            ),
            (
                "other-level",
                json!({"template": elsewhere}),
                format!("no {level} brief template is named"),
            ),
            (
                "nobody",
                json!({"template": "Nobody"}),
                format!("no {level} brief template is named"),
            ),
        ] {
            let (st, b) = create(&h, level, &k(n), fields).await;
            assert_eq!(
                (st, &b["code"]),
                (400, &json!("validation")),
                "{level} {n}: {b}"
            );
            assert!(
                b["message"].as_str().unwrap().contains(&says),
                "{level} {n}: {b}"
            );
        }
        assert_eq!(
            items(&h, level).await,
            before,
            "a refused create made nothing"
        );

        // A retry answers the item its key already has, whatever the default is now.
        assert_eq!(brief_of(&h, level, &k("retry"), ask()).await, SCENARIO);
        mark(&h, &debt).await;
        assert_eq!(brief_of(&h, level, &k("retry"), ask()).await, SCENARIO);
        assert_eq!(
            brief_of(&h, level, &k("after"), ask()).await,
            DEBT,
            "the control: the default moved"
        );
    }
}

/// The one retry the rule does not carry: a create that names a template is looked up by that name before its
/// idempotency key is. When the template was removed or renamed since the first attempt, the same request is
/// refused; the same key without the name answers the item the first attempt made.
#[tokio::test]
async fn a_retry_that_names_a_template_gone_since_is_refused_and_recovers_without_the_name() {
    let h = harness().await;
    for (level, how) in [("initiative", "removed"), ("epic", "renamed")] {
        let t = template(&h, level, "Scenario", SCENARIO).await;
        let idem = format!("{level}-{how}");
        let (st, first) = create(&h, level, &idem, json!({"template": "Scenario"})).await;
        assert_eq!(st, 200, "{first}");
        let first = first["value"].clone();
        assert_eq!(
            first["brief"], SCENARIO,
            "the control: the first attempt started from the template"
        );

        let (st, b) = if how == "removed" {
            admin(&h, "DELETE", &one(&t), None).await
        } else {
            admin(
                &h,
                "PATCH",
                &one(&t),
                Some(json!({"version": 1, "name": "Story map"})),
            )
            .await
        };
        assert_eq!(st, 200, "{how}: {b}");
        let before = items(&h, level).await;

        // The same request again: refused, because the name is nobody's now. Nothing is made.
        let (st, b) = create(&h, level, &idem, json!({"template": "Scenario"})).await;
        assert_eq!((st, &b["code"]), (400, &json!("validation")), "{how}: {b}");
        assert!(
            b["message"]
                .as_str()
                .unwrap()
                .contains(&format!("no {level} brief template is named")),
            "{how}: {b}"
        );
        // The recovery: the same key without the name answers the first attempt's item, brief and version.
        let (st, again) = create(&h, level, &idem, json!({})).await;
        assert_eq!(st, 200, "{how}: {again}");
        for field in ["key", "brief", "version"] {
            assert_eq!(again["value"][field], first[field], "{how}: {field}");
        }
        assert_eq!(
            items(&h, level).await,
            before,
            "{how}: neither the refusal nor the recovery made an item"
        );
    }
}

#[tokio::test]
async fn a_brief_outlives_the_edit_and_the_removal_of_its_template() {
    let h = harness().await;
    let t = template(&h, "initiative", "Scenario", SCENARIO).await;
    mark(&h, &t).await;
    let (st, made) = create(
        &h,
        "initiative",
        "from-default",
        json!({"default_template": true}),
    )
    .await;
    assert_eq!(st, 200, "{made}");
    let ini = made["value"].clone();
    assert_eq!(
        ini["brief"], SCENARIO,
        "the control: the brief came from the template"
    );
    assert!(
        !ini.as_object()
            .unwrap()
            .keys()
            .any(|k| k.contains("template")),
        "the item names no template: {ini}"
    );
    let item = format!("/api/v1/initiatives/{}", key(&ini));

    let (st, b) = admin(
        &h,
        "PATCH",
        &one(&t),
        Some(json!({"version": 1, "body": "# Rewritten\n"})),
    )
    .await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(
        api(&h, &h.admin_token, &item).await,
        ini,
        "editing the template changes no brief and no version"
    );
    let (st, b) = admin(&h, "DELETE", &one(&t), None).await;
    assert_eq!(st, 200, "{b}");
    assert_eq!(
        api(&h, &h.admin_token, &item).await,
        ini,
        "removing it changes none either"
    );

    // The item's own log holds its creation and nothing about a template.
    let log = api(
        &h,
        &h.admin_token,
        &format!("/api/v1/activity?entity_key={}", key(&ini)),
    )
    .await;
    let actions: Vec<&str> = log
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    assert_eq!(actions, ["created"], "{log}");
    assert!(
        !log.to_string().contains("Scenario\""),
        "the entry names no template: {log}"
    );
}

/// A template changes what the advisory finding `empty_brief` sees, and that is accepted (the spec's section 1):
/// a brief that was started from a template and never filled in is not empty, so the finding no longer says that
/// nobody wrote it.
#[tokio::test]
async fn a_brief_started_from_a_template_is_not_found_empty() {
    let h = harness().await;
    for level in ["initiative", "epic"] {
        let t = template(&h, level, "Scenario", SCENARIO).await;
        mark(&h, &t).await;
        let findings = |made: &Value| format!("/api/v1/{level}s/{}/findings", key(&made["value"]));

        let (st, bare) = create(&h, level, &format!("{level}-bare"), json!({})).await;
        assert_eq!(st, 200, "{bare}");
        assert_eq!(
            api(&h, &h.admin_token, &findings(&bare)).await,
            json!([{"finding": "empty_brief"}]),
            "{level}: the control: a create that asks for no template is found empty"
        );

        let (st, started) = create(
            &h,
            level,
            &format!("{level}-started"),
            json!({"default_template": true}),
        )
        .await;
        assert_eq!(st, 200, "{started}");
        assert_eq!(started["value"]["brief"], SCENARIO);
        assert_eq!(
            api(&h, &h.admin_token, &findings(&started)).await,
            json!([]),
            "{level}: the template's text counts as a brief, although nobody has filled it in"
        );
    }
}

#[tokio::test]
async fn a_plan_and_an_import_apply_no_template() {
    let h = harness_with(|cfg| format!("{cfg}\n[import]\nenabled = true")).await;
    for level in ["initiative", "epic"] {
        let t = template(&h, level, "Default", "FROM THE DEFAULT").await;
        mark(&h, &t).await;
        // The control: the default is live for a create that asks.
        assert_eq!(
            brief_of(
                &h,
                level,
                &format!("control-{level}"),
                json!({"default_template": true})
            )
            .await,
            "FROM THE DEFAULT"
        );
    }
    post(
        &h,
        "/api/v1/plans",
        json!({
            "id": "p",
            "initiatives": [{"name": "i", "title": "Planned initiative", "horizon": "Now"}],
            "epics": [{"name": "e", "title": "Planned epic", "initiative": "i"}]
        }),
    )
    .await;
    post(
        &h,
        "/api/v1/import/initiatives",
        json!({"external_ref": "JPD-1", "title": "Imported initiative", "brief": null, "horizon": "Now",
               "target_month": null, "owner": null, "discovery_complete": false}),
    )
    .await;
    post(
        &h,
        "/api/v1/import/epics",
        json!({"external_ref": "JPD-2", "title": "Imported epic", "initiative": null, "brief": null,
               "discovery_complete": false}),
    )
    .await;
    for (path, titles) in [
        (
            "/api/v1/initiatives",
            ["Planned initiative", "Imported initiative"],
        ),
        ("/api/v1/epics", ["Planned epic", "Imported epic"]),
    ] {
        let list = api(&h, &h.admin_token, path).await;
        for title in titles {
            let item = list
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["title"] == title)
                .unwrap_or_else(|| panic!("{title} was not created: {list}"));
            assert_eq!(item["brief"], "", "{title} took a template");
        }
    }
}
