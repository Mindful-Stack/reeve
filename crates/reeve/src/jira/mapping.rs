//! `jira-import.toml`: which Jira projects, fields and values become which Reeve kinds, horizons and states.
//! Values stay strings until [`Mapping::validate`] has checked them, so a bad value is reported with its key.

use reeve_protocol::{DeliveryState, Kind, Resolution};
use reqwest::Url;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mapping {
    pub site: String,
    #[serde(default)]
    pub api_base: Option<String>,
    pub email: String,
    pub discovery_project: String,
    pub delivery_project: String,
    pub bucket_field: String,
    pub target_date_field: String,
    #[serde(default)]
    pub idea_link_type: String,
    #[serde(default = "default_acceptance_heading")]
    pub acceptance_heading: String,
    pub default_resolution: String,
    #[serde(default)]
    pub bucket_to_horizon: BTreeMap<String, String>,
    #[serde(default)]
    pub status_to_delivery: BTreeMap<String, String>,
    #[serde(default)]
    pub resolution_map: BTreeMap<String, String>,
    #[serde(default)]
    pub type_to_kind: BTreeMap<String, String>,
    /// accountId → email, for assignees whose email Jira hides.
    #[serde(default)]
    pub owner_map: BTreeMap<String, String>,
}

fn default_acceptance_heading() -> String {
    "Acceptance criteria".into()
}

/// What the mapping is checked for: a rehearsal tolerates an empty `idea_link_type` (and reports the link types it
/// saw), an apply does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Rehearse,
    Apply,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MappingError {
    #[error("jira-import.toml: {0}")]
    Parse(String),
    #[error("jira-import.toml: {key} = {value:?} is not one of {expected}")]
    Invalid {
        key: String,
        value: String,
        expected: String,
    },
    #[error(
        "jira-import.toml: idea_link_type is empty; --apply needs the link type joining ideas to epics on your site \
         (a rehearsal lists every link type it saw)"
    )]
    MissingIdeaLinkType,
    #[error(
        "jira-import.toml: discovery_project and delivery_project are both {0:?}; ideas and delivery issues must come \
         from different projects"
    )]
    SameProject(String),
    #[error(
        "jira-import.toml: {key} = {value:?} is not a Jira project key (A-Z, then A-Z, 0-9 or _)"
    )]
    BadProject { key: &'static str, value: String },
    /// `value` is left out when the URL carries credentials, so they never reach a log.
    #[error("jira-import.toml: {key} {why}{}", .value.as_ref().map(|v| format!(": {v:?}")).unwrap_or_default())]
    BadUrl {
        key: &'static str,
        value: Option<String>,
        why: &'static str,
    },
}

const DELIVERY: [&str; 5] = ["todo", "in_progress", "in_review", "testing", "done"];
const RESOLUTIONS: [&str; 2] = ["completed", "dropped"];
const KINDS: [&str; 3] = ["story", "bug", "chore"];

impl Mapping {
    pub fn parse(text: &str) -> Result<Self, MappingError> {
        toml::from_str(text).map_err(|e| MappingError::Parse(e.to_string()))
    }

