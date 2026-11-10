//! The TUI against the real server and PostgreSQL: the flows that need the real domain rules and activity log.
mod support;
use crossterm::event::KeyCode;
use reeve_protocol::{Me, TokenKind};
use serde_json::{Value, json};
use std::time::Duration;
use support::*;

const WAIT: Duration = Duration::from_secs(10);

struct Api {
    base: String,
    token: String,
    http: reqwest::Client,
}
impl Api {
    async fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Value {
        let mut r = self
            .http
            .request(method, format!("{}/api/v1{path}", self.base))
            .bearer_auth(&self.token);
        if let Some(b) = body {
            r = r.json(&b);
        }
        let v: Value = r.send().await.unwrap().json().await.unwrap();
        assert_eq!(v["outcome"], "ok", "{path}: {v}");
        v["value"].clone()
    }
    async fn get(&self, path: &str) -> Value {
        self.call(reqwest::Method::GET, path, None).await
    }
    async fn story(&self, title: &str) -> Value {
        self.call(
            reqwest::Method::POST,
            "/stories",
            Some(
                json!({"idempotency_key": title, "title": title, "kind": "story",
            "epic": null, "description": null, "criteria": [], "labels": []}),
            ),
        )
        .await
    }
    async fn own(&self, s: &Value, email: &str) -> Value {
        let path = format!("/stories/{}", s["key"].as_str().unwrap());
        self.call(
            reqwest::Method::PATCH,
            &path,
            Some(json!({"version": s["version"], "owner": email})),
        )
        .await
    }
    async fn me(&self) -> Me {
        serde_json::from_value(self.get("/me").await).unwrap()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tui_end_to_end() {
    tokio::time::timeout(Duration::from_secs(300), flow())
        .await
        .expect("the flow must finish within five minutes");
}

/// The real server on PostgreSQL 16, as `tests/acceptance.rs` builds it (copied, so that test stays independent), with
/// a 50 ms event poll. Returns the test database (keep it bound for the whole flow), the base url, and dan's interactive and automation secrets.
async fn server() -> (reeve_testdb::TestDb, String, String, String) {
    let pg = reeve_testdb::database(&reeve_server::composed_migrator()).await;
    let url = pg.url.clone();
    let pool = pg.pool.clone();
    let config: reeve_server::ServerConfig = toml::from_str(&format!(
        "database_url = \"{url}\"\nkey_prefix = \"TUI\"\nallowed_domains = [\"example.com\"]\nadmins = [\"dan@example.com\"]\n[[repos]]\nname = \"knowledge-base\"\nadr_dir = \"knowledge/adrs\"\n[events]\npoll_ms = 50\n"
    ))
    .unwrap();
    let identity = config.identity();
    let tokens = reeve_identity::make_token_store(pool.clone());
    let activity = reeve_activity::make_activity_store(pool.clone());
    let log: std::sync::Arc<dyn reeve_identity::IdentityActivity> = activity.clone();
    let now = chrono::Utc::now();
    let mut secrets = Vec::new();
    for (kind, name) in [
        (TokenKind::Interactive, "tui"),
        (TokenKind::Automation, "bot"),
    ] {
        let t = reeve_identity::issue_token(
            &pool,
            &*tokens,
            &*log,
            &reeve_identity::AccessPolicy::floor_only(&identity),
            "test",
            reeve_protocol::Via::Interactive,
            "dan@example.com",
            kind,
            name,
            None,
            now,
        )
        .await
        .unwrap();
        secrets.push(t.secret);
    }
    let app = reeve_server::app(reeve_server::build_state(config, pool).await.unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let bot = secrets.pop().unwrap();
    let human = secrets.pop().unwrap();
    (pg, base, human, bot)
}

async fn flow() {
    let (_pg, base, human, bot) = server().await;
    let dan = Api {
        base: base.clone(),
        token: human.clone(),
        http: reqwest::Client::new(),
    };

    let s1 = dan.own(&dan.story("mine").await, "dan@example.com").await;
    let s2 = dan.own(&dan.story("theirs").await, "mia@example.com").await;
    let s3 = dan.story("not ready").await;
    let epic = dan
        .call(
            reqwest::Method::POST,
            "/epics",
            Some(json!({"idempotency_key": "e", "title": "Billing", "initiative": null, "brief": null})),
        )
        .await;

    let mut t = tui(
        &base,
        &human,
        Some(dan.me().await),
        reeve::tui::Options::default(),
    );
    let k = |v: &Value| v["key"].as_str().unwrap().to_string();

    mine_by_default(&mut t, &k(&s1), &k(&s2)).await;
    take(&mut t, &dan, &k(&s2)).await;
    override_with_a_reason(&mut t, &dan, &k(&s3)).await;
    set_epic(&mut t, &dan, &k(&s1), &k(&epic)).await;
    a_second_clients_write_arrives_live(&mut t, &dan).await;
    mark_urgent_with_a_reason(&mut t, &dan).await;
    automation_is_read_only(&base, &bot, &dan, &k(&s3)).await;
}

async fn select(t: &mut Tui, key: &str) {
    t.press(KeyCode::Esc); // clear an earlier query
    t.press(KeyCode::Char('/'));
    t.type_text(key);
    t.press(KeyCode::Enter);
    t.until(&format!("{key} selected"), WAIT, |d| {
        d.selected().as_deref() == Some(key)
    })
    .await;
}

async fn mine_by_default(t: &mut Tui, mine: &str, theirs: &str) {
    t.until("my story", WAIT, |d| d.screen().contains(mine))
        .await;
    assert!(!t.d.screen().contains(theirs), "mine is the default filter");
    t.press(KeyCode::Char('f'));
    t.press(KeyCode::Enter);
    t.press(KeyCode::Esc);
    t.until("everyone's stories", WAIT, |d| d.screen().contains(theirs))
        .await;
}

async fn take(t: &mut Tui, dan: &Api, key: &str) {
    select(t, key).await;
    t.press(KeyCode::Char('t'));
    t.until("taken", WAIT, |d| {
        d.screen().contains(&format!("{key} is yours"))
    })
    .await;
    assert_eq!(
        dan.get(&format!("/stories/{key}")).await["owner"],
        "dan@example.com"
    );
}

async fn override_with_a_reason(t: &mut Tui, dan: &Api, key: &str) {
    select(t, key).await;
    t.press(KeyCode::Char('L'));
    t.until("the override prompt", WAIT, |d| {
        d.screen().contains("override reason:")
    })
    .await;
    t.type_text("e2e override");
    t.press(KeyCode::Enter);
    t.until("moved", WAIT, |d| {
        d.screen()
            .contains(&format!("{key} → in progress (override)"))
    })
    .await;
    assert_eq!(
        dan.get(&format!("/stories/{key}")).await["delivery"],
        "in_progress"
    );
    let entries = dan
        .get(&format!("/activity?entity_key={key}&action=override"))
        .await;
    assert_eq!(entries[0]["via"], "interactive", "{entries}");
    assert_eq!(entries[0]["payload"]["reason"], "e2e override", "{entries}");
    assert_eq!(entries[0]["actor"], "dan@example.com", "{entries}");
}

async fn set_epic(t: &mut Tui, dan: &Api, key: &str, epic: &str) {
    select(t, key).await;
    t.press(KeyCode::Char('e'));
    t.until("the epic picker", WAIT, |d| {
        d.screen().contains(&format!("epic for ● {key}"))
    })
    .await;
    t.type_text("billing");
    // The picker loads the epics after it opens: Enter on a list not yet loaded picks nothing (a slow runner did).
    t.until("the epic highlighted", WAIT, |d| {
        d.screen().contains(&format!("› ■ {epic}"))
    })
    .await;
    t.press(KeyCode::Enter);
    t.until("epic set", WAIT, |d| {
        d.screen().contains(&format!("{key} → epic {epic}"))
    })
    .await;
    assert_eq!(dan.get(&format!("/stories/{key}")).await["epic"], epic);
    let s = dan.get(&format!("/stories/{key}/ready")).await;
    assert!(
        !s["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["reason"] == "untriaged"),
        "{s}"
    );
}

/// Proves the activity path, not a load that happened to be in flight: the board is quiet first, the story must
/// arrive through an activity entry (the ticker names it), and only then the card.
async fn a_second_clients_write_arrives_live(t: &mut Tui, dan: &Api) {
    t.press(KeyCode::Esc);
    t.idle_for(Duration::from_millis(1500)).await; // every earlier load and debounce has landed
    let s4 = dan
        .own(&dan.story("from elsewhere").await, "dan@example.com")
        .await;
    let key = s4["key"].as_str().unwrap().to_string();
    t.until("the activity entry in the ticker", WAIT, |d| {
        d.screen().lines().last().unwrap().contains(&key)
    })
    .await;
    t.until("the other client's story on the board", WAIT, |d| {
        d.screen()
            .lines()
            .any(|l| l.contains(&key) && !l.contains(" · dan"))
    })
    .await;
}

/// A story the TUI marks: the `!` arrives from the server's own answer (the TUI never draws it optimistically), and the
/// server holds the reason the prompt sent.
async fn mark_urgent_with_a_reason(t: &mut Tui, dan: &Api) {
    let s = dan
        .own(&dan.story("urgent e2e").await, "dan@example.com")
        .await;
    let key = s["key"].as_str().unwrap().to_string();
    t.until("the new story on the board", WAIT, |d| {
        d.screen().contains("urgent e2e")
    })
    .await;
    assert!(!t.d.screen().contains("! urgent e2e"), "not urgent yet");
    select(t, &key).await;
    t.press(KeyCode::Char('u'));
    t.until("the reason prompt", WAIT, |d| {
        d.screen().contains("reason (optional):")
    })
    .await;
    t.type_text("e2e");
    t.press(KeyCode::Enter);
    t.until("the mark before the title", WAIT, |d| {
        d.screen().contains("! urgent e2e")
    })
    .await;
    let story = dan.get(&format!("/stories/{key}")).await;
    assert_eq!(story["urgent_mark"]["reason"], "e2e", "{story}");
    assert_eq!(story["urgent_mark"]["by"], "dan@example.com", "{story}");
    assert_eq!(story["version"], s["version"], "a mark takes no version");
}

async fn automation_is_read_only(base: &str, bot: &str, dan: &Api, key: &str) {
    let bot_api = Api {
        base: base.into(),
        token: bot.into(),
        http: reqwest::Client::new(),
    };
    let mut t = tui(
        base,
        bot,
        Some(bot_api.me().await),
        reeve::tui::Options::default(),
    );
    t.until("the automation header", WAIT, |d| {
        d.screen()
            .contains("automation token — run reeve auth login")
    })
    .await;
    let before = dan.get(&format!("/stories/{key}")).await["version"].clone();
    t.press(KeyCode::Char('f'));
    t.press(KeyCode::Enter);
    t.press(KeyCode::Esc);
    select(&mut t, key).await;
    t.press(KeyCode::Char('t'));
    t.until("read-only", WAIT, |d| {
        d.screen().contains("read-only: automation token")
    })
    .await;
    assert_eq!(dan.get(&format!("/stories/{key}")).await["version"], before);
}
