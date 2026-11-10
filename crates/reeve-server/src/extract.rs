//! Extractors whose rejections are envelopes: a malformed body, path or query is a `validation` error, not axum's plain text.
use crate::error::AppError;
use axum::extract::{
    FromRequest, FromRequestParts,
    rejection::{BytesRejection, FailedToBufferBody, JsonRejection, PathRejection, QueryRejection},
};

#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(AppError))]
pub struct ApiJson<T>(pub T);

#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(AppError))]
pub struct ApiPath<T>(pub T);

#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(AppError))]
pub struct ApiQuery<T>(pub T);

impl From<JsonRejection> for AppError {
    fn from(r: JsonRejection) -> Self {
        Self::Validation(r.body_text())
    }
}
impl From<PathRejection> for AppError {
    fn from(r: PathRejection) -> Self {
        Self::Validation(r.body_text())
    }
}
impl From<QueryRejection> for AppError {
    fn from(r: QueryRejection) -> Self {
        Self::Validation(r.body_text())
    }
}

/// A raw body (an upload), whose rejection is an envelope too: over the route's `DefaultBodyLimit` it is 413, with or
/// without a `Content-Length`.
pub struct ApiBytes(pub axum::body::Bytes);

impl<S: Send + Sync> FromRequest<S> for ApiBytes {
    type Rejection = AppError;
    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, AppError> {
        match axum::body::Bytes::from_request(req, state).await {
            Ok(b) => Ok(Self(b)),
            Err(BytesRejection::FailedToBufferBody(FailedToBufferBody::LengthLimitError(_))) => {
                Err(AppError::TooLarge("the file is larger than 10 MiB".into()))
            }
            Err(r) => Err(AppError::Validation(r.body_text())),
        }
    }
}
