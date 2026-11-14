//! `POST /webhooks/github`: GitHub's pull-request events, linked to stories by branch name.
//!
//! Mounted outside the OpenAPI router (GitHub, not a Reeve client, calls it) and authenticated by the
//! `X-Hub-Signature-256` HMAC over the raw body, never by a token. Pull-request status is a cache of GitHub's state: this
//! route only records it on the story and has no path that changes a story's delivery state (kb/ADR-0002 R4).
//!
//! Answers: 401 on a missing or wrong signature; 200 to `ping`; 400 on a body that is not a pull-request event; 500
//! only on a database error (GitHub does not redeliver on its own, so it is logged with the delivery id for a manual
//! redelivery, which the upsert rules make safe); 202 for everything else, including events that change nothing.
use crate::state::AppState;
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use reeve_planning::{Ctx, HandlerError, PrChange, PullRequest};
use reeve_protocol::{Key, PrStatus, Via};
use serde::Deserialize;
use sha2::Sha256;

pub fn router() -> Router<AppState> {
    Router::new().route("/github", post(github))
}

#[derive(Debug, Deserialize)]
struct PullRequestEvent {
    action: Option<String>,
    pull_request: Option<PrPayload>,
    repository: Option<Repository>,
}
#[derive(Debug, Deserialize)]
struct PrPayload {
    number: Option<i64>,
    html_url: Option<String>,
    title: Option<String>,
    draft: Option<bool>,
    merged: Option<bool>,
    state: Option<String>,
    updated_at: Option<DateTime<Utc>>,
    head: Option<Head>,
}
#[derive(Debug, Deserialize)]
struct Head {
    #[serde(rename = "ref")]
    branch: Option<String>,
    /// Absent when GitHub no longer has the head repository (a deleted fork): the webhook then ignores the event.
    repo: Option<Repository>,
}
#[derive(Debug, Deserialize)]
struct Repository {
    full_name: Option<String>,
}

/// What a branch name refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchRef {
    /// A key of the instance prefix (`TST-12`).
    Key(Key),
    /// A key-shaped token of another prefix (`OLD-12`, `SCRUM-11`). It names the native story issued under an earlier
    /// `key_prefix` that has it (keys are retained across a prefix change), else, only when `legacy` (its prefix is in
    /// `legacy_prefixes`), the story imported from that Jira key through its `external_ref`.
    Other { token: String, legacy: bool },
}

/// The item a branch names: its last path segment (`feature/tst-12-fix` → `tst-12-fix`) must start, ignoring case, with
/// `<PREFIX>-<n>` (the prefix `[A-Z][A-Z0-9]*`), the number ending the segment or followed by a non-alphanumeric
/// character. A number has no leading zero (`TST-012` names nothing). The instance prefix gives a `Key`; any other
/// prefix gives `Other`, `legacy` when the prefix is in `legacy_prefixes`.
pub fn resolve_branch(
    branch: &str,
    key_prefix: &str,
    legacy_prefixes: &[String],
) -> Option<BranchRef> {
    let segment = branch.rsplit('/').next().unwrap_or(branch);
    let (prefix, rest) = segment.split_once('-')?;
    if !prefix.starts_with(|c: char| c.is_ascii_alphabetic())
        || !prefix.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return None;
    }
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0
        || rest.starts_with('0')
        || rest[digits..].starts_with(|c: char| c.is_ascii_alphanumeric())
    {
        return None;
    }
    let n: u64 = rest[..digits].parse().ok()?;
    let prefix = prefix.to_ascii_uppercase();
    if prefix.eq_ignore_ascii_case(key_prefix) {
        return Key::new(&prefix, n).ok().map(BranchRef::Key);
    }
    Some(BranchRef::Other {
        legacy: legacy_prefixes
            .iter()
            .any(|p| p.eq_ignore_ascii_case(&prefix)),
        token: format!("{prefix}-{n}"),
    })
}

/// Whether `X-Hub-Signature-256` is `sha256=<hex>` of the HMAC-SHA256 of `body` under `secret`, checked in constant
/// time on the decoded 32 bytes.
fn signature_ok(secret: &str, headers: &HeaderMap, body: &[u8]) -> bool {
    let Some(mac) = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("sha256="))
        .and_then(|h| hex::decode(h).ok())
        .filter(|m| m.len() == 32)
    else {
        return false;
    };
    let Ok(mut h) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    h.update(body);
    h.verify_slice(&mac).is_ok()
}

