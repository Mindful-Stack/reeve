//! `GET|HEAD /cli/{name}` (kb/ADR-0030 R3): the CLI bundle and the install scripts, without sign-in.
use crate::{cli_dist::Entry, error::AppError, state::AppState};
use axum::{
    Router,
    body::Body,
    extract::{Path, State, rejection::PathRejection},
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
    routing::get,
};
use reeve_protocol::{CLI_PATH, TEXT_CONTENT_TYPE};

/// `get` also answers HEAD; any other method gets this route's own 405, with the `/cli/` headers.
pub fn router() -> Router<AppState> {
    Router::new().route(
        &format!("{CLI_PATH}{{name}}"),
        get(serve).fallback(method_not_allowed),
    )
}

/// No `Caller` and no cookie jar: a token or cookie is never read, so it can neither fail nor authenticate a download,
/// and nothing here sets one. The name is a `Result`: a segment that is not UTF-8 once decoded (`/cli/%FF`) would
/// otherwise be axum's 400 before this handler runs; it names no file, so it is a 404.
async fn serve(State(s): State<AppState>, name: Result<Path<String>, PathRejection>) -> Response {
    let res = match name {
        Ok(Path(name)) => answer(&s, &name).await,
        Err(_) => AppError::NotFound.into_response(),
    };
    crate::cli_dist::harden(res)
}

async fn method_not_allowed() -> Response {
    crate::cli_dist::harden(AppError::MethodNotAllowed.into_response())
}

/// The name is only ever compared with the fixed list in `CliDist::entry`, never joined into a path.
async fn answer(s: &AppState, name: &str) -> Response {
    let Some(entry) = s.cli_dist.as_deref().and_then(|c| c.entry(name)) else {
        return AppError::NotFound.into_response();
    };
    match entry {
        Entry::Text(text) => (
            [(
                header::CONTENT_TYPE,
                format!("{TEXT_CONTENT_TYPE}; charset=utf-8"),
            )],
            text.to_owned(),
        )
            .into_response(),
        // Streamed: a binary is tens of megabytes and is never held in memory whole.
        Entry::File(path) => {
            let file = match tokio::fs::File::open(&path).await {
                Ok(f) => f,
                Err(e) => return internal(e, &path),
            };
            let len = match file.metadata().await {
                Ok(m) => m.len(),
                Err(e) => return internal(e, &path),
            };
            let mut res = (
                [(header::CONTENT_TYPE, "application/octet-stream")],
                // 256 KiB reads instead of the default 4 KiB: far fewer chunks for a binary of tens of megabytes.
                Body::from_stream(tokio_util::io::ReaderStream::with_capacity(
                    file,
                    256 * 1024,
                )),
            )
                .into_response();
            res.headers_mut()
                .insert(header::CONTENT_LENGTH, HeaderValue::from(len));
            res
        }
    }
}

/// A bundle file that vanished after startup (the directory must not change until the next start).
fn internal(e: std::io::Error, path: &std::path::Path) -> Response {
    AppError::Internal(anyhow::anyhow!("cli bundle file {}: {e}", path.display())).into_response()
}
