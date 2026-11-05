//! A read-only Jira Cloud REST v3 client: JQL search paged by `nextPageToken`, and issue comments paged by
//! `startAt`. A page that still fails after one retry aborts the run: a partial fetch is never imported.

use chrono::{DateTime, Utc};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Deserializer, de::DeserializeOwned};
use serde_json::Value;
use std::{collections::HashMap, time::Duration};

const PAGE_SIZE: u32 = 100;
/// A guard against a server that never says the last page: 1 000 000 issues at 100 a page.
const MAX_SEARCH_PAGES: usize = 10_000;
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);
/// The wait before the one retry when the response names none.
const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(1);

/// The standard fields every search requests; the mapping's custom field ids are appended (see [`search_fields`]).
pub const BASE_FIELDS: [&str; 10] = [
    "summary",
    "description",
    "status",
    "resolution",
    "issuetype",
    "assignee",
    "labels",
    "parent",
    "created",
    "issuelinks",
];

/// The top-level field names to request: the standard ones and the mapping's bucket and target-date fields.
pub fn search_fields(bucket_field: &str, target_date_field: &str) -> Vec<String> {
    BASE_FIELDS
        .iter()
        .map(|f| f.to_string())
        .chain([bucket_field.to_string(), target_date_field.to_string()])
        .collect()
}