fn accepted(result: &str) -> Response {
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"ok": true, "result": result})),
    )
        .into_response()
}

fn refused(status: StatusCode, error: &str) -> Response {
    (
        status,
        Json(serde_json::json!({"ok": false, "error": error})),
    )
        .into_response()
}

async fn github(State(s): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(cfg) = &s.config.github else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let delivery = headers
        .get("x-github-delivery")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_string();
    if !signature_ok(&cfg.webhook_secret, &headers, &body) {
        tracing::warn!(%delivery, "github webhook refused: missing or wrong signature");
        return refused(StatusCode::UNAUTHORIZED, "bad signature");
    }
    match headers.get("x-github-event").and_then(|v| v.to_str().ok()) {
        Some("ping") => return Json(serde_json::json!({"ok": true})).into_response(),
        Some("pull_request") => {}
        _ => return accepted("ignored: not a pull_request event"),
    }
    let event: PullRequestEvent = match serde_json::from_slice(&body) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(%delivery, error = %e, "github webhook: unreadable pull_request payload");
            return refused(StatusCode::BAD_REQUEST, "not a pull_request event payload");
        }
    };
    let Some(repo) = event.repository.and_then(|r| r.full_name) else {
        return refused(StatusCode::BAD_REQUEST, "repository.full_name is missing");
    };
    // GitHub names are case-insensitive: the identity is `owner/name` lower-cased from here on (the allowlist check,
    // the stored row, every lookup), so one pull request spelled two ways is one row.
    let repo = repo.to_ascii_lowercase();
    if !cfg.repos.iter().any(|r| r.eq_ignore_ascii_case(&repo)) {
        tracing::info!(%delivery, %repo, "github webhook: repository not configured, ignored");
        return accepted("ignored: repository not configured");
    }
    let Some(p) = event.pull_request else {
        return refused(StatusCode::BAD_REQUEST, "pull_request is missing");
    };
    // A pull request from a fork is not trusted to name a story: anyone can open one on a public repository. A head
    // repository GitHub no longer names (a deleted fork) is not trusted either; a same-repository pull request always
    // carries it, even after its branch is deleted.
    let Some(head_repo) = p
        .head
        .as_ref()
        .and_then(|h| h.repo.as_ref())
        .and_then(|r| r.full_name.as_deref())
    else {
        tracing::info!(%delivery, %repo, "github webhook: head repository unknown, ignored");
        return accepted("ignored: head repository unknown");
    };
    if !head_repo.eq_ignore_ascii_case(&repo) {
        tracing::info!(%delivery, %repo, %head_repo, "github webhook: pull request from a fork, ignored");
        return accepted("ignored: pull request from a fork");
    }
    let (Some(number), Some(url), Some(updated_at), Some(branch)) = (
        p.number,
        p.html_url,
        p.updated_at,
        p.head.and_then(|h| h.branch),
    ) else {
        return refused(
            StatusCode::BAD_REQUEST,
            "pull_request needs number, html_url, updated_at and head.ref",
        );
    };
    let key = match resolve_branch(&branch, &s.config.key_prefix, &cfg.legacy_prefixes) {
        Some(BranchRef::Key(k)) => k,
        Some(BranchRef::Other { token, legacy }) => {
            match s.handlers.find_story_by_branch_token(&token, legacy).await {
                Ok(Some(k)) => k,
                Ok(None) if legacy => {
                    return accepted("ignored: no story imported from the branch's key");
                }
                Ok(None) => return accepted("ignored: the branch names no story"),
                Err(e) => return failed(&delivery, e),
            }
        }
        None => return accepted("ignored: the branch names no story"),
    };
    let status = if p.merged == Some(true) {
        PrStatus::Merged
    } else if p.state.as_deref() == Some("closed") {
        PrStatus::Closed
    } else if p.draft == Some(true) {
        PrStatus::Draft
    } else {
        PrStatus::Open
    };
    let ctx = Ctx {
        client: None,
        actor: "github".into(),
        via: Via::System,
        now: Utc::now(),
    };
    let pr = PullRequest {
        repo,
        number,
        url,
        title: p.title.unwrap_or_default(),
        status,
        updated_at,
    };
    match s.handlers.record_pull_request(&ctx, &key, pr).await {
        Ok(change) => {
            tracing::info!(%delivery, story = %key, number, action = ?event.action, ?change, "github webhook recorded");
            accepted(match change {
                PrChange::Linked => "linked",
                PrChange::Relinked { .. } => "relinked",
                PrChange::Updated => "updated",
                PrChange::Touched => "ignored: no visible change",
                PrChange::Ignored => "ignored: stale or repeated",
            })
        }
        Err(HandlerError::Db(e)) => failed(&delivery, HandlerError::Db(e)),
        Err(e) => {
            tracing::info!(%delivery, story = %key, number, error = %e, "github webhook: not recorded");
            accepted("ignored: no such story")
        }
    }
}

