//! `reeve apply <file>`: a plan of items, required decisions and blockers, sent as one request.
use super::finish;
use crate::client::Client;
use crate::glyphs::{Level, mark};
use anyhow::Context;
use clap::Args;
use reeve_protocol::*;
use std::fmt::Write;

/// Create a plan of initiatives, epics, stories, required decisions and blockers in one request. Items name each other
/// by local `name`; any other reference is an existing key. The server validates the whole plan before writing, and
/// applying the same plan `id` again creates nothing.
#[derive(Args)]
pub struct Apply {
    /// The plan, YAML or JSON; `-` reads stdin.
    pub file: std::path::PathBuf,
}

pub async fn run(c: &Client, a: Apply, json: bool) -> anyhow::Result<i32> {
    let text = super::text_arg(None, Some(a.file.clone()))?.unwrap_or_default();
    // Parsed into the wire type here, so a misspelt field is an error before anything is sent. YAML reads JSON too.
    let plan: ApplyPlan = serde_yaml::from_str(&text)
        .with_context(|| format!("reading the plan in {}", a.file.display()))?;
    Ok(finish(
        c.post::<PlanApplied>("/plans", serde_json::to_value(plan)?)
            .await?,
        json,
        render,
    ))
}

fn state(created: bool) -> &'static str {
    if created { "created" } else { "existing" }
}

/// One line per name: its key and whether this apply created it; then each blocker edge.
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
    out.trim_end().to_string()
}