#[derive(Debug, Clone, Deserialize)]
pub struct Issue {
    pub key: String,
    pub fields: IssueFields,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IssueFields {
    #[serde(default, deserialize_with = "null_default")]
    pub summary: String,
    #[serde(default)]
    pub description: Option<Value>,
    #[serde(default)]
    pub status: Option<Named>,
    #[serde(default)]
    pub resolution: Option<Named>,
    #[serde(default)]
    pub issuetype: Option<Named>,
    #[serde(default)]
    pub assignee: Option<User>,
    #[serde(default, deserialize_with = "null_default")]
    pub labels: Vec<String>,
    #[serde(default)]
    pub parent: Option<KeyOnly>,
    /// Requested and parsed (the fixtures check the `+0000` form) but not imported: a Reeve item is dated by its
    /// import, and its Jira provenance is the external reference.
    #[allow(dead_code)]
    #[serde(default, deserialize_with = "opt_timestamp")]
    pub created: Option<DateTime<Utc>>,
    #[serde(default, deserialize_with = "null_default")]
    pub issuelinks: Vec<IssueLink>,
    /// Every other field, custom fields included: read leniently by id, so an unexpected shape can never fail the
    /// page.
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl IssueFields {
    /// A custom field by id; `None` when absent or null.
    pub fn custom(&self, id: &str) -> Option<&Value> {
        self.extra.get(id).filter(|v| !v.is_null())
    }
    pub fn status_name(&self) -> Option<&str> {
        self.status.as_ref().map(|s| s.name.as_str())
    }
    pub fn type_name(&self) -> Option<&str> {
        self.issuetype.as_ref().map(|s| s.name.as_str())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Named {
    #[serde(default, deserialize_with = "null_default")]
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KeyOnly {
    pub key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    #[serde(default, deserialize_with = "null_default")]
    pub account_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub email_address: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueLink {
    #[serde(rename = "type", default)]
    pub link_type: Named,
    #[serde(default)]
    pub inward_issue: Option<KeyOnly>,
    #[serde(default)]
    pub outward_issue: Option<KeyOnly>,
}

impl IssueLink {
    /// The issue on the other end, whichever direction the link points.
    pub fn other(&self) -> Option<&str> {
        self.inward_issue
            .as_ref()
            .or(self.outward_issue.as_ref())
            .map(|k| k.key.as_str())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Comment {
    pub id: String,
    #[serde(default)]
    pub author: Option<User>,
    #[serde(default)]
    pub body: Option<Value>,
    #[serde(deserialize_with = "timestamp")]
    pub created: DateTime<Utc>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchPage {
    #[serde(default, deserialize_with = "null_default")]
    issues: Vec<Issue>,
    #[serde(default)]
    next_page_token: Option<String>,
    #[serde(default)]
    is_last: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentPage {
    #[serde(default, deserialize_with = "null_default")]
    comments: Vec<Comment>,
    #[serde(default)]
    total: Option<u64>,
}

/// A page that could not be fetched (after the retry) or read. `page` names it, e.g. `search page 2`.
#[derive(Debug, thiserror::Error)]
#[error("jira: {page} failed{}: {body}", .status.map(|s| format!(" (HTTP {s})")).unwrap_or_default())]
pub struct FetchError {
    pub page: String,
    pub status: Option<u16>,
    pub body: String,
}

pub struct JiraClient {
    http: reqwest::Client,
    base: Url,
    email: String,
    token: String,
}

impl JiraClient {
    pub fn new(base: Url, email: String, token: String) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .default_headers(headers)
                .build()
                .expect("http client"),
            base,
            email,
            token,
        }
    }

    /// `base` joined with `path`, keeping the base's own path (the scoped-token gateway has one).
    fn url(&self, path: &str) -> Url {
        let mut u = self.base.clone();
        u.path_segments_mut()
            .expect("an http(s) base URL")
            .pop_if_empty()
            .extend(path.split('/'));
        u
    }

    /// Every issue matching `jql`, paged by `nextPageToken` only (the endpoint has no `startAt`).
    pub async fn search(&self, jql: &str, fields: &[String]) -> Result<Vec<Issue>, FetchError> {
        let url = self.url("rest/api/3/search/jql");
        let fields = fields.join(",");
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        for page in 1..=MAX_SEARCH_PAGES + 1 {
            let name = format!("search page {page}");
            if page > MAX_SEARCH_PAGES {
                return Err(FetchError {
                    page: name,
                    status: None,
                    body: format!("more than {MAX_SEARCH_PAGES} pages; stopping"),
                });
            }
            let mut query = vec![
                ("jql", jql.to_string()),
                ("fields", fields.clone()),
                ("maxResults", PAGE_SIZE.to_string()),
            ];
            if let Some(t) = &token {
                query.push(("nextPageToken", t.clone()));
            }
            let p: SearchPage = self.get(&url, &query, &name).await?;
            out.extend(p.issues);
            if p.is_last == Some(true) {
                break;
            }
            match p.next_page_token {
                // Only `isLast: true` ends the search (Jira Cloud always sends `isLast` on `search/jql`); a page that does
                // not say it is the last but gives no token would silently truncate the import.
                None => {
                    return Err(FetchError {
                        page: name,
                        status: None,
                        body: format!(
                            "page {page} says more follow (isLast is {}) but gives no nextPageToken",
                            p.is_last.map_or("absent".to_string(), |b| b.to_string())
                        ),
                    });
                }
                // A token that does not move would page forever.
                Some(t) if t.is_empty() || token.as_deref() == Some(t.as_str()) => {
                    return Err(FetchError {
                        page: name,
                        status: None,
                        body: "jira returned an empty or repeated nextPageToken without isLast"
                            .into(),
                    });
                }
                Some(t) => token = Some(t),
            }
        }
        Ok(out)
    }

    /// Every comment on `key`, paged by `startAt`/`maxResults`/`total`. Every page must carry `total`, and paging ends
    /// only when `startAt` plus the page's comments reaches it: a page without `total`, or an empty page short of it
    /// (no progress), is a `FetchError` naming the page rather than a silently truncated thread.
    pub async fn comments(&self, key: &str) -> Result<Vec<Comment>, FetchError> {
        let url = self.url(&format!("rest/api/3/issue/{key}/comment"));
        let mut out: Vec<Comment> = Vec::new();
        for page in 1.. {
            let name = format!("comments of {key} page {page}");
            let start = out.len();
            let query = [
                ("startAt", start.to_string()),
                ("maxResults", PAGE_SIZE.to_string()),
            ];
            let p: CommentPage = self.get(&url, &query, &name).await?;
            let Some(total) = p.total else {
                return Err(FetchError {
                    page: name,
                    status: None,
                    body: "jira returned a comment page without total".into(),
                });
            };
            let n = p.comments.len();
            out.extend(p.comments);
            if (start + n) as u64 >= total {
                break;
            }
            if n == 0 {
                return Err(FetchError {
                    page: name,
                    status: None,
                    body: format!(
                        "jira returned no comments at startAt {start} but advertises total {total}"
                    ),
                });
            }
        }
        Ok(out)
    }

    /// One GET with a single retry on 429 or 5xx (or a transport error, including a body cut off mid-read), waiting `Retry-After` up to 60 s.
    async fn get<T: DeserializeOwned>(
        &self,
        url: &Url,
        query: &[(&str, String)],
        page: &str,
    ) -> Result<T, FetchError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let sent = self
                .http
                .get(url.clone())
                .query(query)
                .basic_auth(&self.email, Some(&self.token))
                .send()
                .await;
            let (status, wait, body) = match sent {
                Err(e) => (None, DEFAULT_RETRY_AFTER, e.to_string()),
                Ok(resp) => {
                    let status = resp.status();
                    let wait = retry_after(&resp);
                    // Headers without the whole body (the connection closed or timed out mid-page) is a transport
                    // failure: retried like one, its error kept for the FetchError.
                    let body = match resp.text().await {
                        Ok(body) => body,
                        Err(e) => {
                            let body = format!("reading the response body failed: {e}");
                            if attempt >= 2 {
                                return Err(fetch_error(page, Some(status), body));
                            }
                            tokio::time::sleep(wait).await;
                            continue;
                        }
                    };
                    if status.is_success() {
                        return serde_json::from_str(&body).map_err(|e| FetchError {
                            page: page.to_string(),
                            status: Some(status.as_u16()),
                            body: format!("unreadable response: {e}"),
                        });
                    }
                    if !(status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()) {
                        return Err(fetch_error(page, Some(status), body));
                    }
                    (Some(status), wait, body)
                }
            };
            if attempt >= 2 {
                return Err(fetch_error(page, status, body));
            }
            tokio::time::sleep(wait).await;
        }
    }
}

fn fetch_error(page: &str, status: Option<StatusCode>, body: String) -> FetchError {
    FetchError {
        page: page.to_string(),
        status: status.map(|s| s.as_u16()),
        body: body.chars().take(500).collect(),
    }
}

/// `Retry-After` in seconds, capped at 60 s; absent or unreadable means the default wait.
fn retry_after(resp: &reqwest::Response) -> Duration {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|s| Duration::from_secs(s).min(MAX_RETRY_AFTER))
        .unwrap_or(DEFAULT_RETRY_AFTER)
}

fn null_default<'de, D: Deserializer<'de>, T: Default + Deserialize<'de>>(
    d: D,
) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// Jira writes `2026-09-01T10:15:30.123+0000` (no colon in the offset); RFC 3339 is accepted too.
pub fn parse_timestamp(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.3f%z")
        .or_else(|_| DateTime::parse_from_rfc3339(s))
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

fn timestamp<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
    let s = String::deserialize(d)?;
    parse_timestamp(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid timestamp {s:?}")))
}

fn opt_timestamp<'de, D: Deserializer<'de>>(d: D) -> Result<Option<DateTime<Utc>>, D::Error> {
    match Option::<String>::deserialize(d)? {
        None => Ok(None),
        Some(s) => parse_timestamp(&s)
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid timestamp {s:?}"))),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::{
        Router,
        extract::{Path, Query, State},
        http::{HeaderMap, StatusCode as AxStatus},
        response::{IntoResponse, Response},
        routing::get,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    pub(crate) fn fixture(name: &str) -> Value {
        let path = format!("{}/tests/fixtures/jira/{name}", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
    }

    async fn serve(app: Router) -> Url {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}").parse().unwrap()
    }

    /// One request the fake received on `/search/jql`.
    #[derive(Clone, Debug)]
    struct Seen {
        query: HashMap<String, String>,
        authorization: Option<String>,
        accept: Option<String>,
    }
    type Log = Arc<std::sync::Mutex<Vec<Seen>>>;

    /// The fake Jira: serves the fixture pages by project and `nextPageToken`, refuses `startAt` on search, and
    /// records every search request.
    async fn search_jql(
        State(log): State<Log>,
        headers: HeaderMap,
        Query(q): Query<HashMap<String, String>>,
    ) -> Response {
        let header = |n: &str| {
            headers
                .get(n)
                .and_then(|v| v.to_str().ok())
                .map(String::from)
        };
        log.lock().unwrap().push(Seen {
            query: q.clone(),
            authorization: header("authorization"),
            accept: header("accept"),
        });
        if q.contains_key("startAt") {
            return (AxStatus::BAD_REQUEST, "startAt is not supported here").into_response();
        }
        let jql = q.get("jql").cloned().unwrap_or_default();
        let kind = if jql.contains("JPD") {
            "ideas"
        } else {
            "issues"
        };
        let page = match q.get("nextPageToken").map(String::as_str) {
            None => 1,
            Some(t) if t == format!("{kind}-page-2") => 2,
            Some(_) => return (AxStatus::BAD_REQUEST, "bad token").into_response(),
        };
        axum::Json(fixture(&format!("{kind}_page{page}.json"))).into_response()
    }

    async fn comments(Path(key): Path<String>) -> Response {
        axum::Json(fixture(&format!("comments_{key}_page1.json"))).into_response()
    }

    fn fake() -> (Router, Log) {
        let log = Log::default();
        let app = Router::new()
            .route("/rest/api/3/search/jql", get(search_jql))
            .route("/rest/api/3/issue/{key}/comment", get(comments))
            .with_state(log.clone());
        (app, log)
    }

    fn client(base: Url) -> JiraClient {
        JiraClient::new(base, "me@example.com".into(), "token".into())
    }

    fn fields() -> Vec<String> {
        search_fields("customfield_10042", "customfield_10043")
    }

    async fn within<F: std::future::Future>(f: F) -> F::Output {
        tokio::time::timeout(Duration::from_secs(30), f)
            .await
            .expect("the client must stop paging")
    }

    #[tokio::test]
    async fn two_idea_pages_concatenate_through_the_token() {
        let base = serve(fake().0).await;
        let ideas = within(client(base).search("project = JPD", &fields()))
            .await
            .unwrap();
        let keys: Vec<_> = ideas.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, ["JPD-1", "JPD-2", "JPD-3"]);
        let b = ideas[0].fields.custom("customfield_10042").unwrap();
        assert_eq!(b["value"], "Now");
    }

    #[tokio::test]
    async fn issues_parse_null_assignees_and_jira_timestamps() {
        let base = serve(fake().0).await;
        let issues = within(client(base).search("project = SCRUM", &fields()))
            .await
            .unwrap();
        assert_eq!(issues.len(), 10);
        let s12 = issues.iter().find(|i| i.key == "SCRUM-12").unwrap();
        assert!(s12.fields.assignee.is_none());
        assert!(s12.fields.resolution.is_some());
        let s16 = issues.iter().find(|i| i.key == "SCRUM-16").unwrap();
        assert!(s16.fields.resolution.is_none());
        assert_eq!(
            s16.fields.created.unwrap().to_rfc3339(),
            "2026-08-01T09:00:00+00:00"
        );
        let s17 = issues.iter().find(|i| i.key == "SCRUM-17").unwrap();
        assert!(
            s17.fields
                .assignee
                .as_ref()
                .unwrap()
                .email_address
                .is_none()
        );
    }

    /// Catches a client paging by offset: the fake refuses `startAt`, and the log proves it was never sent.
    #[tokio::test]
    async fn search_never_sends_start_at_and_sends_auth_accept_and_fields() {
        let (app, log) = fake();
        let base = serve(app).await;
        within(client(base.clone()).search("project = SCRUM", &fields()))
            .await
            .unwrap();
        let seen = log.lock().unwrap().clone();
        assert_eq!(seen.len(), 2);
        assert!(seen.iter().all(|s| !s.query.contains_key("startAt")));
        assert_eq!(seen[1].query["nextPageToken"], "issues-page-2");
        for s in &seen {
            // base64("me@example.com:token")
            assert_eq!(
                s.authorization.as_deref(),
                Some("Basic bWVAZXhhbXBsZS5jb206dG9rZW4=")
            );
            assert_eq!(s.accept.as_deref(), Some("application/json"));
            assert_eq!(s.query["maxResults"], "100");
            assert_eq!(
                s.query["fields"],
                "summary,description,status,resolution,issuetype,assignee,labels,parent,created,issuelinks,customfield_10042,customfield_10043"
            );
        }
        // The fake itself refuses offsets, so a client that used them would fail.
        let c = client(base);
        let err = c
            .get::<Value>(
                &c.url("rest/api/3/search/jql"),
                &[("jql", "project = JPD".into()), ("startAt", "0".into())],
                "search page 1",
            )
            .await
            .unwrap_err();
        assert_eq!(err.status, Some(400));
    }

    /// Catches a client that pages forever on a token that does not move.
    #[tokio::test]
    async fn a_repeated_or_empty_token_fails_naming_the_page() {
        for token in ["same", ""] {
            let app = Router::new().route(
                "/rest/api/3/search/jql",
                get(move || async move {
                    axum::Json(
                        serde_json::json!({"isLast": false, "nextPageToken": token, "issues": []}),
                    )
                }),
            );
            let base = serve(app).await;
            let err = within(client(base).search("project = JPD", &fields()))
                .await
                .unwrap_err();
            let expected = if token.is_empty() {
                "search page 1"
            } else {
                "search page 2"
            };
            assert_eq!(err.page, expected, "token {token:?}");
            assert!(err.body.contains("repeated nextPageToken"));
        }
    }

    /// Catches a client that ends the search, truncating the import, on a page that does not say it is the last but
    /// gives no token; only `isLast: true` ends it.
    #[tokio::test]
    async fn a_page_that_is_not_last_without_a_token_fails_naming_the_page() {
        for body in [
            serde_json::json!({"isLast": false, "issues": []}),
            serde_json::json!({"isLast": false, "nextPageToken": null, "issues": []}),
            serde_json::json!({"issues": []}),
        ] {
            let page = body.clone();
            let app = Router::new().route(
                "/rest/api/3/search/jql",
                get(move || async move { axum::Json(page) }),
            );
            let base = serve(app).await;
            let err = within(client(base).search("project = JPD", &fields()))
                .await
                .unwrap_err();
            assert_eq!(err.page, "search page 1", "{body}");
            assert!(
                err.body.contains("page 1 says more follow")
                    && err.body.contains("gives no nextPageToken"),
                "{body}: {}",
                err.body
            );
        }
        // `isLast: true` without a token ends the search.
        let app = Router::new().route(
            "/rest/api/3/search/jql",
            get(|| async { axum::Json(serde_json::json!({"isLast": true, "issues": []})) }),
        );
        let base = serve(app).await;
        assert!(
            within(client(base).search("project = JPD", &fields()))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn comments_page_and_parse() {
        let base = serve(fake().0).await;
        let cs = within(client(base).comments("SCRUM-11")).await.unwrap();
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].id, "900");
        assert_eq!(cs[0].created.to_rfc3339(), "2026-09-01T10:15:30.123+00:00");
    }

    #[tokio::test]
    async fn comments_follow_start_at_across_pages() {
        let app = Router::new().route(
            "/rest/api/3/issue/{key}/comment",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                let start: usize = q["startAt"].parse().unwrap();
                let c = |id: usize| {
                    serde_json::json!({"id": id.to_string(), "created": "2026-09-01T10:15:30.123+0000"})
                };
                let comments: Vec<Value> = (start..(start + 2).min(3)).map(c).collect();
                axum::Json(serde_json::json!({"startAt": start, "maxResults": 2, "total": 3, "comments": comments}))
            }),
        );
        let base = serve(app).await;
        let cs = within(client(base).comments("SCRUM-1")).await.unwrap();
        let ids: Vec<_> = cs.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["0", "1", "2"]);
    }

    /// Catches a comment thread silently truncated by an empty page short of `total`, or by a page with no `total`.
    #[tokio::test]
    async fn comment_paging_without_progress_or_total_is_an_error() {
        for (body, needle) in [
            (
                serde_json::json!({"startAt": 0, "maxResults": 2, "total": 3, "comments": []}),
                "advertises total 3",
            ),
            (
                serde_json::json!({"startAt": 0, "maxResults": 2, "comments": [
                    {"id": "1", "created": "2026-09-01T10:15:30.123+0000"}]}),
                "without total",
            ),
        ] {
            let app = Router::new().route(
                "/rest/api/3/issue/{key}/comment",
                get(move || {
                    let body = body.clone();
                    async move { axum::Json(body) }
                }),
            );
            let base = serve(app).await;
            let err = within(client(base).comments("SCRUM-1")).await.unwrap_err();
            assert_eq!(err.page, "comments of SCRUM-1 page 1");
            assert!(err.body.contains(needle), "{}", err.body);
        }
        // An empty thread (total 0) is not an error.
        let app = Router::new().route(
            "/rest/api/3/issue/{key}/comment",
            get(|| async {
                axum::Json(
                    serde_json::json!({"startAt": 0, "maxResults": 2, "total": 0, "comments": []}),
                )
            }),
        );
        let base = serve(app).await;
        assert!(
            within(client(base).comments("SCRUM-1"))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_429_with_retry_after_zero_then_200_succeeds() {
        let hits = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/rest/api/3/search/jql",
                get(|State(hits): State<Arc<AtomicUsize>>| async move {
                    if hits.fetch_add(1, Ordering::SeqCst) == 0 {
                        let mut h = HeaderMap::new();
                        h.insert("Retry-After", "0".parse().unwrap());
                        return (AxStatus::TOO_MANY_REQUESTS, h, "slow down").into_response();
                    }
                    axum::Json(fixture("ideas_page2.json")).into_response()
                }),
            )
            .with_state(hits.clone());
        let base = serve(app).await;
        let ideas = within(client(base).search("project = JPD", &fields()))
            .await
            .unwrap();
        assert_eq!(ideas.len(), 1);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_500_twice_fails_the_run_naming_the_page() {
        let hits = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/rest/api/3/search/jql",
                get(
                    |State(hits): State<Arc<AtomicUsize>>,
                     Query(q): Query<HashMap<String, String>>| async move {
                        if q.contains_key("nextPageToken") {
                            hits.fetch_add(1, Ordering::SeqCst);
                            let mut h = HeaderMap::new();
                            h.insert("Retry-After", "0".parse().unwrap());
                            return (AxStatus::INTERNAL_SERVER_ERROR, h, "boom").into_response();
                        }
                        axum::Json(fixture("ideas_page1.json")).into_response()
                    },
                ),
            )
            .with_state(hits.clone());
        let base = serve(app).await;
        let err = within(client(base).search("project = JPD", &fields()))
            .await
            .unwrap_err();
        assert_eq!(err.page, "search page 2");
        assert_eq!(err.status, Some(500));
        assert_eq!(err.body, "boom");
        assert_eq!(hits.load(Ordering::SeqCst), 2, "one retry, no more");
        assert!(err.to_string().contains("search page 2"));
    }

    /// A raw HTTP server whose first `cut` responses send a 200's headers (a `Content-Length` of 4096) and then close
    /// the connection mid-body; later responses serve `ideas_page2.json` whole. Counts the requests.
    async fn cut_off_server(cut: usize) -> (Url, Arc<AtomicUsize>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut req = Vec::new();
                let mut buf = [0u8; 1024];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = sock.read(&mut buf).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    req.extend_from_slice(&buf[..n]);
                }
                let resp = if counter.fetch_add(1, Ordering::SeqCst) < cut {
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 4096\r\nretry-after: 0\r\n\r\n{\"issues\": [".to_string()
                } else {
                    let body = fixture("ideas_page2.json").to_string();
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                sock.write_all(resp.as_bytes()).await.unwrap();
                sock.shutdown().await.ok();
            }
        });
        (format!("http://{addr}").parse().unwrap(), hits)
    }

    #[tokio::test]
    async fn a_body_cut_off_mid_read_is_retried_once() {
        let (base, hits) = cut_off_server(1).await;
        let ideas = within(client(base).search("project = JPD", &fields()))
            .await
            .unwrap();
        assert_eq!(ideas.len(), 1);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_body_cut_off_twice_fails_naming_the_page_and_the_read_error() {
        let (base, hits) = cut_off_server(usize::MAX).await;
        let err = within(client(base).search("project = JPD", &fields()))
            .await
            .unwrap_err();
        assert_eq!(err.page, "search page 1");
        assert_eq!(err.status, Some(200));
        assert!(
            err.body.starts_with("reading the response body failed: "),
            "{}",
            err.body
        );
        assert_eq!(hits.load(Ordering::SeqCst), 2, "one retry, no more");
    }

    #[tokio::test]
    async fn an_idea_with_a_list_bucket_still_parses() {
        let mut page = fixture("ideas_page2.json");
        page["issues"][0]["fields"]["customfield_10042"] = serde_json::json!([{"value": "Now"}]);
        page["issues"][0]["fields"]["customfield_10043"] = serde_json::json!(42);
        let app = Router::new().route(
            "/rest/api/3/search/jql",
            get(move || {
                let page = page.clone();
                async move { axum::Json(page) }
            }),
        );
        let base = serve(app).await;
        let ideas = within(client(base).search("project = JPD", &fields()))
            .await
            .unwrap();
        assert_eq!(ideas.len(), 1);
        assert!(
            ideas[0]
                .fields
                .custom("customfield_10042")
                .unwrap()
                .is_array()
        );
    }

    #[test]
    fn urls_keep_the_base_path() {
        let c = client("https://api.atlassian.com/ex/jira/abc".parse().unwrap());
        assert_eq!(
            c.url("rest/api/3/search/jql").as_str(),
            "https://api.atlassian.com/ex/jira/abc/rest/api/3/search/jql"
        );
        let c = client("https://x.atlassian.net/".parse().unwrap());
        assert_eq!(
            c.url("rest/api/3/issue/SCRUM-1/comment").as_str(),
            "https://x.atlassian.net/rest/api/3/issue/SCRUM-1/comment"
        );
    }

    #[test]
    fn timestamps_parse_in_both_forms() {
        assert!(parse_timestamp("2026-09-01T10:15:30.123+0000").is_some());
        assert!(parse_timestamp("2026-09-01T10:15:30Z").is_some());
        assert!(parse_timestamp("yesterday").is_none());
    }
}
