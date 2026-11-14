//! Approving a request to connect (kb/ADR-0029 R4, R9): no code is returned unless the activity entry was
//! written. Compiled only with `static-oidc` (the client comes from the fake client host).
#![cfg(feature = "static-oidc")]
mod common;
use async_trait::async_trait;
use chrono::{DateTime, Duration, TimeZone, Utc};
use reeve_identity::{
    AuthError, AuthorizationRequests, ClientPolicy, CodeChallenge, FakeClientHost,
    IdentityActivity, IdentityEvent, Principal, Role, Scope, TokenKind, Via, approve_request,
    make_loopback_client_fetcher, read_request, refuse_request, resolve_client,
};
use std::sync::Mutex;

const CALLBACK: &str = "https://client.example/callback";
const RESOURCE: &str = "https://reeve.example.com/mcp";
/// RFC 7636, appendix B.
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

struct MemActivity(Mutex<Vec<IdentityEvent>>);
#[async_trait]
impl IdentityActivity for MemActivity {
    async fn record(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        e: IdentityEvent,
    ) -> Result<(), sqlx::Error> {
        self.0.lock().unwrap().push(e);
        Ok(())
    }
}

/// An activity log whose write fails.
struct Failing;
#[async_trait]
impl IdentityActivity for Failing {
    async fn record(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _e: IdentityEvent,
    ) -> Result<(), sqlx::Error> {
        Err(sqlx::Error::Protocol("boom".into()))
    }
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 14, 9, 0, 0).unwrap()
}

fn dan(token_kind: TokenKind) -> Principal {
    Principal {
        email: "dan@example.com".to_string(),
        role: Role::Member,
        token_kind,
        token_id: 7,
    }
}

/// A pending request of a client published on the fake host, read by token 7: its id, its approval value and
/// the client id. The host is returned so that it lives as long as the test.
async fn pending(requests: &AuthorizationRequests) -> (String, String, String, FakeClientHost) {
    let host = FakeClientHost::start().await;
    let client_id = host.publish("/client.json", "Example App", &[CALLBACK]);
    let policy = ClientPolicy {
        client_hosts: vec![host.host().to_string()],
        redirect_uris: vec![CALLBACK.to_string()],
    };
    let fetcher = make_loopback_client_fetcher().unwrap();
    let client = resolve_client(&*fetcher, &policy, &client_id, CALLBACK)
        .await
        .unwrap();
    let id = requests
        .open(
            &client,
            CodeChallenge::parse(CHALLENGE).unwrap(),
            RESOURCE,
            Some("s-1".to_string()),
            now(),
        )
        .unwrap();
    let approval = requests.read(&id, 7, now()).unwrap().approval;
    (id, approval, client_id, host)
}

/// Catches an entry that does not say who approved which client for what or that holds the code, a code that is
/// not the request's, and a second approval.
#[tokio::test]
async fn approving_writes_the_entry_and_returns_the_requests_code() {
    let (_db, pool) = common::pg().await;
    let requests = AuthorizationRequests::default();
    let (id, approval, client_id, _host) = pending(&requests).await;
    let who = dan(TokenKind::Interactive);
    let log = MemActivity(Mutex::new(vec![]));
    let (back, code) = approve_request(
        &pool,
        &requests,
        &log,
        &who,
        &id,
        &approval,
        Scope::Write,
        now(),
    )
    .await
    .unwrap()
    .expect("a pending request, read by this token");
    assert_eq!(
        (back.redirect_uri.as_str(), back.state.as_deref()),
        (CALLBACK, Some("s-1"))
    );
    {
        let entries = log.0.lock().unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(
            (e.action, e.actor.as_str(), e.subject.as_str(), e.at),
            (
                "connection_approved",
                "dan@example.com",
                "dan@example.com",
                now()
            )
        );
        assert!(matches!(e.via, Via::Interactive));
        assert_eq!(
            e.payload,
            serde_json::json!({"client_id": client_id, "scope": "write"})
        );
        assert!(
            !e.payload.to_string().contains(&code),
            "an entry never holds a code"
        );
    }
    let grant = requests
        .redeem(
            &code,
            &client_id,
            None,
            None,
            VERIFIER,
            now() + Duration::seconds(5),
        )
        .unwrap();
    assert_eq!(
        (grant.owner.as_str(), grant.scope, grant.approved_at),
        ("dan@example.com", Scope::Write, now())
    );
    let again = approve_request(
        &pool,
        &requests,
        &log,
        &who,
        &id,
        &approval,
        Scope::Write,
        now(),
    )
    .await
    .unwrap();
    assert!(again.is_none());
    assert_eq!(log.0.lock().unwrap().len(), 1);
}

/// kb/ADR-0029 R9: no code without its entry. Catches an approval that returns a code although the entry was
/// not written. (After the failure the request is gone and the person connects again; that the handler forgets
/// it is read in the code, not observable here: the code it held was never returned to anyone.)
#[tokio::test]
async fn an_approval_whose_entry_cannot_be_written_returns_no_code() {
    let (_db, pool) = common::pg().await;
    let requests = AuthorizationRequests::default();
    let (id, approval, _, _host) = pending(&requests).await;
    let who = dan(TokenKind::Interactive);
    let failed = approve_request(
        &pool,
        &requests,
        &Failing,
        &who,
        &id,
        &approval,
        Scope::Write,
        now(),
    )
    .await;
    assert!(matches!(failed, Err(AuthError::Db(_))), "{failed:?}");
    assert!(
        requests.read(&id, 7, now()).is_none(),
        "the request cannot be answered again"
    );
}

/// Catches an automation token that can read or answer (kb/ADR-0015 R7), and a refusal by kind that consumed the
/// request.
#[tokio::test]
async fn an_automation_token_can_neither_read_nor_approve_nor_refuse() {
    let (_db, pool) = common::pg().await;
    let requests = AuthorizationRequests::default();
    let (id, approval, _, _host) = pending(&requests).await;
    let bot = dan(TokenKind::Automation);
    let log = MemActivity(Mutex::new(vec![]));
    let approved = approve_request(
        &pool,
        &requests,
        &log,
        &bot,
        &id,
        &approval,
        Scope::Read,
        now(),
    )
    .await;
    assert!(
        matches!(approved, Err(AuthError::Forbidden)),
        "{approved:?}"
    );
    let refused = refuse_request(&requests, &bot, &id, &approval, now());
    assert!(matches!(refused, Err(AuthError::Forbidden)), "{refused:?}");
    let read = read_request(&requests, &bot, &id, now());
    assert!(matches!(read, Err(AuthError::Forbidden)), "{read:?}");
    assert!(log.0.lock().unwrap().is_empty());
    // The control: the same request, value and token id answer for an interactive token.
    let back = refuse_request(
        &requests,
        &dan(TokenKind::Interactive),
        &id,
        &approval,
        now(),
    )
    .unwrap()
    .expect("the request was not consumed");
    assert_eq!(back.redirect_uri, CALLBACK);
}
