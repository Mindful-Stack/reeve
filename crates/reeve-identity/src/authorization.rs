//! Authorization requests (kb/ADR-0029 R4, R11): one attempt to connect an MCP client, from the moment the
//! browser arrives at authorize until its code is exchanged or it expires. Held in memory, in one process
//! (kb/ADR-0016): a restart loses them and the person connects again. Every method takes `now`; nothing here
//! reads the clock (kb/ADR-0010 R4). Every method is one critical section, so the first answer to a request wins
//! and a code works once by construction.
use crate::{
    clients::ValidatedClient,
    domain::{Principal, hash_secret, random_secret},
    handler::AuthError,
    ports::{IdentityActivity, IdentityEvent},
};
use chrono::{DateTime, Duration, Utc};
use reeve_protocol::{Scope, TokenKind, Via};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

/// At most this many requests are held, codes included: a code is part of its request (kb/ADR-0029 R11).
pub const MAX_AUTHORIZATION_REQUESTS: usize = 1000;
/// How long a request lives from authorize.
const REQUEST_MINUTES: i64 = 10;
/// How long a code lives from the approval (kb/ADR-0029 R4: at most one minute).
const CODE_SECONDS: i64 = 60;

/// 43 to 128 characters of RFC 3986's unreserved set: the grammar of a PKCE verifier and of its challenge
/// (RFC 7636 sections 4.1 and 4.2).
fn is_pkce_text(text: &str) -> bool {
    (43..=128).contains(&text.len())
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~'))
}

/// A PKCE S256 code challenge. `parse` is its only constructor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeChallenge(String);

impl CodeChallenge {
    pub fn parse(text: &str) -> Option<Self> {
        is_pkce_text(text).then(|| Self(text.to_string()))
    }

    /// Whether `verifier` is a verifier (the grammar first) and the secret this challenge was made from:
    /// `BASE64URL-ENCODE(SHA256(ASCII(code_verifier))) == code_challenge`, without padding (RFC 7636 section 4.6).
    fn matches(&self, verifier: &str) -> bool {
        use base64::Engine;
        is_pkce_text(verifier)
            && base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(verifier.as_bytes()))
                == self.0
    }
}

/// What the approval page shows of a pending request, with the one-time value an answer must carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestView {
    pub client_host: String,
    pub client_name: String,
    pub redirect_uri: String,
    pub requested_at: DateTime<Utc>,
    pub approval: String,
}

/// Where an answered request sends the browser back to: the client's validated redirect address, and the
/// `state` the client sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnAddress {
    pub redirect_uri: String,
    pub state: Option<String>,
}

/// What a redeemed code grants: who approved, which client, what it may do, and when it was approved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeGrant {
    pub owner: String,
    pub client_id: String,
    pub client_name: String,
    pub scope: Scope,
    pub approved_at: DateTime<Utc>,
}

/// Why a code was not redeemed, as the token route answers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedeemRefusal {
    /// Unknown, used or expired, or presented with another client, redirect address or verifier.
    InvalidGrant,
    /// Presented for another resource.
    InvalidTarget,
}

/// Every held request has been read by someone, so none can be dropped to make room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestsFull;

enum Phase {
    Pending,
    Approved {
        code_hash: String,
        code_expires_at: DateTime<Utc>,
        owner: String,
        scope: Scope,
        approved_at: DateTime<Utc>,
    },
}

struct Entry {
    client_id: String,
    client_host: String,
    client_name: String,
    redirect_uri: String,
    challenge: CodeChallenge,
    resource: String,
    state: Option<String>,
    created_at: DateTime<Utc>,
    /// The hash of the current approval value and the id of the token it was given to; `None` until the request
    /// is read.
    approval: Option<(String, i64)>,
    phase: Phase,
}

impl Entry {
    fn expired(&self, now: DateTime<Utc>) -> bool {
        now - self.created_at >= Duration::minutes(REQUEST_MINUTES)
    }

    /// Whether nothing can come of this request any more. Pending, when its ten minutes are up; approved, when
    /// its code has expired, whatever the request's age: a code approved late keeps its full minute. (A code that
    /// was exchanged took its request with it.)
    fn lapsed(&self, now: DateTime<Utc>) -> bool {
        match &self.phase {
            Phase::Pending => self.expired(now),
            Phase::Approved {
                code_expires_at, ..
            } => *code_expires_at <= now,
        }
    }

    fn return_address(&self) -> ReturnAddress {
        ReturnAddress {
            redirect_uri: self.redirect_uri.clone(),
            state: self.state.clone(),
        }
    }