    pub fn load(path: &std::path::Path) -> Result<Self, MappingError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| MappingError::Parse(format!("{}: {e}", path.display())))?;
        Self::parse(&text)
    }

    /// `--site` wins over the file's `site`.
    pub fn with_site_override(mut self, site: Option<&str>) -> Self {
        if let Some(s) = site {
            self.site = s.to_string();
        }
        self
    }

    /// The site without a trailing slash: the root of `/browse/<KEY>` links.
    pub fn site(&self) -> &str {
        self.site.trim_end_matches('/')
    }

    /// The REST base (`api_base`, else `site`) as a URL safe to send credentials to; see [`checked_url`].
    pub fn api_url(&self) -> Result<Url, MappingError> {
        match &self.api_base {
            Some(b) => checked_url("api_base", b),
            None => checked_url("site", &self.site),
        }
    }

    /// The site as a checked URL: the root of `/browse/<KEY>` links.
    pub fn site_url(&self) -> Result<Url, MappingError> {
        checked_url("site", &self.site)
    }

    pub fn browse_url(&self, key: &str) -> String {
        format!("{}/browse/{key}", self.site())
    }

    /// `buckets` are the instance's horizons (`GET /api/v1/config`).
    pub fn validate(&self, buckets: &[String], purpose: Purpose) -> Result<(), MappingError> {
        fn check(
            table: &str,
            map: &BTreeMap<String, String>,
            allowed: &[&str],
        ) -> Result<(), MappingError> {
            for (k, v) in map {
                if !allowed.contains(&v.as_str()) {
                    return Err(MappingError::Invalid {
                        key: format!("{table}.{k:?}"),
                        value: v.clone(),
                        expected: allowed.join("|"),
                    });
                }
            }
            Ok(())
        }
        let horizons: Vec<&str> = buckets.iter().map(String::as_str).collect();
        check("bucket_to_horizon", &self.bucket_to_horizon, &horizons)?;
        check("status_to_delivery", &self.status_to_delivery, &DELIVERY)?;
        check("resolution_map", &self.resolution_map, &RESOLUTIONS)?;
        check("type_to_kind", &self.type_to_kind, &KINDS)?;
        if !RESOLUTIONS.contains(&self.default_resolution.as_str()) {
            return Err(MappingError::Invalid {
                key: "default_resolution".into(),
                value: self.default_resolution.clone(),
                expected: RESOLUTIONS.join("|"),
            });
        }
        for (key, value) in [
            ("discovery_project", &self.discovery_project),
            ("delivery_project", &self.delivery_project),
        ] {
            if !project_key(value) {
                return Err(MappingError::BadProject {
                    key,
                    value: value.clone(),
                });
            }
        }
        // Planned items are looked up by bare Jira key, so the two projects must not share a key space.
        if self.discovery_project == self.delivery_project {
            return Err(MappingError::SameProject(self.discovery_project.clone()));
        }
        self.site_url()?;
        self.api_url()?;
        if purpose == Purpose::Apply && self.idea_link_type.trim().is_empty() {
            return Err(MappingError::MissingIdeaLinkType);
        }
        Ok(())
    }

    pub fn horizon(&self, bucket: &str) -> Option<&str> {
        self.bucket_to_horizon.get(bucket).map(String::as_str)
    }
    pub fn delivery(&self, status: &str) -> Option<DeliveryState> {
        self.status_to_delivery
            .get(status)
            .and_then(|v| enum_value(v))
    }
    pub fn resolution(&self, jira: &str) -> Option<Resolution> {
        self.resolution_map.get(jira).and_then(|v| enum_value(v))
    }
    pub fn default_resolution(&self) -> Resolution {
        enum_value(&self.default_resolution).unwrap_or(Resolution::Completed)
    }
    pub fn kind(&self, issue_type: &str) -> Option<Kind> {
        self.type_to_kind
            .get(issue_type)
            .and_then(|v| enum_value(v))
    }
}

/// A Jira project key: an uppercase letter, then uppercase letters or digits (it is quoted into JQL verbatim). The same
/// grammar as an imported item's reference (`KeyRef`): a key with `_` would rehearse and then fail every create.
fn project_key(k: &str) -> bool {
    let mut chars = k.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && k.len() >= 2
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// https with a host and no user name or password; plain http only to a loopback host (a local fake or proxy).
/// Basic auth sends the token with every request, so an http site would leak it before any redirect.
fn checked_url(key: &'static str, raw: &str) -> Result<Url, MappingError> {
    let bad = |why, value: Option<&str>| MappingError::BadUrl {
        key,
        value: value.map(String::from),
        why,
    };
    let url = Url::parse(raw.trim()).map_err(|_| bad("is not a URL", Some(raw)))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(bad("must not carry a user name or password", None));
    }
    if url.cannot_be_a_base() || url.host().is_none() {
        return Err(bad("needs a scheme and a host (https://<site>)", Some(raw)));
    }
    let host = url.host_str().unwrap_or("");
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    match url.scheme() {
        "https" => Ok(url),
        "http" if loopback => Ok(url),
        _ => Err(bad("must be https", Some(raw))),
    }
}

