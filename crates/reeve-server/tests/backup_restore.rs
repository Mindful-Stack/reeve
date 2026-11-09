//! The documented backup and restore commands round-trip this schema
//! (deploy/README.md, "Backup and restore"). The PostgreSQL commands run inside
//! the test container, as `docker compose exec -T postgres …` runs them in the reference stack.
mod common;
use common::*;
use hmac::{Hmac, Mac};
use reeve_server::{
    app, build_state, check_migrations, composed_migrator, export_text, pool_options,
    run_migrations,
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::time::{Duration, Instant};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{ContainerAsync, core::ExecCommand},
};

async fn migration_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn post(app: &axum::Router, token: &str, path: &str, body: Value) -> Value {
    let (st, b) = call(app, Some(token), "POST", path, Some(body)).await;
    assert_eq!(st, 200, "{path}: {b}");
    b["value"].clone()
}

fn key(v: &Value) -> String {
    v["key"].as_str().unwrap().to_string()
}

/// The keys the round trip checks after the restore.
struct Seeded {
    todo: String,
    active: String,
}

/// Every kind of row, through the API: an initiative and an epic with discovery marks, stories in todo, in progress
/// and done (`done_at` is set where the column exists), a block, a decision linked to a record with a check result,
/// a comment with an idempotency key on a story and one on the decision, a goal linked to the epic in an opened sprint,
/// a pull request through the signed webhook, and the activity of all of it. The harness already issued two users' tokens, one of them automation.
async fn seed(h: &Harness) -> Seeded {
    let (app, t) = (&h.app, h.admin_token.as_str());
    let ini = post(
        app,
        t,
        "/api/v1/initiatives",
        json!({"idempotency_key":"i1","title":"Orion","horizon":"Now"}),
    )
    .await;
    let ik = key(&ini);
    post(
        app,
        t,
        &format!("/api/v1/initiatives/{ik}/discovery"),
        json!({"version":ini["version"],"complete":true}),
    )
    .await;
    let epic = post(
        app,
        t,
        "/api/v1/epics",
        json!({"idempotency_key":"e1","title":"v2","initiative":ik}),
    )
    .await;
    let ek = key(&epic);
    post(
        app,
        t,
        &format!("/api/v1/epics/{ek}/discovery"),
        json!({"version":epic["version"],"complete":true}),
    )
    .await;
    // An urgent mark: its columns travel in the dump, and the export carries it.
    post(
        app,
        t,
        &format!("/api/v1/epics/{ek}/urgent"),
        json!({"reason":"restore me"}),
    )
    .await;
    let story = |idem: &str, title: &str| json!({"idempotency_key":idem,"title":title,"kind":"story","epic":ek,"criteria":["works"],"labels":["backup"]});
    let todo = key(&post(app, t, "/api/v1/stories", story("s-todo", "Todo")).await);
    let active = key(&post(app, t, "/api/v1/stories", story("s-active", "Active")).await);
    let done = key(&post(app, t, "/api/v1/stories", story("s-done", "Done")).await);
    // Through the gate: both ancestors discovered, one criterion, no decision, no blocker.
    post(
        app,
        t,
        &format!("/api/v1/stories/{active}/move"),
        json!({"version":1,"to":"in_progress"}),
    )
    .await;
    let moved = post(
        app,
        t,
        &format!("/api/v1/stories/{done}/move"),
        json!({"version":1,"to":"in_progress"}),
    )
    .await;
    post(
        app,
        t,
        &format!("/api/v1/stories/{done}/move"),
        json!({"version":moved["version"],"to":"done","resolution":"completed"}),
    )
    .await;
    post(
        app,
        t,
        &format!("/api/v1/stories/{todo}/blocks"),
        json!({"blocker": active}),
    )
    .await;
    let d = post(
        app,
        t,
        "/api/v1/decisions",
        json!({"idempotency_key":"d1","on":todo,"title":"Pick","decider":"dan@example.com"}),
    )
    .await;
    let id = d["id"].as_i64().unwrap();
    let linked = post(
        app,
        t,
        &format!("/api/v1/decisions/{id}/link"),
        json!({"repo":"orion","path":"docs/adr/0001.md","record_id":"ADR-0001"}),
    )
    .await;
    post(
        app,
        t,
        &format!("/api/v1/decisions/{id}/check"),
        json!({"record_id":"ADR-0001","version":linked["version"],"check":{"kind":"accepted","commit":"abc"}}),
    )
    .await;
    post(
        app,
        t,
        &format!("/api/v1/stories/{todo}/comments"),
        json!({"body":"first","idempotency_key":"c1"}),
    )
    .await;
    post(
        app,
        t,
        &format!("/api/v1/decisions/{id}/comments"),
        json!({"body":"on the decision"}),
    )
    .await;
    // A goal linked to the epic in an opened sprint (opening also creates the next draft).
    let g = post(
        app,
        t,
        "/api/v1/goals",
        json!({"idempotency_key":"g1","outcome":"Customers can book","check":"one booked"}),
    )
    .await;
    let gid = g["goal"]["id"].as_i64().unwrap();
    post(
        app,
        t,
        &format!("/api/v1/goals/{gid}/links"),
        json!({"items":[ek]}),
    )
    .await;
    let (st, sprints) = call(app, Some(t), "GET", "/api/v1/sprints", None).await;
    assert_eq!(st, 200, "{sprints}");
    let draft = &sprints["value"]["draft"];
    post(
        app,
        t,
        &format!("/api/v1/sprints/{}/open", draft["id"]),
        json!({"version":draft["version"]}),
    )
    .await;
    // A pull request through the signed webhook (fixture as in tests/webhook.rs).
    let body = include_str!("fixtures/github/pr_opened.json")
        .replace("BRANCH", &format!("{active}-work"))
        .replace("REPO", "Acme/portal")
        .replace("NUMBER", "7")
        .replace("UPDATED", "2026-10-26T10:00:00Z");
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(WEBHOOK_SECRET.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/webhooks/github")
        .header("content-type", "application/json")
        .header("x-github-event", "pull_request")
        .header("x-github-delivery", "backup-restore-1")
        .header(
            "x-hub-signature-256",
            format!("sha256={}", hex::encode(mac.finalize().into_bytes())),
        )
        .body(axum::body::Body::from(body))
        .unwrap();
    let res = call_raw(app, req).await;
    assert_eq!(res.status().as_u16(), 202);
    assert_eq!(body_json(res).await["result"], "linked");
    Seeded { todo, active }
}

/// Every table of the public schema: name, row count, md5 over its rows' text in a fixed order.
async fn table_digests(pool: &PgPool) -> Vec<(String, i64, String)> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT tablename::text FROM pg_tables WHERE schemaname = 'public' ORDER BY 1",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let mut out = Vec::new();
    for t in tables {
        let (n, digest): (i64, String) = sqlx::query_as(&format!(
            "SELECT count(*), coalesce(md5(string_agg(x::text, E'\\n' ORDER BY x::text)), '') FROM public.\"{}\" x",
            t.replace('"', "\"\"")
        ))
        .fetch_one(pool)
        .await
        .unwrap();
        out.push((t, n, digest));
    }
    out
}

