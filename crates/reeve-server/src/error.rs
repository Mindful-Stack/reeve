use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use reeve_planning::HandlerError;
use reeve_protocol::{ErrorCode, Reason};

#[derive(Debug)]
pub enum AppError {
    Unauthenticated,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    Conflict {
        current: i64,
    },
    Validation(String),
    /// A body over its route's limit: 413 with the `validation` code.
    TooLarge(String),
    Refused(Vec<Reason>),
    Internal(anyhow::Error),
}

impl From<HandlerError> for AppError {
    fn from(e: HandlerError) -> Self {
        match e {
            HandlerError::NotFound => Self::NotFound,
            HandlerError::Conflict { current } => Self::Conflict { current },
            HandlerError::Validation(m) => Self::Validation(m),
            HandlerError::Refused(r) => Self::Refused(r),
            HandlerError::Forbidden => Self::Forbidden,
            HandlerError::Db(e) => AppError::from(e),
        }
    }
}
impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        // 40001 serialization_failure from a REPEATABLE READ move whose row changed underneath it: a conflict, not a crash.
        // HandlerError::Db delegates here so a wrapped 40001 maps the same way; a unit test in error.rs pins both paths.
        if let sqlx::Error::Database(db) = &e
            && db.code().as_deref() == Some("40001")
        {
            return Self::Conflict { current: -1 };
        }
        Self::Internal(e.into())
    }
}
impl From<reeve_identity::AuthError> for AppError {
    fn from(e: reeve_identity::AuthError) -> Self {
        use reeve_identity::AuthError as A;
        match e {
            A::NotAllowed | A::Validation(_) => Self::Validation(e.to_string()),
            A::Unauthenticated => Self::Unauthenticated,
            A::Disabled | A::Forbidden => Self::Forbidden,
            A::NotFound => Self::NotFound,
            A::Db(e) => AppError::from(e),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, body): (StatusCode, reeve_protocol::EmptyResponse) = match self {
            AppError::Refused(reasons) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                reeve_protocol::Response::refused(reasons),
            ),
            AppError::Unauthenticated => {
                let body = reeve_protocol::Response::<reeve_protocol::Empty>::Error {
                    code: ErrorCode::Unauthenticated,
                    message: "unauthenticated".into(),
                    current: None,
                };
                return (
                    StatusCode::UNAUTHORIZED,
                    [(axum::http::header::WWW_AUTHENTICATE, "Bearer")],
                    Json(body),
                )
                    .into_response();
            }
            AppError::Forbidden => (
                StatusCode::FORBIDDEN,
                reeve_protocol::Response::Error {
                    code: ErrorCode::Forbidden,
                    message: "forbidden".into(),
                    current: None,
                },
            ),
            AppError::NotFound => (
                StatusCode::NOT_FOUND,
                reeve_protocol::Response::Error {
                    code: ErrorCode::NotFound,
                    message: "not found".into(),
                    current: None,
                },
            ),
            AppError::MethodNotAllowed => (
                StatusCode::METHOD_NOT_ALLOWED,
                reeve_protocol::Response::Error {
                    code: ErrorCode::Validation,
                    message: "method not allowed".into(),
                    current: None,
                },
            ),
            AppError::Conflict { current } => (
                StatusCode::CONFLICT,
                reeve_protocol::Response::Error {
                    code: ErrorCode::Conflict,
                    message: if current < 0 {
                        "concurrent update, retry".into()
                    } else {
                        format!("version conflict: current is {current}")
                    },
                    current: Some(current),
                },
            ),
            AppError::TooLarge(m) => (
                StatusCode::PAYLOAD_TOO_LARGE,
                reeve_protocol::Response::Error {
                    code: ErrorCode::Validation,
                    message: m,
                    current: None,
                },
            ),
            AppError::Validation(m) => (
                StatusCode::BAD_REQUEST,
                reeve_protocol::Response::Error {
                    code: ErrorCode::Validation,
                    message: m,
                    current: None,
                },
            ),
            AppError::Internal(e) => {
                tracing::error!(error = ?e, "internal");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    reeve_protocol::Response::Error {
                        code: ErrorCode::Internal,
                        message: "internal error".into(),
                        current: None,
                    },
                )
            }
        };
        (status, Json(body)).into_response()
    }
}

pub fn ok<T: serde::Serialize>(value: T) -> Json<reeve_protocol::Response<T>> {
    Json(reeve_protocol::Response::Ok { value })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    #[derive(Debug)]
    struct Pg40001;
    impl std::fmt::Display for Pg40001 {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("could not serialize access")
        }
    }
    impl std::error::Error for Pg40001 {}
    impl sqlx::error::DatabaseError for Pg40001 {
        fn message(&self) -> &str {
            "could not serialize access"
        }
        fn code(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed("40001"))
        }
        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }
        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }
    }

    #[test]
    fn serialization_failure_maps_to_conflict_directly_and_through_the_handler() {
        let direct = AppError::from(sqlx::Error::Database(Box::new(Pg40001)));
        assert!(matches!(direct, AppError::Conflict { .. }));
        let wrapped = AppError::from(HandlerError::Db(sqlx::Error::Database(Box::new(Pg40001))));
        assert!(matches!(wrapped, AppError::Conflict { .. }));
        assert!(matches!(
            AppError::from(sqlx::Error::RowNotFound),
            AppError::Internal(_)
        ));
    }
}