    /// Whether an answer carrying `approval` from the token whose id is `token_id` may be given now.
    fn answerable(&self, approval: &str, token_id: i64, now: DateTime<Utc>) -> bool {
        !self.expired(now)
            && matches!(self.phase, Phase::Pending)
            && self.approval.as_ref().is_some_and(|(hash, given_to)| {
                *given_to == token_id && *hash == hash_secret(approval)
            })
    }
}

/// The authorization requests of one server process. Created once in the server's `build_state`.
#[derive(Default)]
pub struct AuthorizationRequests {
    entries: Mutex<HashMap<String, Entry>>,
}

impl AuthorizationRequests {
    pub fn new() -> Self {
        Self::default()
    }

    fn entries(&self) -> MutexGuard<'_, HashMap<String, Entry>> {
        self.entries.lock().expect("the request store's lock")
    }

    /// Holds a validated request and returns its id. Expired requests, and approved ones whose code has expired,
    /// are dropped first; when the store is still full, the oldest request nobody has read is dropped; when every
    /// one has been read, `RequestsFull`.
    pub fn open(
        &self,
        client: &ValidatedClient,
        challenge: CodeChallenge,
        resource: &str,
        state: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<String, RequestsFull> {
        let mut entries = self.entries();
        entries.retain(|_, e| !e.lapsed(now));
        if entries.len() >= MAX_AUTHORIZATION_REQUESTS {
            // Unread means nobody has been handed an approval value: nobody is looking at it.
            let oldest_unread = entries
                .iter()
                .filter(|(_, e)| e.approval.is_none())
                .min_by_key(|(_, e)| e.created_at)
                .map(|(id, _)| id.clone());
            match oldest_unread {
                Some(id) => {
                    entries.remove(&id);
                }
                None => return Err(RequestsFull),
            }
        }
        let id = random_secret("");
        entries.insert(
            id.clone(),
            Entry {
                client_id: client.client_id().to_string(),
                client_host: client.client_host().to_string(),
                client_name: client.client_name().to_string(),
                redirect_uri: client.redirect_uri().to_string(),
                challenge,
                resource: resource.to_string(),
                state,
                created_at: now,
                approval: None,
                phase: Phase::Pending,
            },
        );
        Ok(id)
    }

    /// What the page shows, with a fresh approval value bound to the token whose id is `token_id`. The value given before
    /// stops working. `None` when the request is unknown, expired or answered.
    pub fn read(&self, id: &str, token_id: i64, now: DateTime<Utc>) -> Option<RequestView> {
        let mut entries = self.entries();
        let e = entries
            .get_mut(id)
            .filter(|e| !e.expired(now) && matches!(e.phase, Phase::Pending))?;
        let approval = random_secret("");
        e.approval = Some((hash_secret(&approval), token_id));
        Some(RequestView {
            client_host: e.client_host.clone(),
            client_name: e.client_name.clone(),
            redirect_uri: e.redirect_uri.clone(),
            requested_at: e.created_at,
            approval,
        })
    }

    /// Approve: checks the answer and gives the request a code for `owner` with `scope`, in one step. Returns
    /// where to send the browser, the code, and the request's client id (for the activity entry).
    pub(crate) fn approve(
        &self,
        id: &str,
        approval: &str,
        token_id: i64,
        owner: &str,
        scope: Scope,
        now: DateTime<Utc>,
    ) -> Option<(ReturnAddress, String, String)> {
        let mut entries = self.entries();
        let e = entries
            .get_mut(id)
            .filter(|e| e.answerable(approval, token_id, now))?;
        let code = random_secret("reeve_code_");
        e.phase = Phase::Approved {
            code_hash: hash_secret(&code),
            code_expires_at: now + Duration::seconds(CODE_SECONDS),
            owner: owner.to_string(),
            scope,
            approved_at: now,
        };
        Some((e.return_address(), code, e.client_id.clone()))
    }

    /// Refuse: checks the answer and removes the request, in one step. Returns where to send the browser.
    pub(crate) fn refuse(
        &self,
        id: &str,
        approval: &str,
        token_id: i64,
        now: DateTime<Utc>,
    ) -> Option<ReturnAddress> {
        let mut entries = self.entries();
        if !entries
            .get(id)
            .is_some_and(|e| e.answerable(approval, token_id, now))
        {
            return None;
        }
        entries.remove(id).map(|e| e.return_address())
    }

    /// Drops a request whatever its state: an approval whose activity entry could not be written.
    pub(crate) fn forget(&self, id: &str) {
        self.entries().remove(id);
    }

    /// The token route's first step. The code is checked before it is spent: a request with the wrong client,
    /// redirect address, resource or verifier changes nothing, and the right one can still follow. `redirect_uri`
    /// and `resource` are checked when sent (OAuth 2.1 section 10.2; RFC 8707 section 2).
    pub fn redeem(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: Option<&str>,
        resource: Option<&str>,
        verifier: &str,
        now: DateTime<Utc>,
    ) -> Result<CodeGrant, RedeemRefusal> {
        let hash = hash_secret(code);
        let mut entries = self.entries();
        let found = entries.iter().find_map(|(id, e)| match &e.phase {
            Phase::Approved {
                code_hash,
                code_expires_at,
                owner,
                scope,
                approved_at,
            } if *code_hash == hash => Some((
                id.clone(),
                *code_expires_at,
                CodeGrant {
                    owner: owner.clone(),
                    client_id: e.client_id.clone(),
                    client_name: e.client_name.clone(),
                    scope: *scope,
                    approved_at: *approved_at,
                },
            )),
            _ => None,
        });
        let Some((id, code_expires_at, grant)) = found else {
            return Err(RedeemRefusal::InvalidGrant);
        };
        let e = &entries[&id];
        // Checked first, changing nothing: someone who saw only the code cannot spend it.
        if e.client_id != client_id || redirect_uri.is_some_and(|sent| sent != e.redirect_uri) {
            return Err(RedeemRefusal::InvalidGrant);
        }
        if resource.is_some_and(|sent| sent != e.resource) {
            return Err(RedeemRefusal::InvalidTarget);
        }
        if !e.challenge.matches(verifier) {
            return Err(RedeemRefusal::InvalidGrant);
        }
        entries.remove(&id);
        if code_expires_at <= now {
            return Err(RedeemRefusal::InvalidGrant);
        }
        Ok(grant)
    }
}

