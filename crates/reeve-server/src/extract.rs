//! Extractors whose rejections are envelopes: a malformed body, path or query is a `validation` error, not axum's plain text.
use crate::error::AppError;
use axum::extract::{
    FromRequest, FromRequestParts,
    rejection::{JsonRejection, PathRejection, QueryRejection},
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
