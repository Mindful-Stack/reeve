//! `reeve apply <file>`: a plan of items, required decisions, blockers, item links and web links, sent as one
//! request.
use super::finish;
use crate::client::Client;
use crate::glyphs::{Level, mark};
use crate::terminal::line;
use anyhow::Context;
use clap::Args;
use reeve_protocol::*;
use std::fmt::Write;

/// Create a plan of items, required decisions, blockers, item links and web links from a YAML or JSON file.
///
/// A plan lists initiatives, epics, stories, required decisions and blockers, and may relate its items with item
/// links and web links. Items name each other by local `name`.
/// A reference that is not a plan name is an existing item's key or external reference, looked up at the level its
/// place takes: a story under `blocks`, an epic under `epic`, an initiative under `initiative`, any level (native keys
/// first) under `links`, `web_links` and a decision's `on`; so one spelling can name different items in different
/// places. The server validates the whole plan before writing, and applying the same plan `id` again creates nothing.
/// Writes commit one by one; if one fails, apply the same `id` again once the cause is corrected.
///
/// A plan may also relate its items: `links` is a list of `{from, type, to}` between items of any level, with type
/// follows, duplicates or relates_to, and `web_links` is a list of `{on, title, url}`. `from` is the item that
/// follows: `{from: step-2, type: follows, to: step-1}` means step-2 comes after step-1. A plan holds at most 500
/// item links and 500 web links; put the rest in another plan that names the items by key (a name is local to one
/// plan id).
///
/// Item links are informational: to hold a story back, list it under `blocks` as `{story, blocked_by}`. A plan
/// written from another tool's data puts a blocking relation under `blocks` or leaves it out, never under `links`
/// (kb/ADR-0031 R2).
#[derive(Args)]
pub struct Apply {
    /// The plan, YAML or JSON; `-` reads stdin.
    pub file: std::path::PathBuf,
}

/// What text mode adds when the answer holds a `follows` between two stories, created by this apply or there
/// already: the word reads like a dependency. Not only when created: an apply that made the link and failed later
/// printed nothing, and the apply that finishes the plan reports the link existing.
const FOLLOWS_NOTE: &str =
    "Informational: follows does not hold a story back. To hold one back, list it under blocks.";

/// The answer to a blocking word written as an item link's `type` in a plan file: the plan's form of the pointer
/// `reeve link` gives (kb/ADR-0031 R2). A blocker is between two stories, so the relation of anything else has no
/// place in a plan and is left out, never written as an item link.
const BLOCKING_IN_A_PLAN: &str = "blocking is not an item link: list it under `blocks` as `{story, blocked_by}` \
                                  (stories only; otherwise leave it out)";

pub async fn run(c: &Client, a: Apply, json: bool) -> anyhow::Result<i32> {
    let text = super::text_arg(None, Some(a.file.clone()))?.unwrap_or_default();
    // Parsed into the wire type here, so a misspelt field is an error before anything is sent. YAML reads JSON too.
    let plan: ApplyPlan = match serde_yaml::from_str(&text) {
        Ok(plan) => plan,
        // A blocking word is no item link name, so the typed parse refuses it; say where blocking goes in a plan
        // instead of only listing the three names.
        Err(_) if names_blocking_as_a_link(&text) => {
            anyhow::bail!(
                "reading the plan in {}: {BLOCKING_IN_A_PLAN}",
                a.file.display()
            )
        }
        Err(e) => {
            return Err(e).with_context(|| format!("reading the plan in {}", a.file.display()));
        }
    };
    Ok(finish(
        c.post::<PlanApplied>("/plans", serde_json::to_value(plan)?)
            .await?,
        json,
        render,
    ))
}

/// Whether a plan file that did not parse writes a blocking word as the `type` of an entry under `links`. Read
/// from the file as a plain value, since the typed parse has already failed; a file that is not YAML at all, or
/// has no such entry, answers false and gets the parser's own message.
fn names_blocking_as_a_link(text: &str) -> bool {
    let Ok(raw) = serde_yaml::from_str::<serde_json::Value>(text) else {
        return false;
    };
    raw["links"].as_array().is_some_and(|links| {
        links
            .iter()
            .any(|l| l["type"].as_str().is_some_and(is_blocking_word))
    })
}

fn state(created: bool) -> &'static str {
    if created { "created" } else { "existing" }
}

/// One line per name: its key and whether this apply created it; then each blocker edge, each item link and each
/// web link. A web link's title and URL are stored text, so they print through `terminal::line`.
fn render(p: &PlanApplied) -> String {
    let mut out = String::new();
    for i in &p.items {
        let _ = writeln!(
            out,
            "{}\t{}\t{}",
            i.name,
            mark(Level::of_item(i.item), &i.key),
            state(i.created)
        );
    }
    for d in &p.decisions {
        let _ = writeln!(out, "{}\tdecision #{}\t{}", d.name, d.id, state(d.created));
    }
    for b in &p.blocks {
        let _ = writeln!(
            out,
            "{} blocked by {}\t{}",
            mark(Level::Story, &b.story),
            mark(Level::Story, &b.blocked_by),
            state(b.created)
        );
    }
    for l in &p.links {
        let _ = writeln!(
            out,
            "{} {} {}\t{}",
            mark(Level::of_item(l.from_level), &l.from),
            l.link_type.label(ItemLinkDirection::Out),
            mark(Level::of_item(l.to_level), &l.to),
            state(l.created)
        );
    }
    for w in &p.web_links {
        let _ = writeln!(
            out,
            "web link on {}: {}  {}\t{}",
            mark(Level::of_item(w.level), &w.on),
            line(&w.title),
            line(&w.url),
            state(w.created)
        );
    }
    let stories =
        |l: &PlannedLink| l.from_level == ItemType::Story && l.to_level == ItemType::Story;
    if p.links
        .iter()
        .any(|l| l.link_type == ItemLinkType::Follows && stories(l))
    {
        let _ = writeln!(out, "{FOLLOWS_NOTE}");
    }
    out.trim_end().to_string()
}