/// The protocol enums are snake_case on the wire; the mapping uses the same spelling.
fn enum_value<T: serde::de::DeserializeOwned>(v: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(v.to_string())).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> Mapping {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../jira-import.example.toml"
        ))
        .unwrap();
        Mapping::parse(&text).unwrap()
    }
    fn buckets() -> Vec<String> {
        ["Now", "Next", "Upcoming", "Later", "Won't do"]
            .map(String::from)
            .to_vec()
    }

    #[test]
    fn the_example_parses_and_its_values_map() {
        let mut m = example();
        m.idea_link_type = "Polaris work item link".into();
        m.validate(&buckets(), Purpose::Apply).unwrap();
        assert_eq!(
            m.delivery("Ready for review"),
            Some(DeliveryState::InReview)
        );
        assert_eq!(m.delivery("In testing"), Some(DeliveryState::Testing));
        assert_eq!(m.resolution("Won't Do"), Some(Resolution::Dropped));
        assert_eq!(m.kind("Task"), Some(Kind::Chore));
        assert_eq!(m.default_resolution(), Resolution::Completed);
        assert_eq!(
            m.api_url().unwrap().as_str(),
            "https://acme-team.atlassian.net/"
        );
    }

    #[test]
    fn an_unknown_delivery_value_is_refused_naming_the_key() {
        let mut m = example();
        m.status_to_delivery
            .insert("Blocked".into(), "blocked".into());
        let err = m.validate(&buckets(), Purpose::Rehearse).unwrap_err();
        assert_eq!(
            err,
            MappingError::Invalid {
                key: "status_to_delivery.\"Blocked\"".into(),
                value: "blocked".into(),
                expected: DELIVERY.join("|"),
            }
        );
        assert!(err.to_string().contains("status_to_delivery.\"Blocked\""));
    }

    #[test]
    fn a_horizon_outside_the_instance_buckets_is_refused() {
        let m = example();
        let err = m
            .validate(&["Now".into(), "Next".into()], Purpose::Rehearse)
            .unwrap_err();
        assert!(
            matches!(err, MappingError::Invalid { ref key, .. } if key.starts_with("bucket_to_horizon."))
        );
    }

    #[test]
    fn bad_resolutions_and_kinds_are_refused() {
        let mut m = example();
        m.resolution_map.insert("Done".into(), "finished".into());
        assert!(matches!(
            m.validate(&buckets(), Purpose::Rehearse),
            Err(MappingError::Invalid { ref key, .. }) if key == "resolution_map.\"Done\""
        ));
        let mut m = example();
        m.type_to_kind.insert("Spike".into(), "spike".into());
        assert!(matches!(
            m.validate(&buckets(), Purpose::Rehearse),
            Err(MappingError::Invalid { ref key, .. }) if key == "type_to_kind.\"Spike\""
        ));
        let mut m = example();
        m.default_resolution = "done".into();
        assert!(matches!(
            m.validate(&buckets(), Purpose::Rehearse),
            Err(MappingError::Invalid { ref key, .. }) if key == "default_resolution"
        ));
    }

    #[test]
    fn site_flag_wins_over_the_file() {
        let m = example().with_site_override(Some("https://other.atlassian.net/"));
        assert_eq!(m.site(), "https://other.atlassian.net");
        assert_eq!(
            m.api_url().unwrap().as_str(),
            "https://other.atlassian.net/"
        );
        assert_eq!(
            m.browse_url("SCRUM-1"),
            "https://other.atlassian.net/browse/SCRUM-1"
        );
        let kept = example().with_site_override(None);
        assert_eq!(kept.site(), "https://acme-team.atlassian.net");
    }

    #[test]
    fn api_base_is_kept_when_set() {
        let mut m = example();
        m.api_base = Some("https://api.atlassian.com/ex/jira/abc".into());
        assert_eq!(
            m.api_url().unwrap().as_str(),
            "https://api.atlassian.com/ex/jira/abc"
        );
        assert_eq!(m.site(), "https://acme-team.atlassian.net");
    }

    #[test]
    fn an_empty_idea_link_type_rehearses_but_does_not_apply() {
        let m = example();
        assert_eq!(m.idea_link_type, "");
        m.validate(&buckets(), Purpose::Rehearse).unwrap();
        let err = m.validate(&buckets(), Purpose::Apply).unwrap_err();
        assert_eq!(err, MappingError::MissingIdeaLinkType);
        assert!(err.to_string().contains("idea_link_type"));
    }

    #[test]
    fn unknown_keys_are_refused() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../jira-import.example.toml"
        ))
        .unwrap();
        let misspelled = text.replace("acceptance_heading", "acceptance_heading_typo");
        let Err(MappingError::Parse(msg)) = Mapping::parse(&misspelled) else {
            panic!("a misspelled key must be refused");
        };
        assert!(
            msg.contains("unknown field `acceptance_heading_typo`"),
            "{msg}"
        );
    }

    fn with_site(site: &str) -> Mapping {
        example().with_site_override(Some(site))
    }

    #[test]
    fn an_http_site_is_refused() {
        let err = with_site("http://acme.atlassian.net")
            .validate(&buckets(), Purpose::Rehearse)
            .unwrap_err();
        assert!(
            matches!(
                err,
                MappingError::BadUrl {
                    key: "site",
                    why: "must be https",
                    ..
                }
            ),
            "{err}"
        );
        let mut m = example();
        m.api_base = Some("http://api.atlassian.com/ex/jira/abc".into());
        assert!(matches!(
            m.api_url(),
            Err(MappingError::BadUrl {
                key: "api_base",
                ..
            })
        ));
    }

    #[test]
    fn a_site_without_a_scheme_is_refused_not_a_panic() {
        for raw in ["acme.atlassian.net:443", "acme.atlassian.net", "mailto:x@y"] {
            let err = with_site(raw).api_url().unwrap_err();
            assert!(
                matches!(err, MappingError::BadUrl { key: "site", .. }),
                "{raw}: {err}"
            );
        }
    }

    #[test]
    fn credentials_in_the_site_are_refused_and_never_echoed() {
        let err = with_site("https://me:secret@acme.atlassian.net")
            .site_url()
            .unwrap_err();
        assert!(matches!(err, MappingError::BadUrl { value: None, .. }));
        assert!(!err.to_string().contains("secret"));
    }

    #[test]
    fn loopback_http_is_allowed_for_a_local_fake() {
        assert!(with_site("http://127.0.0.1:8080").api_url().is_ok());
        assert!(with_site("http://localhost:8080").api_url().is_ok());
        assert_eq!(
            with_site("https://acme.atlassian.net")
                .api_url()
                .unwrap()
                .as_str(),
            "https://acme.atlassian.net/"
        );
    }

    /// Catches a mapping whose two projects share one key space, or a key that would break the JQL.
    #[test]
    fn projects_must_differ_and_be_project_keys() {
        let mut m = example();
        m.delivery_project = "JPD".into();
        assert_eq!(
            m.validate(&buckets(), Purpose::Rehearse),
            Err(MappingError::SameProject("JPD".into()))
        );
        for bad in ["", "P", "jpd", "JP\"D", "1AB", "AB CD", "SCRUM_2"] {
            let mut m = example();
            m.delivery_project = bad.into();
            assert_eq!(
                m.validate(&buckets(), Purpose::Rehearse),
                Err(MappingError::BadProject {
                    key: "delivery_project",
                    value: bad.into()
                }),
                "{bad:?}"
            );
        }
        let mut m = example();
        m.delivery_project = "SCRUM2".into();
        m.validate(&buckets(), Purpose::Rehearse).unwrap();
    }
}
