//! Stored registrations (kb/ADR-0029 R3): the cap, the expiry, what the table refuses, and the two races the
//! store must win: two registrations at the cap, and an approval of the registration that is being dropped.
//! Real PostgreSQL; every instant is passed in, and nothing sleeps: a test that needs an order holds a lock and
//! watches for the waiter.
mod common;
use async_trait::async_trait;
use chrono::{DateTime, Duration, TimeZone, Utc};
use reeve_identity::{
    AuthorizationRequests, ClientDocumentFetcher, ClientPolicy, ClientRefusal, CodeChallenge,
    IdentityActivity, IdentityEvent, Principal, RegisteredClient, RegisteredClients,
    RegistrationRefusal, Role, Scope, TokenKind, approve_request, make_registered_clients,
    register_client, resolve_client,
};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};

const CALLBACK: &str = "https://client.example/callback";
const OTHER: &str = "https://other.example/cb";

/// A fixed instant in whole microseconds, so it reads back from the database as it was written.
fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 15, 9, 0, 0).unwrap()
}

fn id(n: u8) -> String {
    format!("dyn_{n:048x}")
}

fn row(n: u8, created_at: DateTime<Utc>, last_used_at: Option<DateTime<Utc>>) -> RegisteredClient {
    RegisteredClient {
        client_id: id(n),
        client_name: format!("app {n}"),
        redirect_uris: vec![CALLBACK.to_string()],
        created_at,
        last_used_at,
    }
}