/// Only an interactive token may read or answer a request: an automation token must never become an interactive
/// MCP token (kb/ADR-0015 R7), and a read hands out an approval value and marks the request read.
fn interactive(who: &Principal) -> Result<(), AuthError> {
    if who.token_kind == TokenKind::Interactive {
        Ok(())
    } else {
        Err(AuthError::Forbidden)
    }
}

/// Read, for the approval page: what it shows, with a fresh approval value bound to `who`'s token.
/// `Ok(None)`: the request is unknown, expired or answered. `Forbidden`: an automation token.
pub fn read_request(
    requests: &AuthorizationRequests,
    who: &Principal,
    id: &str,
    now: DateTime<Utc>,
) -> Result<Option<RequestView>, AuthError> {
    interactive(who)?;
    Ok(requests.read(id, who.token_id, now))
}

/// Approve (kb/ADR-0029 R4, R9): the request gets its code, the `connection_approved` entry is written and
/// committed, and only then is the code returned. If the entry cannot be written the request is forgotten and
/// the error returned: no code reaches anyone unless the entry committed, and the person connects again.
/// `Ok(None)`: the request is unknown, expired, answered, or the value or token is not the current one.
/// `Forbidden`: an automation token.
#[allow(clippy::too_many_arguments)]
pub async fn approve_request(
    pool: &PgPool,
    requests: &AuthorizationRequests,
    activity: &dyn IdentityActivity,
    who: &Principal,
    id: &str,
    approval: &str,
    scope: Scope,
    now: DateTime<Utc>,
) -> Result<Option<(ReturnAddress, String)>, AuthError> {
    interactive(who)?;
    let Some((back, code, client_id)) =
        requests.approve(id, approval, who.token_id, &who.email, scope, now)
    else {
        return Ok(None);
    };
    let written: Result<(), sqlx::Error> = async {
        let mut tx = pool.begin().await?;
        activity
            .record(
                &mut tx,
                IdentityEvent {
                    at: now,
                    actor: who.email.clone(),
                    via: Via::Interactive,
                    subject: who.email.clone(),
                    action: "connection_approved",
                    payload: serde_json::json!({"client_id": client_id, "scope": scope}),
                },
            )
            .await?;
        tx.commit().await
    }
    .await;
    if let Err(e) = written {
        requests.forget(id);
        return Err(e.into());
    }
    Ok(Some((back, code)))
}

