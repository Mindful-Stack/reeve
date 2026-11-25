//! Web links (kb/ADR-0031 R5) in the handlers: adding, removing and reading them. Informational: nothing here is
//! read by a move or by readiness (R3; the isolation lint bans these names in `handler/moves.rs`), and no item's
//! version changes. Any caller who may edit an item may add or remove its web links, whatever its token kind (R7).
use super::*;
use crate::domain::web_link::{WebLink, WebLinkError};
use crate::ports::StoredWebLink;
use reeve_protocol::WebLinkDto;
use sqlx::{Postgres, Transaction};

/// Which stored row `link` repeats, among the web links of one item (`stored`; the caller passes one item's rows):
/// `Ok(None)` when it is new there, `Ok(Some(row))` when that row has the same URL under the same title, and
/// `WebLink::is_new_among`'s refusal when the URL is there under another title. The one place a web link is matched to
/// its row: adding one (`add_web_link_in`) and plan apply's pre-read both ask here.
pub(super) fn repeat_among<'a>(
    link: &WebLink,
    stored: &'a [StoredWebLink],
) -> Result<Option<&'a StoredWebLink>, WebLinkError> {
    if link.is_new_among(stored.iter().map(|s| &s.link))? {
        return Ok(None);
    }
    Ok(stored.iter().find(|s| s.link == *link))
}

impl Handlers {
    /// `add_web_link`, answering the web link as stored as well (its id, its trimmed title and its URL as the URL
    /// parser writes it): the row this call wrote (true), or the row that was already there under the same title
    /// (false). Public for the server's action, whose MCP receipt names the web link by the id it is removed with
    /// and reports its title and URL from this answer, never from a read after the commit.
    pub async fn add_web_link_reporting(
        &self,
        c: &Ctx,
        item: &Key,
        title: &str,
        url: &str,
    ) -> Result<(WebLinkDto, bool), HandlerError> {
        let link = WebLink::new(title, url)?;
        let level = self.item_type_of(item).await?;
        let mut tx = self.pool.begin().await?;
        let answer = self.add_web_link_in(&mut tx, c, level, item, &link).await?;
        tx.commit().await?;
        Ok(answer)
    }

    /// Attaches a titled URL to an item of any level. A bad title or URL is a validation error; the item must
    /// exist (NotFound otherwise). Answers whether this call added it: the same URL with the same title is already
    /// there and changes nothing; the same URL with another title is a validation error that says to remove it
    /// first (a web link is not edited in place). Logged as `web_link_added` on the item, in the same transaction;
    /// no version moves.
    pub async fn add_web_link(
        &self,
        c: &Ctx,
        item: &Key,
        title: &str,
        url: &str,
    ) -> Result<bool, HandlerError> {
        self.add_web_link_reporting(c, item, title, url)
            .await
            .map(|(_, added)| added)
    }

