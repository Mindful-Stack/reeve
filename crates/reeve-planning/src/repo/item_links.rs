//! The SQL behind `ItemLinkRepo` (kb/ADR-0031). Only `item_links` is named here: no item table, no version, and
//! nothing reads the clock.
use super::*;
use crate::domain::item_link::ItemLink;

#[async_trait]
impl ItemLinkRepo for PgPlanningRepo {
    async fn add_item_link(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        link: &ItemLink,
    ) -> Result<bool, RepoError> {
        let r = sqlx::query(
            "INSERT INTO item_links (from_key, to_key, link_type) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(link.from_key().to_string())
        .bind(link.to_key().to_string())
        .bind(link.link_type().wire())
        .execute(&mut **tx)
        .await?;
        Ok(r.rows_affected() == 1)
    }
    async fn remove_item_link(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        link: &ItemLink,
    ) -> Result<bool, RepoError> {
        let r = sqlx::query(
            "DELETE FROM item_links WHERE from_key = $1 AND to_key = $2 AND link_type = $3",
        )
        .bind(link.from_key().to_string())
        .bind(link.to_key().to_string())
        .bind(link.link_type().wire())
        .execute(&mut **tx)
        .await?;
        Ok(r.rows_affected() == 1)
    }
    async fn item_links_of_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        items: &[Key],
    ) -> Result<Vec<ItemLink>, RepoError> {
        let rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT from_key, to_key, link_type FROM item_links \
             WHERE from_key = ANY($1) OR to_key = ANY($1) ORDER BY from_key, to_key, link_type",
        )
        .bind(items.iter().map(Key::to_string).collect::<Vec<_>>())
        .fetch_all(&mut **tx)
        .await?;
        rows.into_iter()
            .map(|(from, to, name)| -> Result<ItemLink, RepoError> {
                let link_type = reeve_protocol::ItemLinkType::from_wire(&name)
                    .ok_or_else(|| bad("item link name", &name))?;
                Ok(ItemLink::from_persisted(key(&from), link_type, key(&to)))
            })
            .collect()
    }
    async fn all_item_links_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<ItemLink>, RepoError> {
        let rows: Vec<(String, String, String)> =
            sqlx::query_as("SELECT from_key, to_key, link_type FROM item_links")
                .fetch_all(&mut **tx)
                .await?;
        rows.into_iter()
            .map(|(from, to, name)| -> Result<ItemLink, RepoError> {
                let link_type = reeve_protocol::ItemLinkType::from_wire(&name)
                    .ok_or_else(|| bad("item link name", &name))?;
                Ok(ItemLink::from_persisted(key(&from), link_type, key(&to)))
            })
            .collect()
    }
}