async fn seed(pool: &PgPool, rows: &[RegisteredClient]) {
    for r in rows {
        sqlx::query(
            "INSERT INTO registered_clients (client_id, client_name, redirect_uris, created_at, last_used_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&r.client_id)
        .bind(&r.client_name)
        .bind(&r.redirect_uris)
        .bind(r.created_at)
        .bind(r.last_used_at)
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn ids(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT client_id FROM registered_clients ORDER BY 1")
        .fetch_all(pool)
        .await
        .unwrap()
}

async fn last_used(pool: &PgPool, client_id: &str) -> Option<DateTime<Utc>> {
    sqlx::query_scalar("SELECT last_used_at FROM registered_clients WHERE client_id = $1")
        .bind(client_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// `insert_capped` in a transaction of its own, committed whatever it answers (as the use case does).
async fn insert(
    pool: &PgPool,
    store: &dyn RegisteredClients,
    client: &RegisteredClient,
    cap: i64,
) -> bool {
    let mut tx = pool.begin().await.unwrap();
    let stored = store.insert_capped(&mut tx, client, cap).await.unwrap();
    tx.commit().await.unwrap();
    stored
}

/// Waits until `n` backends of this test's database wait for a lock, or every one of `tasks` has finished
/// (a store that takes no lock never waits: the test then goes on and fails on what it finds). Bounded.
async fn until_waiting<T>(pool: &PgPool, n: i64, tasks: &[&tokio::task::JoinHandle<T>]) {
    let waiting = async {
        loop {
            let now: i64 = sqlx::query_scalar(
                "SELECT count(DISTINCT l.pid) FROM pg_locks l JOIN pg_stat_activity a ON a.pid = l.pid WHERE NOT l.granted AND a.datname = current_database()",
            )
            .fetch_one(pool)
            .await
            .unwrap();
            if now >= n || tasks.iter().all(|task| task.is_finished()) {
                break;
            }
            tokio::task::yield_now().await;
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), waiting)
        .await
        .expect("neither a waiter nor an answer within ten seconds");
}

/// The task's answer, or a failed test if it gives none within ten seconds (a task that waits for a lock it is
/// never granted).
async fn joined<T>(task: tokio::task::JoinHandle<T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(10), task)
        .await
        .expect("no answer within ten seconds")
        .unwrap()
}

fn listing(addresses: &[&str]) -> ClientPolicy {
    ClientPolicy {
        client_hosts: vec![],
        redirect_uris: addresses.iter().map(|a| a.to_string()).collect(),
    }
}

/// Catches a column that is not stored or not read, addresses that are reordered, and a lookup that answers
/// another row.
#[tokio::test]
async fn a_stored_registration_reads_back_as_it_was_stored() {
    let (_db, pool) = common::pg().await;
    let store = make_registered_clients(pool.clone());
    let mut client = row(1, t0(), None);
    client.redirect_uris = vec![OTHER.to_string(), CALLBACK.to_string()];
    assert!(insert(&pool, &*store, &client, 1000).await);
    assert_eq!(store.find(&id(1)).await.unwrap(), Some(client));
    assert_eq!(store.find(&id(2)).await.unwrap(), None);
}

/// Catches an expiry that is off at either boundary, one that judges a used registration by when it was made,
/// and a delete that disagrees with `RegisteredClient::expired` (the resolver's rule) about any row.
#[tokio::test]
async fn making_a_registration_deletes_the_expired_ones_and_no_other() {
    let (_db, pool) = common::pg().await;
    let store = make_registered_clients(pool.clone());
    let (day, second) = (Duration::hours(24), Duration::seconds(1));
    let long_ago = t0() - Duration::days(200);
    let seeded = [
        row(1, t0() - day, None),
        row(2, t0() - day + second, None),
        row(3, long_ago, Some(t0() - Duration::days(90))),
        row(4, long_ago, Some(t0() - Duration::days(90) + second)),
    ];
    seed(&pool, &seeded).await;
    assert!(insert(&pool, &*store, &row(9, t0(), None), 1000).await);
    let kept = ids(&pool).await;
    assert_eq!(kept, [id(2), id(4), id(9)]);
    for r in &seeded {
        assert_eq!(
            !kept.contains(&r.client_id),
            r.expired(t0()),
            "the store and `expired` agree on {}",
            r.client_name
        );
    }
}

/// Catches a cap that is not held, one that drops the newest or a used registration, and one that drops more
/// than one.
#[tokio::test]
async fn a_full_store_drops_the_oldest_never_used_registration() {
    let (_db, pool) = common::pg().await;
    let store = make_registered_clients(pool.clone());
    let hour = Duration::hours(1);
    seed(
        &pool,
        &[
            row(1, t0() - hour * 3, Some(t0() - hour * 2)),
            row(2, t0() - hour * 2, None),
            row(3, t0() - hour, None),
        ],
    )
    .await;
    assert!(insert(&pool, &*store, &row(9, t0(), None), 3).await);
    assert_eq!(ids(&pool).await, [id(1), id(3), id(9)]);
}

/// Catches a cap that drops a registration a person approved, and a refusal that stores the new one anyway.
#[tokio::test]
async fn a_store_full_of_used_registrations_refuses_and_drops_none() {
    let (_db, pool) = common::pg().await;
    let store = make_registered_clients(pool.clone());
    let used = Some(t0() - Duration::hours(1));
    seed(
        &pool,
        &[
            row(1, t0() - Duration::hours(3), used),
            row(2, t0() - Duration::hours(2), used),
            row(3, t0() - Duration::hours(1), used),
        ],
    )
    .await;
    assert!(!insert(&pool, &*store, &row(9, t0(), None), 3).await);
    assert_eq!(ids(&pool).await, [id(1), id(2), id(3)]);
}

/// Catches a mark that is not written, one that says it marked a registration that is gone or makes that row
/// again, and a time of last use that moves back when two approvals commit out of the order their instants were
/// read: the 90 days then run from the older one.
#[tokio::test]
async fn an_approval_marks_a_registration_used_and_its_last_use_never_moves_back() {
    let (_db, pool) = common::pg().await;
    let store = make_registered_clients(pool.clone());
    seed(&pool, &[row(1, t0(), None)]).await;
    let minutes = Duration::minutes;
    let mut tx = pool.begin().await.unwrap();
    assert!(
        store
            .mark_used(&mut tx, &id(1), t0() + minutes(10))
            .await
            .unwrap()
    );
    assert!(
        !store
            .mark_used(&mut tx, &id(255), t0() + minutes(10))
            .await
            .unwrap(),
        "no such registration"
    );
    tx.commit().await.unwrap();
    assert_eq!(last_used(&pool, &id(1)).await, Some(t0() + minutes(10)));
    assert_eq!(ids(&pool).await, [id(1)], "the unknown id made no row");

    // The approval whose instant was read first commits second.
    let mut tx = pool.begin().await.unwrap();
    assert!(
        store
            .mark_used(&mut tx, &id(1), t0() + minutes(5))
            .await
            .unwrap()
    );
    tx.commit().await.unwrap();
    let newest = t0() + minutes(10);
    assert_eq!(last_used(&pool, &id(1)).await, Some(newest));
    // The 90 days run from the newest use.
    let stored = store.find(&id(1)).await.unwrap().unwrap();
    let ninety = Duration::days(90);
    assert!(!stored.expired(newest + ninety - Duration::seconds(1)));
    assert!(stored.expired(newest + ninety));
    assert!(
        insert(
            &pool,
            &*store,
            &row(8, newest + ninety - Duration::seconds(1), None),
            1000
        )
        .await
    );
    assert_eq!(ids(&pool).await, [id(1), id(8)], "one second short");
    assert!(insert(&pool, &*store, &row(9, newest + ninety, None), 1000).await);
    assert_eq!(
        ids(&pool).await,
        [id(8), id(9)],
        "ninety days after the newest use"
    );
}

/// Catches a table that would hold a metadata document's client id, or a registration without an address.
#[tokio::test]
async fn the_table_refuses_an_id_that_is_a_url_and_a_list_of_no_addresses() {
    let (_db, pool) = common::pg().await;
    let insert = |client_id: &'static str, uris: Vec<String>| {
        let pool = pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO registered_clients (client_id, client_name, redirect_uris, created_at) VALUES ($1, 'x', $2, $3)",
            )
            .bind(client_id)
            .bind(uris)
            .bind(t0())
            .execute(&pool)
            .await
        }
    };
    let code = |e: sqlx::Error| {
        e.as_database_error()
            .and_then(|d| d.code())
            .map(|c| c.into_owned())
            .unwrap_or_default()
    };
    let url = insert("https://claude.ai/client.json", vec![CALLBACK.to_string()])
        .await
        .unwrap_err();
    assert_eq!(code(url), "23514");
    let empty = insert(
        "dyn_000000000000000000000000000000000000000000000001",
        vec![],
    )
    .await
    .unwrap_err();
    assert_eq!(code(empty), "23514");
    assert!(ids(&pool).await.is_empty());
}

/// Catches an id that is reused or not of the registered shape, a registration stored as used, a clock other
/// than the one passed in, and an address the instance does not list reaching the table.
#[tokio::test]
async fn registering_stores_only_listed_addresses_and_makes_a_new_id_each_time() {
    let (_db, pool) = common::pg().await;
    let store = make_registered_clients(pool.clone());
    let policy = listing(&[CALLBACK]);
    let body = format!(r#"{{"client_name":"Example Assistant","redirect_uris":["{CALLBACK}"]}}"#);
    let mut made = vec![];
    for _ in 0..2 {
        let client = register_client(&pool, &*store, &policy, body.as_bytes(), t0())
            .await
            .unwrap();
        let hex = client.client_id.strip_prefix("dyn_").expect("the prefix");
        assert!(
            hex.len() == 48 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
            "{}",
            client.client_id
        );
        assert_eq!(
            (
                client.created_at,
                client.last_used_at,
                &client.redirect_uris
            ),
            (t0(), None, &vec![CALLBACK.to_string()])
        );
        assert_eq!(
            store.find(&client.client_id).await.unwrap(),
            Some(client.clone())
        );
        made.push(client.client_id);
    }
    assert_ne!(made[0], made[1]);
    let both = format!(r#"{{"client_name":"x","redirect_uris":["{CALLBACK}","{OTHER}"]}}"#);
    let refused = register_client(&pool, &*store, &policy, both.as_bytes(), t0()).await;
    assert!(
        matches!(
            &refused,
            Err(RegistrationRefusal::InvalidRedirectUri { host: Some(host), path: Some(path) })
                if host == "other.example" && path == "/cb"
        ),
        "{refused:?}"
    );
    made.sort();
    assert_eq!(ids(&pool).await, made, "the refused one stored nothing");
}

/// Two registrations at once at one place short of the cap. The test holds the store's lock, so both are known
/// to wait for it; made one at a time, the second finds the store full and drops the first (the oldest never
/// used), and the cap holds. Catches a store that counts and inserts without the lock: both then see a free
/// place, and the table holds one more than the cap.
#[tokio::test]
async fn two_registrations_at_the_cap_are_made_one_at_a_time() {
    let (_db, pool) = common::pg().await;
    let store = make_registered_clients(pool.clone());
    let used = Some(t0() - Duration::hours(1));
    seed(
        &pool,
        &[
            row(1, t0() - Duration::hours(2), used),
            row(2, t0() - Duration::hours(2), used),
        ],
    )
    .await;
    let mut held = pool.begin().await.unwrap();
    // The adapter's own lock (`insert_capped` in `registrations_repo.rs`).
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('reeve_registered_clients'))")
        .execute(&mut *held)
        .await
        .unwrap();
    let start = |n: u8| {
        let (pool, store): (PgPool, Arc<dyn RegisteredClients>) = (pool.clone(), store.clone());
        tokio::spawn(async move { insert(&pool, &*store, &row(n, t0(), None), 3).await })
    };
    let (first, second) = (start(8), start(9));
    until_waiting(&pool, 2, &[&first, &second]).await;
    assert!(
        !first.is_finished() && !second.is_finished(),
        "both registrations wait for the store's lock"
    );
    held.commit().await.unwrap();
    let outcomes = (joined(first).await, joined(second).await);
    assert_eq!(outcomes, (true, true), "both are stored");
    let kept = ids(&pool).await;
    assert_eq!(kept.len(), 3, "the cap: {kept:?}");
    assert_eq!(kept[..2], [id(1), id(2)], "the used ones stay");
}

/// A registration is being dropped to make room while a person approves a connection for the oldest never-used
/// one: the approval's transaction (held open by the test) has marked that row. The eviction skips the row being
/// approved and drops the next never-used one, without waiting for the approval, so nobody is refused while a
/// never-used registration remains; the approval then commits and its row stays, used. Catches an eviction that
/// waits for the row (the bounded join fails), one that refuses the newcomer, and one that drops the approved row.
#[tokio::test]
async fn an_approval_that_commits_while_the_eviction_runs_keeps_its_registration() {
    let (_db, pool) = common::pg().await;
    let store = make_registered_clients(pool.clone());
    seed(
        &pool,
        &[
            row(1, t0() - Duration::hours(3), None),
            row(2, t0() - Duration::hours(2), None),
            row(3, t0() - Duration::hours(1), Some(t0())),
        ],
    )
    .await;
    let mut approval = pool.begin().await.unwrap();
    assert!(store.mark_used(&mut approval, &id(1), t0()).await.unwrap());
    let eviction = {
        let (pool, store) = (pool.clone(), store.clone());
        tokio::spawn(async move { insert(&pool, &*store, &row(9, t0(), None), 3).await })
    };
    // Joined while the approval is still open: an eviction that waited for its row would not answer.
    assert!(joined(eviction).await, "the newcomer is stored");
    approval.commit().await.unwrap();
    assert_eq!(ids(&pool).await, [id(1), id(3), id(9)], "the next never-used one went");
    assert_eq!(last_used(&pool, &id(1)).await, Some(t0()));
}

/// A registered client is never fetched: resolving one with this fetcher proves it.
struct Never;
#[async_trait]
impl ClientDocumentFetcher for Never {
    async fn fetch(&self, _client_id: &url::Url) -> Result<Vec<u8>, ClientRefusal> {
        unreachable!("a registered client causes no fetch")
    }
}

/// An activity log in memory; `fail` makes its write fail.
struct Recorder {
    entries: Mutex<Vec<IdentityEvent>>,
    fail: bool,
}

impl Recorder {
    fn new(fail: bool) -> Self {
        Self {
            entries: Mutex::new(vec![]),
            fail,
        }
    }
}

#[async_trait]
impl IdentityActivity for Recorder {
    async fn record(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        e: IdentityEvent,
    ) -> Result<(), sqlx::Error> {
        if self.fail {
            return Err(sqlx::Error::Protocol("boom".into()));
        }
        self.entries.lock().unwrap().push(e);
        Ok(())
    }
}

/// RFC 7636, appendix B.
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
const TOKEN_ID: i64 = 7;

fn dan() -> Principal {
    Principal {
        email: "dan@example.com".to_string(),
        role: Role::Member,
        token_kind: TokenKind::Interactive,
        token_id: TOKEN_ID,
    }
}

/// A client registered at `t0` for `CALLBACK` under `name`.
async fn registered(pool: &PgPool, store: &dyn RegisteredClients, name: &str) -> String {
    let body = format!(r#"{{"client_name":"{name}","redirect_uris":["{CALLBACK}"]}}"#);
    register_client(pool, store, &listing(&[CALLBACK]), body.as_bytes(), t0())
        .await
        .unwrap()
        .client_id
}

/// A pending request of the registered client `client_id`, read by `dan`'s token: its id and approval value.
async fn pending(
    requests: &AuthorizationRequests,
    store: &dyn RegisteredClients,
    client_id: &str,
) -> (String, String) {
    let client = resolve_client(
        &Never,
        Some(store),
        &listing(&[CALLBACK]),
        client_id,
        CALLBACK,
        t0(),
    )
    .await
    .unwrap();
    let challenge = CodeChallenge::parse(CHALLENGE).unwrap();
    let id = requests
        .open(
            &client,
            challenge,
            "https://reeve.example.com/mcp",
            None,
            t0(),
        )
        .unwrap();
    let approval = requests.read(&id, TOKEN_ID, t0()).unwrap().approval;
    (id, approval)
}

/// Catches an approval that does not mark its registration (the cap could then drop it), one that marks
/// another, a mark that outlives an entry that could not be written, and an entry that names a registered
/// client by its id alone: the id says nothing to a reader, so the entry also holds the host the person was
/// shown and the name the app gave itself.
#[tokio::test]
async fn approving_marks_a_registered_client_used_and_no_other() {
    let (_db, pool) = common::pg().await;
    let store = make_registered_clients(pool.clone());
    let requests = AuthorizationRequests::default();
    let approved = registered(&pool, &*store, "Example Assistant").await;
    let other = registered(&pool, &*store, "Another").await;
    let at = t0() + Duration::minutes(5);

    // The entry cannot be written: nothing is marked, and no code is returned.
    let (id, approval) = pending(&requests, &*store, &approved).await;
    let failed = approve_request(
        &pool,
        &requests,
        &*store,
        &Recorder::new(true),
        &dan(),
        &id,
        &approval,
        Scope::Write,
        at,
    )
    .await;
    assert!(failed.is_err(), "{failed:?}");
    assert_eq!(last_used(&pool, &approved).await, None);

    let (id, approval) = pending(&requests, &*store, &approved).await;
    let log = Recorder::new(false);
    let answer = approve_request(
        &pool,
        &requests,
        &*store,
        &log,
        &dan(),
        &id,
        &approval,
        Scope::Write,
        at,
    )
    .await
    .unwrap();
    assert!(answer.is_some());
    assert_eq!(last_used(&pool, &approved).await, Some(at));
    assert_eq!(last_used(&pool, &other).await, None, "no other is marked");
    let entries = log.entries.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, "connection_approved");
    assert_eq!(
        entries[0].payload,
        serde_json::json!({
            "client_id": approved,
            "client_host": "client.example",
            "client_name": "Example Assistant",
            "scope": "write",
        })
    );
}

/// The two orders of an approval and an eviction. The approval first: the registration is used, and a full store
/// then refuses the newcomer and keeps it. The eviction first (the person had the approval page open while the
/// registration was dropped): the approval fails cleanly, with no code, no entry and the request forgotten, and
/// the registration is not made again outside the cap. Catches an approval that answers a code for a client
/// that no longer exists, and one that recreates the row.
#[tokio::test]
async fn an_approval_and_an_eviction_in_either_order_leave_no_half_state() {
    let (_db, pool) = common::pg().await;
    let store = make_registered_clients(pool.clone());
    let requests = AuthorizationRequests::default();
    let log = Recorder::new(false);
    let approve = |id: String, approval: String| {
        let (pool, store) = (pool.clone(), store.clone());
        let (requests, log) = (&requests, &log);
        async move {
            approve_request(
                &pool,
                requests,
                &*store,
                log,
                &dan(),
                &id,
                &approval,
                Scope::Read,
                t0(),
            )
            .await
            .unwrap()
        }
    };

    // The approval wins: a store that is full with it keeps it.
    let first = registered(&pool, &*store, "First").await;
    let (id, approval) = pending(&requests, &*store, &first).await;
    assert!(approve(id, approval).await.is_some());
    assert!(!insert(&pool, &*store, &row(9, t0(), None), 1).await);
    assert_eq!(ids(&pool).await, std::slice::from_ref(&first));
    assert_eq!(log.entries.lock().unwrap().len(), 1);

    // The eviction wins: the newcomer takes the place of a registration whose approval page is open.
    let second = registered(&pool, &*store, "Second").await;
    let (id, approval) = pending(&requests, &*store, &second).await;
    assert!(
        insert(
            &pool,
            &*store,
            &row(9, t0() + Duration::seconds(1), None),
            2
        )
        .await
    );
    let mut kept = [first, id_of(9)];
    kept.sort();
    assert_eq!(ids(&pool).await, kept, "the never-used one went");
    assert!(approve(id.clone(), approval.clone()).await.is_none());
    assert_eq!(
        log.entries.lock().unwrap().len(),
        1,
        "no entry for the failed approval"
    );
    assert_eq!(ids(&pool).await, kept, "the row is not made again");
    assert!(
        requests.read(&id, TOKEN_ID, t0()).is_none(),
        "the request is forgotten: the person connects again"
    );
}

fn id_of(n: u8) -> String {
    id(n)
}
