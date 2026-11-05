use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotFound,
    Conflict,
    Forbidden,
    Unauthenticated,
    Validation,
    Internal,
    /// Too many requests (HTTP 429, usually from the proxy in front of the server): wait and retry the same request;
    /// a create retried with its idempotency key answers the same item. The server itself never sends it.
    RateLimited,
}

/// Every API response body is one of these three shapes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Response<T> {
    Ok {
        value: T,
    },
    // Built with `Response::refused`, which sets `overridable` from the reasons.
    Refused {
        reasons: Vec<crate::readiness::Reason>,
        /// True when there is at least one reason and every one is a readiness fact: an interactive caller may retry
        /// with an override reason. False when any reason is a delivery rule. Absent (an older server) reads as false.
        #[serde(default)]
        #[schema(required = true)]
        overridable: bool,
    },
    Error {
        code: ErrorCode,
        message: String,
        /// On a `conflict`: the row's version now, or -1 when the server retried a serialisation failure. Absent otherwise.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current: Option<i64>,
    },
}

impl<T> Response<T> {
    /// The refusal for `reasons`, its `overridable` flag computed from them: the one constructor, so no refusal can
    /// leave the flag unset.
    pub fn refused(reasons: Vec<crate::readiness::Reason>) -> Self {
        let overridable = !reasons.is_empty() && reasons.iter().all(|r| r.overridable());
        Response::Refused {
            reasons,
            overridable,
        }
    }
}

/// The value of a response that carries nothing; serialises as `{}` and has a real schema component.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Empty {}

/// utoipa 5 cannot build a schema for `Response<()>` inline; routes with no value use this alias and return `Response::Ok { value: Empty {} }`.
pub type EmptyResponse = Response<Empty>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::readiness::Reason;
    #[test]
    fn envelope_variants_roundtrip_and_tag() {
        let ok: Response<u8> = Response::Ok { value: 3 };
        let v = serde_json::to_value(&ok).unwrap();
        assert_eq!(v["outcome"], "ok");
        let refused: Response<u8> = Response::refused(vec![Reason::NoCriteria]);
        let v2 = serde_json::to_value(&refused).unwrap();
        assert_eq!(v2["reasons"][0]["reason"], "no_criteria");
        let back: Response<u8> = serde_json::from_value(v2).unwrap();
        assert_eq!(back, refused);
    }
    #[test]
    fn a_refusal_is_overridable_only_when_every_reason_is() {
        let flag = |rs: Vec<Reason>| match Response::<u8>::refused(rs) {
            Response::Refused { overridable, .. } => overridable,
            other => panic!("{other:?}"),
        };
        assert!(flag(vec![Reason::NoCriteria, Reason::Untriaged]));
        assert!(!flag(vec![Reason::NoCriteria, Reason::ResolutionRequired]));
        assert!(!flag(vec![Reason::AutomationCannotOverride]));
        assert!(
            !flag(vec![]),
            "a refusal without reasons offers no override"
        );
        let v = serde_json::to_value(Response::<u8>::refused(vec![Reason::NoCriteria])).unwrap();
        assert_eq!(v["overridable"], true);
    }
    #[test]
    fn an_older_refusal_without_the_flag_reads_as_not_overridable() {
        let v = serde_json::json!({"outcome":"refused","reasons":[{"reason":"no_criteria"}]});
        assert_eq!(
            serde_json::from_value::<Response<u8>>(v).unwrap(),
            Response::Refused {
                reasons: vec![Reason::NoCriteria],
                overridable: false,
            }
        );
    }
    #[test]
    fn error_carries_current_only_when_set() {
        let plain: Response<u8> = Response::Error {
            code: ErrorCode::Validation,
            message: "m".into(),
            current: None,
        };
        let v = serde_json::to_value(&plain).unwrap();
        assert!(v.get("current").is_none());
        assert_eq!(serde_json::from_value::<Response<u8>>(v).unwrap(), plain);
        let conflict: Response<u8> = Response::Error {
            code: ErrorCode::Conflict,
            message: "m".into(),
            current: Some(7),
        };
        assert_eq!(serde_json::to_value(&conflict).unwrap()["current"], 7);
    }
    #[test]
    fn empty_value_is_an_object_and_error_codes_are_snake() {
        let v = serde_json::to_value(EmptyResponse::Ok { value: Empty {} }).unwrap();
        assert_eq!(v["value"], serde_json::json!({}));
        assert_eq!(
            serde_json::to_value(ErrorCode::NotFound).unwrap(),
            "not_found"
        );
        assert_eq!(
            serde_json::to_value(ErrorCode::Unauthenticated).unwrap(),
            "unauthenticated"
        );
    }
}
