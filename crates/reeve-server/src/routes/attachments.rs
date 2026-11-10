//! Attachments (kb/ADR-0022, kb/ADR-0023): upload, serve, list and delete. Every request passes the normal
//! authentication (`Caller`), and serving reads one chunk per query, so no connection is held for a slow client.
use crate::{
    auth::Caller,
    error::{AppError, ok},
    extract::{ApiBytes, ApiPath, ApiQuery},
    state::AppState,
};
use axum::{
    Json,
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response as HttpResponse},
};
use reeve_planning::{Attachment, MAX_BYTES, attachment_dto};
use reeve_protocol::*;
use serde::Deserialize;
use std::sync::Arc;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct UploadQuery {
    /// The file's name, as the person's device gave it; the server keeps only a safe last segment.
    pub filename: String,
    /// Caller-chosen key that makes a retried upload safe: a repeat returns the first upload. In the query, not the
    /// body, because the body is the file.
    pub idempotency_key: Option<String>,
}

/// The raw bytes of a PNG, JPEG or WebP image; the server identifies the type from the bytes, whatever the
/// request's `Content-Type` says.
#[derive(utoipa::ToSchema)]
#[schema(value_type = String, format = Binary)]
#[allow(dead_code)] // only names the body's schema
struct ImageBytes(Vec<u8>);

/// Uploads an image, not yet the item's: the write that saves the text showing it claims it.
#[utoipa::path(post, path = "/attachments", params(UploadQuery),
    request_body(content = inline(ImageBytes), content_type = "application/octet-stream"),
    responses((status = 200, body = Response<AttachmentDto>), (status = 413, description = "larger than 10 MiB", body = Response<Empty>)))]
async fn upload(
    State(s): State<AppState>,
    c: Caller,
    ApiQuery(q): ApiQuery<UploadQuery>,
    ApiBytes(body): ApiBytes,
) -> Result<Json<Response<AttachmentDto>>, AppError> {
    let a = s
        .handlers
        .upload_attachment(&c.ctx(), &q.filename, q.idempotency_key.as_deref(), body)
        .await?;
    Ok(ok(attachment_dto(&a)))
}

fn etag(a: &Attachment) -> String {
    let hex: String = a.sha256().iter().map(|b| format!("{b:02x}")).collect();
    format!("\"{hex}\"")
}

/// The headers of every attachment response (kb/ADR-0023 R4).
fn image_headers(a: &Attachment, etag: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    let set = |h: &mut HeaderMap, k: header::HeaderName, v: &str| {
        if let Ok(v) = HeaderValue::from_str(v) {
            h.insert(k, v);
        }
    };
    set(&mut h, header::CONTENT_TYPE, a.image_type().mime());
    set(&mut h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(
        &mut h,
        header::CONTENT_SECURITY_POLICY,
        "default-src 'none'; sandbox",
    );
    set(&mut h, header::CACHE_CONTROL, "private, no-cache");
    set(&mut h, header::ETAG, etag);
    set(
        &mut h,
        header::CONTENT_DISPOSITION,
        &format!(
            "inline; filename*=UTF-8''{}",
            urlencoding::encode(a.filename())
        ),
    );
    h
}

/// The image, streamed one stored chunk at a time; 304 for a matching `If-None-Match`, only after the access check.
#[utoipa::path(get, path = "/attachments/{id}", params(("id" = Uuid, Path)),
    responses((status = 200, description = "the image", content_type = "application/octet-stream", body = inline(ImageBytes)), (status = 304, description = "not modified")))]
async fn get(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<Uuid>,
    req: HeaderMap,
) -> Result<HttpResponse, AppError> {
    // Authentication (the Caller) and visibility first: a 304 is an answer about the file too (kb/ADR-0023 R4, R5).
    let a = s.handlers.attachment_meta(id, &c.ctx().actor).await?;
    let tag = etag(&a);
    let headers = image_headers(&a, &tag);
    if req
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == tag))
    {
        return Ok((StatusCode::NOT_MODIFIED, headers).into_response());
    }
    let handlers = Arc::new(s.handlers.clone());
    let body = futures_util::stream::unfold(Some(0i32), move |n| {
        let handlers = handlers.clone();
        async move {
            let n = n?;
            match handlers.attachment_chunk(id, n).await {
                Ok(Some(b)) => Some((Ok(b), Some(n + 1))),
                Ok(None) => None,
                Err(e) => {
                    tracing::error!(error = ?e, %id, chunk = n, "attachment chunk");
                    Some((Err(std::io::Error::other("attachment chunk")), None))
                }
            }
        }
    });
    let mut res = (headers, Body::from_stream(body)).into_response();
    if let Ok(len) = HeaderValue::from_str(&a.byte_size().to_string()) {
        res.headers_mut().insert(header::CONTENT_LENGTH, len);
    }
    Ok(res)
}

/// An item's attachments, oldest first.
#[utoipa::path(get, path = "/items/{key}/attachments", params(("key" = String, Path)), responses((status = 200, body = Response<Vec<AttachmentDto>>)))]
async fn list(
    State(s): State<AppState>,
    _c: Caller,
    ApiPath(key): ApiPath<String>,
) -> Result<Json<Response<Vec<AttachmentDto>>>, AppError> {
    let (t, key) = s.handlers.resolve_any(&key).await?;
    let all = s.handlers.list_attachments(t, &key).await?;
    Ok(ok(all.iter().map(attachment_dto).collect()))
}

/// Deletes an attachment for good: any member's for a claimed one, only the uploader's otherwise.
#[utoipa::path(delete, path = "/attachments/{id}", params(("id" = Uuid, Path)), responses((status = 200, body = Response<Empty>)))]
async fn delete(
    State(s): State<AppState>,
    c: Caller,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<Response<Empty>>, AppError> {
    s.handlers.delete_attachment(&c.ctx(), id).await?;
    Ok(ok(Empty {}))
}

pub fn router() -> OpenApiRouter<AppState> {
    // The larger body limit is the upload's alone; every other route keeps axum's default.
    let uploads = OpenApiRouter::new()
        .routes(routes!(upload))
        .layer(DefaultBodyLimit::max(MAX_BYTES));
    OpenApiRouter::new()
        .merge(uploads)
        .routes(routes!(get, delete))
        .routes(routes!(list))
}