/// A database error: 500, logged with the delivery id, because GitHub will not redeliver on its own. Any other error
/// is not worth a redelivery and is answered 202.
fn failed(delivery: &str, e: HandlerError) -> Response {
    match e {
        HandlerError::Db(e) => {
            tracing::error!(%delivery, error = %e, "github webhook failed on the database; redeliver this delivery from the webhook's Recent Deliveries");
            refused(StatusCode::INTERNAL_SERVER_ERROR, "database error")
        }
        e => {
            tracing::info!(%delivery, error = %e, "github webhook: not recorded");
            accepted("ignored")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy() -> Vec<String> {
        vec!["SCRUM".into()]
    }
    fn key(s: &str) -> Option<BranchRef> {
        Some(BranchRef::Key(s.parse().unwrap()))
    }
    fn other(token: &str, legacy: bool) -> Option<BranchRef> {
        Some(BranchRef::Other {
            token: token.into(),
            legacy,
        })
    }

    #[test]
    fn resolve_branch_finds_instance_and_legacy_keys() {
        let r = |b: &str| resolve_branch(b, "TST", &legacy());
        assert_eq!(r("TST-12-fix"), key("TST-12"));
        assert_eq!(r("feature/tst-12"), key("TST-12"));
        assert_eq!(r("dan/feature/TST-7_login"), key("TST-7"));
        assert_eq!(r("TST12"), None);
        assert_eq!(r("SCRUM-11-x"), other("SCRUM-11", true));
        assert_eq!(r("scrum-11"), other("SCRUM-11", true));
        assert_eq!(r("OTHER-1"), other("OTHER-1", false));
        assert_eq!(r("old-12-fix"), other("OLD-12", false));
        assert_eq!(r("S2-4"), other("S2-4", false));
        assert_eq!(r("2X-4"), None, "a prefix starts with a letter");
        assert_eq!(r("TST-"), None);
        assert_eq!(r("TST-12abc"), None);
        assert_eq!(r("TST-0"), None, "keys are positive");
        assert_eq!(r("TST-012"), None, "keys have no leading zeros");
        assert_eq!(r("TST-012-fix"), None);
        assert_eq!(r("SCRUM-011"), None);
        assert_eq!(r("OLD-012"), None);
        assert_eq!(
            r("TST-10"),
            key("TST-10"),
            "a zero inside the number is fine"
        );
        assert_eq!(r("fix-TST-12"), None, "the key starts the segment");
        assert_eq!(r("main"), None);
        assert_eq!(r(""), None);
        assert_eq!(r("é-1"), None, "non-ASCII does not panic");
    }

    #[test]
    fn the_instance_prefix_wins_over_a_same_legacy_prefix() {
        assert_eq!(
            resolve_branch("TST-3", "TST", &["TST".into()]),
            key("TST-3")
        );
    }

    #[test]
    fn signature_is_checked_on_the_decoded_mac() {
        let body = b"{\"x\":1}";
        let mut h = Hmac::<Sha256>::new_from_slice(b"0123456789abcdef").unwrap();
        h.update(body);
        let good = format!("sha256={}", hex::encode(h.finalize().into_bytes()));
        let with = |v: &str| {
            let mut m = HeaderMap::new();
            m.insert("x-hub-signature-256", v.parse().unwrap());
            m
        };
        assert!(signature_ok("0123456789abcdef", &with(&good), body));
        assert!(signature_ok(
            "0123456789abcdef",
            &with(&good.to_uppercase().replace("SHA256=", "sha256=")),
            body
        ));
        assert!(!signature_ok(
            "0123456789abcdef",
            &with(&good),
            b"{\"x\":2}"
        ));
        assert!(!signature_ok("other-secret-0000", &with(&good), body));
        assert!(!signature_ok(
            "0123456789abcdef",
            &with(&good[..good.len() - 2]),
            body
        ));
        assert!(!signature_ok(
            "0123456789abcdef",
            &with(&good.replace("sha256=", "")),
            body
        ));
        assert!(!signature_ok("0123456789abcdef", &HeaderMap::new(), body));
    }
}
