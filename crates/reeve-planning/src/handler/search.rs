//! Search items by text: is there already an item about X? Titles, briefs, descriptions and external
//! references, across every level, newest first.
use super::*;
use reeve_protocol::SearchHit;

/// At most this many hits: a search narrows, it does not list.
const LIMIT: i64 = 50;

impl Handlers {
    pub async fn search(&self, q: &str) -> Result<Vec<SearchHit>, HandlerError> {
        let q = q.trim();
        if q.is_empty() || q.chars().count() > 200 {
            return Err(HandlerError::Validation(
                "search text must be 1..=200 characters".into(),
            ));
        }
        let needle = q.to_lowercase();
        let has = |s: &str| s.to_lowercase().contains(&needle);
        Ok(self
            .repo
            .search_items(q, LIMIT)
            .await?
            .into_iter()
            .map(|r| {
                let mut matched = vec![];
                if has(&r.title) {
                    matched.push("title".to_string());
                }
                if has(&r.text) {
                    matched.push(match r.item {
                        ItemType::Story => "description".to_string(),
                        _ => "brief".to_string(),
                    });
                }
                if r.external_ref.as_deref().is_some_and(has) {
                    matched.push("external_ref".to_string());
                }
                SearchHit {
                    key: r.key,
                    item: r.item,
                    kind: r.kind,
                    title: r.title,
                    matched,
                }
            })
            .collect())
    }
}
