//! The SQL behind `WebLinkRepo` (kb/ADR-0031 R5). Only `web_links` is named here: no item table, no version, and
//! nothing reads the clock.
use super::*;
use crate::domain::web_link::WebLink;

#[async_trait]
impl WebLinkRepo for PgPlanningRepo {
    async fn add_web_link(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: &Key,
        link: &WebLink,
    ) -> Result<Option<i64>, RepoError> {
        Ok(sqlx::query_scalar(
            "INSERT INTO web_links (item_key, title, url) VALUES ($1, $2, $3) \
             ON CONFLICT (item_key, url) DO NOTHING RETURNING id",
        )
        .bind(item.to_string())
        .bind(link.title())
        .bind(link.url())
        .fetch_optional(&mut **tx)
        .await?)
    }
    async fn remove_web_link(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        item: &Key,
        id: i64,
    ) -> Result<Option<WebLink>, RepoError> {
        let row: Option<(String, String)> = sqlx::query_as(
            "DELETE FROM web_links WHERE item_key = $1 AND id = $2 RETURNING title, url",
        )
        .bind(item.to_string())
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.map(|(title, url)| WebLink::from_persisted(title, url)))
    }
    async fn web_links_of_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        items: &[Key],
    ) -> Result<Vec<StoredWebLink>, RepoError> {
        let rows: Vec<(i64, String, String, String)> = sqlx::query_as(
            "SELECT id, item_key, title, url FROM web_links WHERE item_key = ANY($1) ORDER BY id",
        )
        .bind(items.iter().map(Key::to_string).collect::<Vec<_>>())
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, item, title, url)| StoredWebLink {
                id,
                item: key(&item),
                link: WebLink::from_persisted(title, url),
            })
            .collect())
    }
    async fn all_web_links_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
    ) -> Result<Vec<StoredWebLink>, RepoError> {
        let rows: Vec<(i64, String, String, String)> =
            sqlx::query_as("SELECT id, item_key, title, url FROM web_links")
                .fetch_all(&mut **tx)
                .await?;
        Ok(rows
            .into_iter()
            .map(|(id, item, title, url)| StoredWebLink {
                id,
                item: key(&item),
                link: WebLink::from_persisted(title, url),
            })
            .collect())
    }
}
