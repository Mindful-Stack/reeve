// The consumer-defined activity port (kb/ADR-0010 R2, R5)
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};

#[derive(Debug, Clone)]
pub struct IdentityEvent {
    pub at: DateTime<Utc>,
    pub actor: String,
    pub via: reeve_protocol::Via,
    pub subject: String,
    pub action: &'static str,
    pub payload: serde_json::Value,
}

#[async_trait]
pub trait IdentityActivity: Send + Sync {
    async fn record(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        e: IdentityEvent,
    ) -> Result<(), sqlx::Error>;
}
