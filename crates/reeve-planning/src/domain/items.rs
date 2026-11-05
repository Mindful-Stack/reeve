use chrono::{DateTime, Utc};
use reeve_protocol::{DeliveryState, Key, Kind, Resolution};

#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveryMark {
    pub at: DateTime<Utc>,
    pub by: String,
}

/// Private fields; constructed only by `new` (validated) or `from_persisted` (trusted rows).
#[derive(Debug, Clone, PartialEq)]
pub struct Initiative {
    key: Key,
    title: String,
    brief: String,
    horizon: String,
    target_month: Option<String>,
    owner: Option<String>,
    discovery: Option<DiscoveryMark>,
    external_ref: Option<String>,
    version: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Epic {
    key: Key,
    title: String,
    initiative: Option<Key>,
    brief: String,
    discovery: Option<DiscoveryMark>,
    external_ref: Option<String>,
    version: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Criterion {
    pub text: String,
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Story {
    key: Key,
    title: String,
    kind: Kind,
    epic: Option<Key>,
    description: String,
    criteria: Vec<Criterion>,
    owner: Option<String>,
    labels: Vec<String>,
    rank: i32,
    delivery: DeliveryState,
    resolution: Option<Resolution>,
    external_ref: Option<String>,
    version: i64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ItemError {
    #[error("title must be 1..=200 characters")]
    Title,
    #[error("horizon {0:?} is not one of the configured buckets")]
    Horizon(String),
    #[error("a criterion must not be blank")]
    BlankCriterion,
}

pub(crate) fn check_title(t: &str) -> Result<(), ItemError> {
    let n = t.trim().chars().count();
    if (1..=200).contains(&n) {
        Ok(())
    } else {
        Err(ItemError::Title)
    }
}

/// A horizon the initiative already has is always valid (its bucket may have been unlisted since); a different one
/// must be a configured bucket.
pub fn check_horizon_change(
    current: &str,
    next: &str,
    buckets: &[String],
) -> Result<(), ItemError> {
    if next == current || buckets.iter().any(|b| b == next) {
        Ok(())
    } else {
        Err(ItemError::Horizon(next.into()))
    }
}

impl Initiative {
    pub fn new(
        key: Key,
        title: &str,
        brief: &str,
        horizon: &str,
        buckets: &[String],
        target_month: Option<String>,
        owner: Option<String>,
    ) -> Result<Self, ItemError> {
        check_title(title)?;
        if !buckets.iter().any(|b| b == horizon) {
            return Err(ItemError::Horizon(horizon.into()));
        }
        Ok(Self {
            key,
            title: title.trim().into(),
            brief: brief.into(),
            horizon: horizon.into(),
            target_month,
            owner,
            discovery: None,
            external_ref: None,
            version: 1,
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub fn from_persisted(
        key: Key,
        title: String,
        brief: String,
        horizon: String,
        target_month: Option<String>,
        owner: Option<String>,
        discovery: Option<DiscoveryMark>,
        external_ref: Option<String>,
        version: i64,
    ) -> Self {
        Self {
            key,
            title,
            brief,
            horizon,
            target_month,
            owner,
            discovery,
            external_ref,
            version,
        }
    }
    pub fn key(&self) -> &Key {
        &self.key
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn brief(&self) -> &str {
        &self.brief
    }
    pub fn horizon(&self) -> &str {
        &self.horizon
    }
    pub fn target_month(&self) -> Option<&str> {
        self.target_month.as_deref()
    }
    pub fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }
    pub fn discovery(&self) -> Option<&DiscoveryMark> {
        self.discovery.as_ref()
    }
    /// The origin a hand-created item records (a Jira key it was migrated from); validated by the handler.
    pub fn with_external_ref(mut self, external_ref: Option<String>) -> Self {
        self.external_ref = external_ref;
        self
    }
    pub fn external_ref(&self) -> Option<&str> {
        self.external_ref.as_deref()
    }
    pub fn version(&self) -> i64 {
        self.version
    }
}

impl Epic {
    pub fn new(
        key: Key,
        title: &str,
        initiative: Option<Key>,
        brief: &str,
    ) -> Result<Self, ItemError> {
        check_title(title)?;
        Ok(Self {
            key,
            title: title.trim().into(),
            initiative,
            brief: brief.into(),
            discovery: None,
            external_ref: None,
            version: 1,
        })
    }
    pub fn from_persisted(
        key: Key,
        title: String,
        initiative: Option<Key>,
        brief: String,
        discovery: Option<DiscoveryMark>,
        external_ref: Option<String>,
        version: i64,
    ) -> Self {
        Self {
            key,
            title,
            initiative,
            brief,
            discovery,
            external_ref,
            version,
        }
    }
    pub fn key(&self) -> &Key {
        &self.key
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn initiative(&self) -> Option<&Key> {
        self.initiative.as_ref()
    }
    pub fn brief(&self) -> &str {
        &self.brief
    }
    pub fn discovery(&self) -> Option<&DiscoveryMark> {
        self.discovery.as_ref()
    }
    /// The origin a hand-created item records (a Jira key it was migrated from); validated by the handler.
    pub fn with_external_ref(mut self, external_ref: Option<String>) -> Self {
        self.external_ref = external_ref;
        self
    }
    pub fn external_ref(&self) -> Option<&str> {
        self.external_ref.as_deref()
    }
    pub fn version(&self) -> i64 {
        self.version
    }
}

impl Story {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        key: Key,
        title: &str,
        kind: Kind,
        epic: Option<Key>,
        description: &str,
        criteria: Vec<String>,
        owner: Option<String>,
        labels: Vec<String>,
    ) -> Result<Self, ItemError> {
        check_title(title)?;
        if criteria.iter().any(|c| c.trim().is_empty()) {
            return Err(ItemError::BlankCriterion);
        }
        Ok(Self {
            key,
            title: title.trim().into(),
            kind,
            epic,
            description: description.into(),
            criteria: criteria
                .into_iter()
                .map(|t| Criterion {
                    text: t.trim().into(),
                    done: false,
                })
                .collect(),
            owner,
            labels,
            rank: 0,
            delivery: DeliveryState::Todo,
            resolution: None,
            external_ref: None,
            version: 1,
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub fn from_persisted(
        key: Key,
        title: String,
        kind: Kind,
        epic: Option<Key>,
        description: String,
        criteria: Vec<Criterion>,
        owner: Option<String>,
        labels: Vec<String>,
        rank: i32,
        delivery: DeliveryState,
        resolution: Option<Resolution>,
        external_ref: Option<String>,
        version: i64,
    ) -> Self {
        Self {
            key,
            title,
            kind,
            epic,
            description,
            criteria,
            owner,
            labels,
            rank,
            delivery,
            resolution,
            external_ref,
            version,
        }
    }
    pub fn key(&self) -> &Key {
        &self.key
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn kind(&self) -> Kind {
        self.kind
    }
    pub fn epic(&self) -> Option<&Key> {
        self.epic.as_ref()
    }
    pub fn description(&self) -> &str {
        &self.description
    }
    pub fn criteria(&self) -> &[Criterion] {
        &self.criteria
    }
    pub fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }
    pub fn labels(&self) -> &[String] {
        &self.labels
    }
    pub fn rank(&self) -> i32 {
        self.rank
    }
    pub fn delivery(&self) -> DeliveryState {
        self.delivery
    }
    pub fn resolution(&self) -> Option<Resolution> {
        self.resolution
    }
    /// The origin a hand-created item records (a Jira key it was migrated from); validated by the handler.
    pub fn with_external_ref(mut self, external_ref: Option<String>) -> Self {
        self.external_ref = external_ref;
        self
    }
    pub fn external_ref(&self) -> Option<&str> {
        self.external_ref.as_deref()
    }
    pub fn version(&self) -> i64 {
        self.version
    }
    pub fn is_untriaged(&self) -> bool {
        self.kind == Kind::Story && self.epic.is_none()
    }
}

/// `progress` is None; the handler fills it from story counts.
impl From<&Initiative> for reeve_protocol::InitiativeDto {
    fn from(i: &Initiative) -> Self {
        Self {
            key: i.key.clone(),
            title: i.title.clone(),
            brief: i.brief.clone(),
            horizon: i.horizon.clone(),
            target_month: i.target_month.clone(),
            owner: i.owner.clone(),
            discovery_complete: i
                .discovery
                .as_ref()
                .map(|d| reeve_protocol::DiscoveryMarkDto {
                    at: d.at,
                    by: d.by.clone(),
                }),
            progress: None,
            external_ref: i.external_ref.clone(),
            version: i.version,
        }
    }
}
impl From<&Epic> for reeve_protocol::EpicDto {
    fn from(e: &Epic) -> Self {
        Self {
            key: e.key.clone(),
            title: e.title.clone(),
            initiative: e.initiative.clone(),
            brief: e.brief.clone(),
            discovery_complete: e
                .discovery
                .as_ref()
                .map(|d| reeve_protocol::DiscoveryMarkDto {
                    at: d.at,
                    by: d.by.clone(),
                }),
            external_ref: e.external_ref.clone(),
            version: e.version,
        }
    }
}
/// `blocked_by`, `prs` and `goals` are empty; the handler fills them from the repo.
impl From<&Story> for reeve_protocol::StoryDto {
    fn from(s: &Story) -> Self {
        Self {
            key: s.key.clone(),
            title: s.title.clone(),
            kind: s.kind,
            epic: s.epic.clone(),
            description: s.description.clone(),
            criteria: s
                .criteria
                .iter()
                .map(|c| reeve_protocol::CriterionDto {
                    text: c.text.clone(),
                    done: c.done,
                })
                .collect(),
            owner: s.owner.clone(),
            labels: s.labels.clone(),
            rank: s.rank,
            blocked_by: vec![],
            delivery: s.delivery,
            resolution: s.resolution,
            external_ref: s.external_ref.clone(),
            version: s.version,
            prs: vec![],
            goals: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_validates_title_and_horizon() {
        let b = vec!["Now".to_string(), "Later".to_string()];
        assert_eq!(
            Initiative::new("ABC-1".parse().unwrap(), " ", "", "Now", &b, None, None).unwrap_err(),
            ItemError::Title
        );
        assert!(matches!(
            Initiative::new("ABC-1".parse().unwrap(), "x", "", "Soon", &b, None, None).unwrap_err(),
            ItemError::Horizon(_)
        ));
        assert_eq!(
            Story::new(
                "ABC-2".parse().unwrap(),
                "t",
                Kind::Story,
                None,
                "",
                vec!["ok".into(), " ".into()],
                None,
                vec![]
            )
            .unwrap_err(),
            ItemError::BlankCriterion
        );
    }
    #[test]
    fn a_kept_horizon_is_valid_and_a_changed_one_must_be_a_bucket() {
        let b = vec!["Soon".to_string(), "Later".to_string()];
        assert_eq!(check_horizon_change("Now", "Now", &b), Ok(()));
        assert_eq!(check_horizon_change("Now", "Soon", &b), Ok(()));
        assert_eq!(
            check_horizon_change("Now", "Someday", &b),
            Err(ItemError::Horizon("Someday".into()))
        );
        assert_eq!(check_horizon_change("Soon", "Soon", &[]), Ok(()));
    }
    #[test]
    fn title_and_criteria_are_trimmed() {
        let s = Story::new(
            "ABC-2".parse().unwrap(),
            "  t  ",
            Kind::Story,
            None,
            "",
            vec!["  ok ".into()],
            None,
            vec![],
        )
        .unwrap();
        assert_eq!(s.title(), "t");
        assert_eq!(s.criteria()[0].text, "ok");
    }
    #[test]
    fn title_length_boundary() {
        let new = |t: &str| Epic::new("ABC-2".parse().unwrap(), t, None, "");
        assert!(new(&"a".repeat(200)).is_ok());
        assert_eq!(new(&"a".repeat(201)).unwrap_err(), ItemError::Title);
    }
    #[test]
    fn horizon_match_is_exact() {
        let b = vec!["Now".to_string()];
        for h in ["now", " Now"] {
            assert!(matches!(
                Initiative::new("ABC-1".parse().unwrap(), "x", "", h, &b, None, None).unwrap_err(),
                ItemError::Horizon(_)
            ));
        }
    }
}
