use chrono::{DateTime, Utc};

pub use reeve_protocol::Via;

pub trait ViaStr {
    fn as_str(&self) -> &'static str;
    fn parse(s: &str) -> Option<Via>;
}
impl ViaStr for Via {
    fn as_str(&self) -> &'static str {
        match *self {
            Via::Interactive => "interactive",
            Via::Automation => "automation",
            Via::System => "system",
        }
    }
    fn parse(s: &str) -> Option<Via> {
        match s {
            "interactive" => Some(Via::Interactive),
            "automation" => Some(Via::Automation),
            "system" => Some(Via::System),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActivityEntry {
    pub id: Option<i64>, // converted to reeve_protocol::ActivityEntryDto by the server; the crate owns no wire type
    pub at: DateTime<Utc>,
    pub actor: String,
    pub via: Via,
    pub entity_type: String,
    pub entity_key: String,
    pub action: String,
    pub payload: serde_json::Value,
}

impl ActivityEntry {
    pub fn new(
        at: DateTime<Utc>,
        actor: &str,
        via: Via,
        entity_type: &str,
        entity_key: &str,
        action: &str,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            id: None,
            at,
            actor: actor.into(),
            via,
            entity_type: entity_type.into(),
            entity_key: entity_key.into(),
            action: action.into(),
            payload,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ActivityFilter {
    pub entity_type: Option<String>,
    pub entity_key: Option<String>,
    pub action: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
    /// Only entries with a greater id, oldest first (a stream replays in order); `None` lists newest first.
    pub after_id: Option<i64>,
}