/// Every sequence of the public schema with its `last_value` (NULL before first use).
async fn sequences(pool: &PgPool) -> Vec<(String, Option<i64>)> {
    sqlx::query_as(
        "SELECT sequencename::text, last_value FROM pg_sequences WHERE schemaname = 'public' ORDER BY 1",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

fn without_exported_at(d: &reeve_protocol::ExportDto) -> Value {
    let mut v = serde_json::to_value(d).unwrap();
    v.as_object_mut().unwrap().remove("exported_at").unwrap();
    v
}

/// The rehearsal script's comparison: the text without its one top-level `exported_at` line.
fn strip_at(text: &str) -> String {
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| !l.starts_with("  \"exported_at\": "))
        .collect();
    assert_eq!(kept.len() + 1, text.lines().count(), "one exported_at line");
    kept.join("\n")
}

/// What one command run in the container returned. `stdout` and `stderr` are lossy UTF-8 text for assertions on
/// messages and counts, never a binary channel: bytes that are not UTF-8 are replaced, so a dump must go through a file
/// or a pipe inside the container, not through these fields.
#[derive(Debug)]
struct Run {
    code: i64,
    stdout: String,
    stderr: String,
}

/// Runs `script` with `bash -o pipefail -c` inside the container, as the image's default user (root) over the local
/// socket, which is how `docker compose exec postgres …` reaches the server. Reads stdout and stderr to their end
/// (both streams end when the command exits), then polls the exit status, which docker may report a moment later.
/// The exec and each stream read are bounded to 120 s, the exit status to 30 s; on expiry it panics with the script.
async fn sh(c: &ContainerAsync<Postgres>, script: &str) -> Run {
    let limit = Duration::from_secs(120);
    let mut r = tokio::time::timeout(
        limit,
        c.exec(ExecCommand::new(["bash", "-o", "pipefail", "-c", script])),
    )
    .await
    .unwrap_or_else(|_| panic!("docker exec did not start within {limit:?}: {script:?}"))
    .expect("docker exec");
    let stdout = tokio::time::timeout(limit, r.stdout_to_vec())
        .await
        .unwrap_or_else(|_| panic!("stdout did not end within {limit:?}: {script:?}"))
        .unwrap();
    let stderr = tokio::time::timeout(limit, r.stderr_to_vec())
        .await
        .unwrap_or_else(|_| panic!("stderr did not end within {limit:?}: {script:?}"))
        .unwrap();
    let (stdout, stderr) = (
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let code = loop {
        if let Some(code) = r.exit_code().await.unwrap() {
            break code;
        }
        assert!(Instant::now() < deadline, "no exit status for {script:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    Run {
        code,
        stdout,
        stderr,
    }
}

#[tokio::test]
async fn the_postgres_image_dumps_and_restores_through_pipes() {
    let (c, _h) = harness_in_own_container().await;
    let c = &c;

    // The helper sees a failure and keeps the streams apart.
    let r = sh(c, "echo out; echo err >&2; exit 3").await;
    assert_eq!(
        (r.code, r.stdout.as_str(), r.stderr.as_str()),
        (3, "out\n", "err\n")
    );

    // The tools the README uses, and the client's major version.
    let r = sh(
        c,
        "for t in pg_dump pg_restore createdb dropdb psql; do command -v \"$t\" || exit 1; done; pg_dump --version",
    )
    .await;
    assert_eq!(r.code, 0, "{r:?}");
    assert!(r.stdout.contains("(PostgreSQL) 16."), "{r:?}");

    // Four tables, restored in name order (a_first, bulk, t, z_later), one sequence-backed and one holding enough data
    // that the archive is mostly table data, so cutting 200 bytes off its tail cuts into the data blocks (whichever
    // table's data pg_dump wrote last, not necessarily `bulk`), never the table of contents; through pg_dump's stdout
    // pipe and pg_restore's stdin pipe.
    let r = sh(
        c,
        "createdb -U postgres probe && \
         psql -q -v ON_ERROR_STOP=1 -U postgres -d probe -c \"CREATE TABLE a_first (id int)\" \
           -c \"CREATE TABLE t (id BIGSERIAL PRIMARY KEY, v TEXT)\" -c \"CREATE TABLE z_later (id int)\" \
           -c \"INSERT INTO t (v) VALUES ('a'), ('b')\" -c \"CREATE TABLE bulk (v text)\" \
           -c \"INSERT INTO bulk SELECT md5(g::text) FROM generate_series(1, 20000) g\" && \
         pg_dump -U postgres -d probe --format=custom --no-owner | cat > /tmp/probe.dump && \
         createdb -U postgres -O postgres probe_copy && \
         cat /tmp/probe.dump | pg_restore -U postgres -d probe_copy --no-owner --exit-on-error && \
         psql -At -U postgres -d probe_copy -c \"SELECT nextval('t_id_seq')\" \
           -c \"SELECT count(*) FROM t\" -c \"SELECT count(*) FROM bulk\"",
    )
    .await;
    assert_eq!(r.code, 0, "{r:?}");
    assert_eq!(
        r.stdout.lines().collect::<Vec<_>>(),
        ["3", "2", "20000"],
        "the sequence continues after the two restored rows; t holds 2 rows and bulk 20000"
    );

    // The README's check before anything is dropped is a full read, `pg_restore -f /dev/null` (every entry, data
    // included, into no database). `--list` reads only the header and the table of contents: an archive whose tail
    // (table data) is cut passes `--list` and fails the full read; one cut inside its header fails both.
    let r = sh(c, "pg_restore --list < /tmp/probe.dump").await;
    assert_eq!(r.code, 0, "{r:?}");
    assert!(r.stdout.contains("TABLE public z_later"), "{r:?}");
    let r = sh(c, "cat /tmp/probe.dump | pg_restore -f /dev/null").await;
    assert_eq!(r.code, 0, "the full read accepts the whole archive: {r:?}");
    let r = sh(
        c,
        "head -c $(( $(stat -c %s /tmp/probe.dump) - 200 )) /tmp/probe.dump > /tmp/cut.dump && \
         pg_restore --list < /tmp/cut.dump > /dev/null",
    )
    .await;
    assert_eq!(r.code, 0, "--list does not see a cut in the data: {r:?}");
    let r = sh(c, "cat /tmp/cut.dump | pg_restore -f /dev/null").await;
    assert_ne!(r.code, 0, "the full read does: {r:?}");
    for check in ["pg_restore --list", "pg_restore -f /dev/null"] {
        let r = sh(c, &format!("head -c 60 /tmp/probe.dump | {check}")).await;
        assert_ne!(
            r.code, 0,
            "{check} refuses an archive cut inside its header: {r:?}"
        );
    }

    // --exit-on-error stops at the first error: with `a_first` already in the target, `z_later` (restored after it)
    // must never be created. The control run without the flag shows the probe can tell the difference: PostgreSQL's
    // default continues past the error, creates `z_later`, and only reports the error count at the end.
    let trap = sh(
        c,
        "for d in probe_trap probe_ctl; do createdb -U postgres \"$d\" && \
           psql -q -v ON_ERROR_STOP=1 -U postgres -d \"$d\" -c \"CREATE TABLE a_first (id int)\" || exit 1; done; \
         cat /tmp/probe.dump | pg_restore -U postgres -d probe_trap --no-owner --exit-on-error",
    )
    .await;
    assert_ne!(trap.code, 0, "{trap:?}");
    assert!(
        trap.stderr.contains("\"a_first\" already exists"),
        "{trap:?}"
    );
    let ctl = sh(
        c,
        "cat /tmp/probe.dump | pg_restore -U postgres -d probe_ctl --no-owner",
    )
    .await;
    assert!(ctl.stderr.contains("errors ignored on restore"), "{ctl:?}");
    let later = |db: &str| {
        format!("psql -At -U postgres -d {db} -c \"SELECT to_regclass('z_later') IS NOT NULL\"")
    };
    assert_eq!(
        sh(c, &later("probe_trap")).await.stdout.trim(),
        "f",
        "the later table was not restored"
    );
    assert_eq!(
        sh(c, &later("probe_ctl")).await.stdout.trim(),
        "t",
        "without the flag it would have been"
    );
}

/// The README's commands, with this container's names (database `postgres`, user `postgres`, scratch database
/// `restored`), round-trip the schema: the restored database holds the same rows and sequences, migrates as a
/// no-op, exports the same document, and keeps tokens, idempotency keys and the key sequence working.
#[tokio::test]
async fn a_dump_restores_into_a_fresh_database_with_everything_an_export_lacks() {
    let (c, h) = harness_in_own_container().await;
    let seeded = seed(&h).await;
    let prefix = h.state.handlers.settings.key_prefix.clone();

    // 2. the offline export of the source, through the subcommand's function
    let before = reeve_server::export_document(&h.state.export_sources(), chrono::Utc::now())
        .await
        .unwrap();
    // Non-vacuity guards: the seed reached every part of the document.
    assert!(
        before
            .stories
            .iter()
            .any(|s| s.delivery == reeve_protocol::DeliveryState::Done)
    ); // sets done_at where the column exists
    assert!(before.stories.iter().any(|s| !s.prs.is_empty()));
    assert!(before.stories.iter().any(|s| !s.blocked_by.is_empty()));
    assert!(
        !before.decisions.is_empty() && !before.comments.is_empty() && !before.activity.is_empty()
    );
    assert!(before.users.len() >= 2);
    assert!(
        before
            .epics
            .iter()
            .any(|e| e.urgent_mark.as_ref().and_then(|m| m.reason.as_deref()) == Some("restore me"))
    );
    assert!(before.stories.iter().any(|s| !s.urgent_sources.is_empty()));

    // 3. the README's commands; pg_dump writes to a pipe and pg_restore reads one, as through `docker compose exec -T`
    let c = &c;
    let r = sh(
        c,
        "pg_dump -U postgres -d postgres --format=custom --no-owner | cat > /tmp/reeve.dump",
    )
    .await;
    assert_eq!(r.code, 0, "{r:?}");
    let r = sh(c, "createdb -U postgres -O postgres restored").await;
    assert_eq!(r.code, 0, "{r:?}");
    // the rehearsal script drops only what its own createdb made: createdb must refuse an existing database
    let r = sh(c, "createdb -U postgres -O postgres restored").await;
    assert_ne!(
        r.code, 0,
        "createdb on an existing database must fail: {r:?}"
    );
    let r = sh(
        c,
        "cat /tmp/reeve.dump | pg_restore -U postgres -d restored --no-owner --exit-on-error",
    )
    .await;
    assert_eq!(r.code, 0, "{r:?}");

    // 4. migrate the restored database: nothing to apply
    let restored_url: PgConnectOptions = h
        .url
        .parse::<PgConnectOptions>()
        .unwrap()
        .database("restored");
    let rpool = pool_options().connect_with(restored_url).await.unwrap();
    let applied_before = migration_count(&rpool).await;
    run_migrations(&rpool).await.unwrap();
    assert_eq!(
        migration_count(&rpool).await,
        applied_before,
        "the migrator applied nothing"
    );
    assert_eq!(
        applied_before as usize,
        composed_migrator().migrations.len()
    );
    check_migrations(&rpool).await.unwrap();

    // Row-level equality: what the export leaves out (token hashes, idempotency keys, the key sequence, done_at) too.
    let source_tables = table_digests(&h.state.pool).await;
    for (t, n, _) in &source_tables {
        assert!(
            *n > 0,
            "the seed leaves table {t} empty: extend the seed so the round trip covers it"
        );
    }
    assert_eq!(table_digests(&rpool).await, source_tables);
    assert_eq!(sequences(&rpool).await, sequences(&h.state.pool).await);

    // 5. the restored instance exports the same document
    let rstate = build_state((*h.state.config).clone(), rpool.clone())
        .await
        .unwrap();
    let after = reeve_server::export_document(&rstate.export_sources(), chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(without_exported_at(&after), without_exported_at(&before));
    // the subcommand's own path, as the rehearsal script takes it: offline, by database name
    let offline =
        reeve_server::offline_export(&h.state.config, Some("restored"), None, chrono::Utc::now())
            .await
            .unwrap();
    assert_eq!(
        strip_at(&offline),
        strip_at(&export_text(&before)),
        "the script's diff is empty"
    );

    // 6. tokens survived: both users are signed in, and the automation token is still automation
    let rapp = app(rstate.clone());
    let (st, me) = call(&rapp, Some(&h.admin_token), "GET", "/api/v1/me", None).await;
    assert_eq!(
        (st, me["value"]["email"].as_str()),
        (200, Some("dan@example.com")),
        "{me}"
    );
    let (st, _) = call(&rapp, Some(&h.victim_token), "GET", "/api/v1/me", None).await;
    assert_eq!(st, 200);
    let todo_now = after
        .stories
        .iter()
        .find(|s| s.key.to_string() == seeded.todo)
        .unwrap();
    let (st, refused) = call(
        &rapp,
        Some(&h.bot_token),
        "POST",
        &format!("/api/v1/stories/{}/move", seeded.todo),
        Some(json!({"version": todo_now.version, "to": "in_progress", "override_reason": "after the restore"})),
    )
    .await;
    assert_eq!(st, 422, "{refused}");
    assert!(
        refused["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["reason"] == "automation_cannot_override"),
        "{refused}"
    );

    // Idempotency keys survived: a retried create returns the item from before the dump, a retried comment adds none.
    let again = post(&rapp, &h.admin_token, "/api/v1/stories", json!({"idempotency_key":"s-todo","title":"Retried","kind":"story","criteria":[],"labels":[]})).await;
    assert_eq!(key(&again), seeded.todo);
    let thread = post(
        &rapp,
        &h.admin_token,
        &format!("/api/v1/stories/{}/comments", seeded.todo),
        json!({"body":"again","idempotency_key":"c1"}),
    )
    .await;
    assert_eq!(thread.as_array().unwrap().len(), 1, "{thread}");

    // 7. the key sequence survived: the next item takes the key after the last one issued before the dump
    let last: i64 = sqlx::query_scalar("SELECT last FROM key_sequence WHERE prefix = $1")
        .bind(&prefix)
        .fetch_one(&h.state.pool)
        .await
        .unwrap();
    let fresh = post(
        &rapp,
        &h.admin_token,
        "/api/v1/stories",
        json!({"idempotency_key":"s-new","title":"New","kind":"story","criteria":[],"labels":[]}),
    )
    .await;
    assert_eq!(key(&fresh), format!("{prefix}-{}", last + 1));
    // and the activity ids continue past the restored log
    let max_before = before.activity.iter().map(|e| e.id).max().unwrap();
    let newest: i64 = sqlx::query_scalar("SELECT max(id) FROM activity WHERE entity_key = $1")
        .bind(key(&fresh))
        .fetch_one(&rpool)
        .await
        .unwrap();
    assert!(newest > max_before, "{newest} > {max_before}");
    // nothing above moved a story: the webhook's pull request left `active` in progress
    let (_, s) = call(
        &rapp,
        Some(&h.admin_token),
        "GET",
        &format!("/api/v1/stories/{}", seeded.active),
        None,
    )
    .await;
    assert_eq!(s["value"]["delivery"], "in_progress");
}
