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
