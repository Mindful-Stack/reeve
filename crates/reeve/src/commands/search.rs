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
                crate::terminal::line(&h.title),
                h.matched
                    .iter()
                    .map(|m| crate::terminal::line(m))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hostile::{Paths, assert_one_line, assert_shown};

    fn hit(item: ItemType, kind: Option<Kind>) -> SearchHit {
        SearchHit {
            key: "ABC-1".parse().unwrap(),
            item,
            kind,
            title: "Booking".into(),
            matched: vec!["title".into(), "external_ref".into()],
            exact: false,
        }
    }

    /// A hit is one tab-separated row: a title or a matched field's name cannot add a row or a column.
    #[test]
    fn a_hits_title_stays_in_its_column() {
        let made = [(ItemType::Story, Some(Kind::Bug)), (ItemType::Epic, None)];
        let hits: Vec<SearchHit> = made.iter().map(|&(item, kind)| hit(item, kind)).collect();
        // A hit made without a kind leaves it empty: an initiative or an epic has none.
        let paths = made
            .iter()
            .enumerate()
            .filter(|(_, (_, kind))| kind.is_none())
            .fold(Paths::none(), |p, (i, _)| {
                p.absent(
                    &format!("/{i}/kind"),
                    "an initiative or an epic has no kind",
                )
            });
        assert_shown(
            &assert_one_line(&hits, &paths, |h| render(h)),
            &["/0/title", "/0/matched/0", "/0/matched/1", "/1/title"],
        );
        let row = render(&[SearchHit {
            title: "a\tb".into(),
            matched: vec![],
            ..hit(ItemType::Epic, None)
        }]);
        assert_eq!(row.matches('\t').count(), 2, "{row:?}");
    }
}
