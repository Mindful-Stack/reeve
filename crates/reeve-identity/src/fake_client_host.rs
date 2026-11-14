//! A fake client host for tests, compiled only under `static-oidc` (which the Dockerfile never enables): an axum
//! server on 127.0.0.1 that serves client metadata documents and counts the requests it receives.
//!
//! Not reproduced: TLS, a host name, a public address. A test that passes against this fake says nothing about
//! the production fetcher's client id rules or its address check; those are tested in `client_fetch.rs`.
use axum::{
    Router,
    extract::State,
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// What the fake answers for one path. A path without an answer is a 404.
#[derive(Debug, Clone)]
pub enum FakeAnswer {
    /// `200`, `Content-Type: application/json`, and exactly `value.to_string()` as the body.
    Json(serde_json::Value),
    /// This status with an empty body, and a `Location` header when given.
    Status { code: u16, location: Option<String> },
    /// `200` without a `Content-Length`, then `after` bytes of body (the byte `x`), then nothing more: the body
    /// is left open and never ends.
    Stalls { after: usize },
}

struct Shared {
    answers: Mutex<HashMap<String, FakeAnswer>>,
    hits: Mutex<HashMap<String, usize>>,
}

pub struct FakeClientHost {
    base: String,
    shared: Arc<Shared>,
}

impl FakeClientHost {
    /// Binds a loopback port of its own and serves until the runtime ends.
    pub async fn start() -> FakeClientHost {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback port");
        let address = listener.local_addr().expect("the listener's address");
        let shared = Arc::new(Shared {
            answers: Mutex::new(HashMap::new()),
            hits: Mutex::new(HashMap::new()),
        });
        let app = Router::new().fallback(serve).with_state(shared.clone());
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("the fake client host serves");
        });
        FakeClientHost {
            base: format!("http://{address}"),
            shared,
        }
    }

    /// The client id of a document at `path` (which starts with `/`): `http://127.0.0.1:<port><path>`.
    pub fn client_id(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// The host part of every client id here: what `client_hosts` must hold for these clients.
    pub fn host(&self) -> &'static str {
        "127.0.0.1"
    }

    /// Sets what `path` answers from now on.
    pub fn answer(&self, path: &str, answer: FakeAnswer) {
        self.shared
            .answers
            .lock()
            .unwrap()
            .insert(path.to_string(), answer);
    }

    /// Serves a valid client document at `path` and returns its client id.
    pub fn publish(&self, path: &str, client_name: &str, redirect_uris: &[&str]) -> String {
        let client_id = self.client_id(path);
        self.answer(
            path,
            FakeAnswer::Json(serde_json::json!({
                "client_id": client_id,
                "client_name": client_name,
                "redirect_uris": redirect_uris,
                "token_endpoint_auth_method": "none"
            })),
        );
        client_id
    }

    /// How many requests for `path` the server received, answered or not.
    pub fn hits(&self, path: &str) -> usize {
        self.shared
            .hits
            .lock()
            .unwrap()
            .get(path)
            .copied()
            .unwrap_or(0)
    }
}

async fn serve(State(shared): State<Arc<Shared>>, uri: Uri) -> Response {
    let path = uri.path().to_string();
    *shared.hits.lock().unwrap().entry(path.clone()).or_default() += 1;
    let answer = shared.answers.lock().unwrap().get(&path).cloned();
    match answer {
        None => StatusCode::NOT_FOUND.into_response(),
        Some(FakeAnswer::Json(value)) => (
            [(header::CONTENT_TYPE, "application/json")],
            value.to_string(),
        )
            .into_response(),
        Some(FakeAnswer::Status { code, location }) => {
            let status = StatusCode::from_u16(code).expect("a status code");
            match location {
                Some(to) => (status, [(header::LOCATION, to)]).into_response(),
                None => status.into_response(),
            }
        }
        Some(FakeAnswer::Stalls { after }) => {
            let sent = futures_util::stream::once(async move {
                Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(vec![b'x'; after]))
            });
            // A stream has no known length, so the answer is chunked; `pending` never yields and never ends.
            axum::body::Body::from_stream(sent.chain(futures_util::stream::pending()))
                .into_response()
        }
    }
}
