//! Search items by text: is there already an item about X? Titles, briefs, descriptions and external
//! references, across every level. A query that names a key (`12`, `#12`, `abc-12`) finds that item first; then an
//! exact external reference, then titles, then the rest, newest first within each.
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
        let key = self.key_named_by(q);
        let needle = q.to_lowercase();
        let has = |s: &str| s.to_lowercase().contains(&needle);
        Ok(self
            .repo
            .search_items(q, key.as_ref(), LIMIT)
            .await?
            .into_iter()
            .map(|r| {
                let mut matched = vec![];
                if key.as_ref() == Some(&r.key) {
                    matched.push("key".to_string());
                }
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

    /// The key a query names: a bare number or `#n` under the current prefix, or a key in any case (`abc-12`, an
    /// earlier prefix's `old-3`).
    fn key_named_by(&self, q: &str) -> Option<Key> {
        let n = q.strip_prefix('#').unwrap_or(q);
        if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) {
            return format!("{}-{n}", self.settings.key_prefix).parse().ok();
        }
        q.to_ascii_uppercase().parse().ok()
    }
}