/// Refuse: the same proofs as Approve, no entry (nothing was granted). `Ok(None)` and `Forbidden` as there.
pub fn refuse_request(
    requests: &AuthorizationRequests,
    who: &Principal,
    id: &str,
    approval: &str,
    now: DateTime<Utc>,
) -> Result<Option<ReturnAddress>, AuthError> {
    interactive(who)?;
    Ok(requests.refuse(id, approval, who.token_id, now))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const CLIENT: &str = "https://client.example/app.json";
    const CALLBACK: &str = "https://client.example/callback";
    const RESOURCE: &str = "https://reeve.example.com/mcp";
    /// RFC 7636, appendix B.
    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    const TOKEN_ID: i64 = 7;
    const DAN: &str = "dan@example.com";

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 14, 9, 0, 0).unwrap()
    }

    fn after(seconds: i64) -> DateTime<Utc> {
        t0() + Duration::seconds(seconds)
    }

    /// The S256 challenge of `verifier`, computed here: for verifiers the RFC gives no pair for.
    fn challenge_of(verifier: &str) -> String {
        use base64::Engine;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
    }

    fn open_at(store: &AuthorizationRequests, now: DateTime<Utc>) -> String {
        store
            .open(
                &ValidatedClient::for_tests(CLIENT, CALLBACK),
                CodeChallenge::parse(CHALLENGE).unwrap(),
                RESOURCE,
                Some("s-1".to_string()),
                now,
            )
            .unwrap()
    }

    /// `open`'s own answer, for a store that may be full.
    fn open_unread(
        store: &AuthorizationRequests,
        now: DateTime<Utc>,
    ) -> Result<String, RequestsFull> {
        store.open(
            &ValidatedClient::for_tests(CLIENT, CALLBACK),
            CodeChallenge::parse(CHALLENGE).unwrap(),
            RESOURCE,
            None,
            now,
        )
    }

    /// A request opened at `t0`, read by `TOKEN_ID` and approved one second later: its id and its code.
    fn approved(store: &AuthorizationRequests) -> (String, String) {
        let id = open_at(store, t0());
        let view = store.read(&id, TOKEN_ID, t0()).unwrap();
        let (_, code, _) = store
            .approve(&id, &view.approval, TOKEN_ID, DAN, Scope::Read, after(1))
            .unwrap();
        (id, code)
    }

    fn redeem_at(
        store: &AuthorizationRequests,
        code: &str,
        now: DateTime<Utc>,
    ) -> Result<CodeGrant, RedeemRefusal> {
        store.redeem(code, CLIENT, Some(CALLBACK), Some(RESOURCE), VERIFIER, now)
    }

    /// Catches a transformation other than S256 without padding (the pair is RFC 7636's own, not one this code
    /// made), a challenge that matches any verifier, a `parse` that admits a length or character the RFC does
    /// not, and a verifier accepted although it is not one: each bad verifier below is tried against **its own**
    /// challenge, so only the grammar can refuse it.
    #[test]
    fn a_challenge_matches_its_verifier_and_only_a_well_formed_one() {
        let challenge = CodeChallenge::parse(CHALLENGE).unwrap();
        assert!(challenge.matches(VERIFIER));
        assert!(!challenge.matches("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXK"));
        assert!(!challenge.matches(CHALLENGE));
        for ok in [
            "a".repeat(43),
            "a".repeat(128),
            "aA0-._~aA0-._~aA0-._~aA0-._~aA0-._~aA0-._~a".to_string(),
        ] {
            assert!(CodeChallenge::parse(&ok).is_some(), "{ok}");
            assert!(
                CodeChallenge::parse(&challenge_of(&ok))
                    .unwrap()
                    .matches(&ok),
                "the control: {ok}"
            );
        }
        for bad in [
            "a".repeat(42),
            "a".repeat(129),
            format!("{}+", "a".repeat(43)),
            format!("{} ", "a".repeat(43)),
            "é".repeat(43),
            String::new(),
        ] {
            assert!(CodeChallenge::parse(&bad).is_none(), "{bad:?}");
            let own = CodeChallenge::parse(&challenge_of(&bad))
                .expect("a SHA-256 in base64url is 43 characters");
            assert!(
                !own.matches(&bad),
                "a malformed verifier is refused even with its own challenge: {bad:?}"
            );
        }
    }

    /// Catches a request that outlives ten minutes, a read that works on an answered request, and a view that is
    /// not what was opened.
    #[test]
    fn a_request_is_read_while_pending_for_ten_minutes() {
        let store = AuthorizationRequests::new();
        let id = open_at(&store, t0());
        assert_eq!(id.len(), 48, "24 random bytes in hex: {id}");
        let view = store.read(&id, TOKEN_ID, after(599)).unwrap();
        assert_eq!(
            (
                view.client_host.as_str(),
                view.client_name.as_str(),
                view.redirect_uri.as_str(),
                view.requested_at
            ),
            ("client.example", "Example App", CALLBACK, t0())
        );
        assert!(
            store.read(&id, TOKEN_ID, after(600)).is_none(),
            "ten minutes"
        );
        assert!(store.read("unknown", TOKEN_ID, t0()).is_none());
        let (answered, _) = approved(&store);
        assert!(
            store.read(&answered, TOKEN_ID, after(2)).is_none(),
            "an approved request is not read again"
        );
    }

    /// Catches an answer accepted with a value a later read replaced, from another token, with a wrong value,
    /// after ten minutes, or a second time. The right answer still works after every refusal, so a refusal that
    /// passed because the request was consumed shows. One lock, one step: these sequential cases are the whole
    /// of "the first answer wins".
    #[test]
    fn an_answer_needs_the_current_value_from_the_token_that_read_it_and_is_given_once() {
        let store = AuthorizationRequests::new();
        let id = open_at(&store, t0());
        let try_approve = |value: &str, token_id: i64, now| {
            store.approve(&id, value, token_id, DAN, Scope::Write, now)
        };
        assert!(
            try_approve("", TOKEN_ID, t0()).is_none(),
            "nobody has read it: there is no value"
        );
        let stale = store.read(&id, TOKEN_ID, t0()).unwrap().approval;
        let current = store.read(&id, TOKEN_ID, t0()).unwrap().approval;
        assert_ne!(stale, current);
        assert!(try_approve(&stale, TOKEN_ID, t0()).is_none());
        assert!(store.refuse(&id, &stale, TOKEN_ID, t0()).is_none());
        assert!(try_approve(&current, TOKEN_ID + 1, t0()).is_none());
        assert!(store.refuse(&id, &current, TOKEN_ID + 1, t0()).is_none());
        assert!(try_approve("not the value", TOKEN_ID, t0()).is_none());
        assert!(try_approve(&current, TOKEN_ID, after(600)).is_none());
        // The right answer, once.
        let (back, code, client_id) = try_approve(&current, TOKEN_ID, after(599)).unwrap();
        assert_eq!(
            back,
            ReturnAddress {
                redirect_uri: CALLBACK.to_string(),
                state: Some("s-1".to_string())
            }
        );
        assert!(code.starts_with("reeve_code_"), "{code}");
        assert_eq!(client_id, CLIENT);
        assert!(try_approve(&current, TOKEN_ID, after(599)).is_none());
        assert!(store.refuse(&id, &current, TOKEN_ID, after(599)).is_none());
        // Refuse wins the same way, and removes the request.
        let id = open_at(&store, t0());
        let value = store.read(&id, TOKEN_ID, t0()).unwrap().approval;
        assert!(store.refuse(&id, &value, TOKEN_ID, t0()).is_some());
        assert!(
            store
                .approve(&id, &value, TOKEN_ID, DAN, Scope::Read, t0())
                .is_none()
        );
        assert!(store.refuse(&id, &value, TOKEN_ID, t0()).is_none());
    }

    /// Catches a code spent by a request that was wrong (the right one must still work afterwards), a code that
    /// works twice, a grant that is not what was approved, and a forgotten request whose code still works.
    #[test]
    fn a_code_is_checked_before_it_is_spent_and_works_once() {
        let store = AuthorizationRequests::new();
        let (_, code) = approved(&store);
        let at = after(30);
        let invalid = Err(RedeemRefusal::InvalidGrant);
        assert_eq!(redeem_at(&store, "reeve_code_unknown", at), invalid);
        assert_eq!(
            store.redeem(
                &code,
                "https://other.example/app.json",
                Some(CALLBACK),
                Some(RESOURCE),
                VERIFIER,
                at
            ),
            invalid
        );
        assert_eq!(
            store.redeem(
                &code,
                CLIENT,
                Some("https://client.example/other"),
                Some(RESOURCE),
                VERIFIER,
                at
            ),
            invalid
        );
        assert_eq!(
            store.redeem(
                &code,
                CLIENT,
                Some(CALLBACK),
                Some("https://reeve.example.com/other"),
                VERIFIER,
                at
            ),
            Err(RedeemRefusal::InvalidTarget)
        );
        assert_eq!(
            store.redeem(
                &code,
                CLIENT,
                Some(CALLBACK),
                Some(RESOURCE),
                &"a".repeat(43),
                at
            ),
            invalid
        );
        assert_eq!(
            redeem_at(&store, &code, at),
            Ok(CodeGrant {
                owner: DAN.to_string(),
                client_id: CLIENT.to_string(),
                client_name: "Example App".to_string(),
                scope: Scope::Read,
                approved_at: after(1),
            })
        );
        assert_eq!(redeem_at(&store, &code, at), invalid, "once");
        let (id, code) = approved(&store);
        store.forget(&id);
        assert_eq!(redeem_at(&store, &code, at), invalid, "forgotten");
    }

    /// Catches a token route that needs `redirect_uri` or `resource` (an OAuth 2.1 client sends no redirect
    /// address there), and a code that lives longer than a minute, and one approved late that
    /// loses part of its minute to the request's ten.
    #[test]
    fn a_code_needs_no_redirect_or_resource_and_lives_one_minute() {
        let store = AuthorizationRequests::new();
        let (_, code) = approved(&store); // approved at 1 s
        assert!(
            store
                .redeem(&code, CLIENT, None, None, VERIFIER, after(60))
                .is_ok(),
            "59 s after approval"
        );
        let (_, code) = approved(&store);
        assert_eq!(
            redeem_at(&store, &code, after(61)),
            Err(RedeemRefusal::InvalidGrant),
            "60 s after approval"
        );
        assert_eq!(
            redeem_at(&store, &code, after(30)),
            Err(RedeemRefusal::InvalidGrant),
            "an expired code is gone"
        );
        // Approved 9 min 50 s after authorize: the code keeps its full minute, past the request's ten minutes.
        let id = open_at(&store, t0());
        let value = store.read(&id, TOKEN_ID, t0()).unwrap().approval;
        let (_, code, _) = store
            .approve(&id, &value, TOKEN_ID, DAN, Scope::Read, after(590))
            .unwrap();
        assert!(
            redeem_at(&store, &code, after(649)).is_ok(),
            "59 s after a late approval"
        );
        let id = open_at(&store, t0());
        let value = store.read(&id, TOKEN_ID, t0()).unwrap().approval;
        let (_, code, _) = store
            .approve(&id, &value, TOKEN_ID, DAN, Scope::Read, after(590))
            .unwrap();
        assert_eq!(
            redeem_at(&store, &code, after(650)),
            Err(RedeemRefusal::InvalidGrant),
            "60 s after a late approval"
        );
    }

    /// Catches a full store that drops a request someone is looking at, one that refuses while an unread request
    /// could go, one that keeps expired requests, and one that keeps an approved request whose code has expired.
    #[test]
    fn a_full_store_drops_the_oldest_unread_request_and_never_a_read_one() {
        let store = AuthorizationRequests::new();
        let mut ids = vec![];
        for i in 0..MAX_AUTHORIZATION_REQUESTS {
            // One millisecond apart: each has its own instant, so "the oldest" is one request.
            ids.push(open_at(&store, t0() + Duration::milliseconds(i as i64)));
        }
        for (i, id) in ids.iter().enumerate() {
            if i != 1 && i != 2 {
                store.read(id, TOKEN_ID, after(100)).unwrap();
            }
        }
        let newest = open_at(&store, after(101));
        assert!(
            store.read(&ids[1], TOKEN_ID, after(101)).is_none(),
            "the oldest unread went"
        );
        assert!(
            store.read(&ids[0], TOKEN_ID, after(101)).is_some(),
            "a read one stays"
        );
        assert!(
            store.read(&ids[2], TOKEN_ID, after(101)).is_some(),
            "the younger unread stays"
        );
        store.read(&newest, TOKEN_ID, after(101)).unwrap();
        let full = open_unread(&store, after(102));
        assert_eq!(full, Err(RequestsFull));
        // An approval whose code has lapsed stops counting: approved at 102 s, its code ends at 162 s. One second
        // earlier the store is still full.
        let value = store.read(&newest, TOKEN_ID, after(102)).unwrap().approval;
        store
            .approve(&newest, &value, TOKEN_ID, DAN, Scope::Read, after(102))
            .unwrap();
        assert_eq!(open_unread(&store, after(161)), Err(RequestsFull));
        let room = open_unread(&store, after(162)).expect("the lapsed approval made room");
        store.read(&room, TOKEN_ID, after(162)).unwrap();
        // Ten minutes after the first thousand were opened (all within one second) they are gone: there is room.
        assert_eq!(open_at(&store, after(601)).len(), 48);
    }
}
