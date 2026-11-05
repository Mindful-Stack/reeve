//! `reeve search <text>`: is there already an item about this?
use super::finish;
use crate::client::Client;
use crate::glyphs::{Level, mark};
use clap::Args;
use reeve_protocol::*;

/// Find initiatives, epics and stories whose title, brief, description or Jira reference contains the text
/// (case-insensitive), newest first.
#[derive(Args)]
pub struct Search {
    /// The text; several words are searched as one phrase.
    #[arg(required = true, num_args = 1..)]
    pub text: Vec<String>,
}

pub async fn run(c: &Client, s: Search, json: bool) -> anyhow::Result<i32> {
    let q = s.text.join(" ");
    Ok(finish(
        c.get_query::<Vec<SearchHit>>("/search", &[("q", q)])
            .await?,
        json,
        |h| render(h),
    ))
}

fn render(hits: &[SearchHit]) -> String {
    if hits.is_empty() {
        return "nothing found".into();
    }
    hits.iter()
        .map(|h| {
            let level = match (h.item, h.kind) {
                (ItemType::Story, Some(k)) => Level::of_kind(k),
                (t, _) => Level::of_item(t),
            };
            format!(
                "{}\t{}\t{}",
                mark(level, &h.key),
                h.title,
                h.matched.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