    /// Inside the caller's transaction: attaches `link` to `item` and, when this call added it, records
    /// `web_link_added` on the item, last. `item` is the item's key and `level` its level; the caller has resolved
    /// both and validated the link (`WebLink::new`). Nothing here reads through the pool, so it also serves a
    /// caller whose item is not committed yet, and it neither commits nor rolls back. It answers the web link as
    /// stored (its id is how one is removed) and whether this call added it: the same URL with the same title
    /// answers the row that is there, false, and writes nothing; with another title it is `WebLink::is_new_among`'s refusal as a
    /// validation error, and what the caller does with its transaction then is the caller's. It passes
    /// `is_new_among` the item's stored web links; that method takes the web links of one item, whatever their
    /// source. Plan apply (GRA-879) composes it with the plan's other writes.
    pub(super) async fn add_web_link_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        c: &Ctx,
        level: ItemType,
        item: &Key,
        link: &WebLink,
    ) -> Result<(WebLinkDto, bool), HandlerError> {
        let Some(id) = self.ports.web_links.add_web_link(tx, item, link).await? else {
            // The item already has this URL: the same title is a repeat, another title is not an edit. The rows
            // read are this one item's, which is what the rule is over.
            let stored = self
                .ports
                .web_links
                .web_links_of_in(tx, std::slice::from_ref(item))
                .await?;
            // Not refused and a repeat: that row is the answer. No repeat when it was removed between the
            // insert and the read: nothing was added, so say so.
            return match repeat_among(link, &stored)? {
                Some(there) => Ok((super::dto::web_link_dto(there), false)),
                None => Err(HandlerError::Conflict { current: -1 }),
            };
        };
        self.activity
            .record(
                tx,
                self.rec(
                    c,
                    item_name(level),
                    &item.to_string(),
                    "web_link_added",
                    serde_json::json!({"id": id, "title": link.title(), "url": link.url()}),
                ),
            )
            .await?;
        // The row as the insert wrote it: the link's own title and URL, already in their stored form.
        Ok((
            WebLinkDto {
                id,
                title: link.title().to_owned(),
                url: link.url().to_owned(),
            },
            true,
        ))
    }

    /// Removes one web link of an item by its id. The item must exist; an id the item does not have is a no-op
    /// that logs nothing. Logged as `web_link_removed` on the item with the title and URL it had.
    pub async fn remove_web_link(
        &self,
        c: &Ctx,
        item: &Key,
        id: i64,
    ) -> Result<bool, HandlerError> {
        let level = self.item_type_of(item).await?;
        let mut tx = self.pool.begin().await?;
        let removed = self.remove_web_link_in(&mut tx, c, level, item, id).await?;
        tx.commit().await?;
        Ok(removed)
    }

    /// Inside the caller's transaction: removes the web link `id` of `item` and, when the item had it, records
    /// `web_link_removed` with the title and URL it had, last. The same contract as `add_web_link_in`: the caller
    /// brings the item's key and level, nothing is read through the pool, and the transaction is neither committed
    /// nor rolled back here.
    pub(super) async fn remove_web_link_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        c: &Ctx,
        level: ItemType,
        item: &Key,
        id: i64,
    ) -> Result<bool, HandlerError> {
        let Some(link) = self.ports.web_links.remove_web_link(tx, item, id).await? else {
            return Ok(false);
        };
        self.activity
            .record(
                tx,
                self.rec(
                    c,
                    item_name(level),
                    &item.to_string(),
                    "web_link_removed",
                    serde_json::json!({"id": id, "title": link.title(), "url": link.url()}),
                ),
            )
            .await?;
        Ok(true)
    }

    /// One item's web links, oldest first, and nothing else, from one read-only snapshot. NotFound when no item has
    /// the key. The answer of a write that changed a web link (the POST and the DELETE): it loads no item link.
    pub async fn web_links_of(&self, key: &Key) -> Result<Vec<WebLinkDto>, HandlerError> {
        self.item_type_of(key).await?;
        let mut tx = self
            .pool
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        let out = self.web_link_dtos_in(&mut tx, key).await;
        end_read(tx, out).await
    }

    /// The web links of one item on the caller's transaction, oldest first. One statement.
    pub(crate) async fn web_link_dtos_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        key: &Key,
    ) -> Result<Vec<WebLinkDto>, HandlerError> {
        Ok(self
            .ports
            .web_links
            .web_links_of_in(tx, std::slice::from_ref(key))
            .await?
            .iter()
            .map(super::dto::web_link_dto)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, title: &str, url: &str) -> StoredWebLink {
        StoredWebLink {
            id,
            item: "ABC-1".parse().unwrap(),
            link: WebLink::new(title, url).unwrap(),
        }
    }

    /// New among none and among other URLs; a repeat answers the row with the same URL and title (whatever the
    /// spelling of the URL); the same URL under another title is the refusal, never a row.
    #[test]
    fn repeat_among_finds_the_row_a_web_link_repeats() {
        let stored = [
            row(1, "Spec", "https://example.com/spec"),
            row(2, "Board", "https://example.com/board"),
        ];
        let spec = WebLink::new("Spec", "HTTPS://Example.com/spec").unwrap();
        assert_eq!(repeat_among(&spec, &[]), Ok(None));
        assert_eq!(repeat_among(&spec, &stored[1..]), Ok(None));
        assert_eq!(repeat_among(&spec, &stored), Ok(Some(&stored[0])));
        let renamed = WebLink::new("The spec", "https://example.com/spec").unwrap();
        assert_eq!(
            repeat_among(&renamed, &stored),
            Err(WebLinkError::SameUrlOtherTitle {
                titled: "Spec".to_owned()
            })
        );
    }
}
