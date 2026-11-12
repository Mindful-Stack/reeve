//! Short text renderers; `--json` bypasses all of these.
use crate::glyphs::{Level, mark};
use crate::promise;
use crate::urgency::{own_mark_text, urgent_lines, urgent_marker};
use chrono::{DateTime, Utc};
use reeve_protocol::*;
use std::fmt::Write;

pub fn state_word(s: DeliveryState) -> &'static str {
    match s {
        DeliveryState::Todo => "todo",
        DeliveryState::InProgress => "in-progress",
        DeliveryState::InReview => "in-review",
        DeliveryState::Testing => "testing",
        DeliveryState::Done => "done",
    }
}

pub fn reason(r: &Reason) -> String {
    match r {
        Reason::Untriaged => "untriaged: no epic yet; triage places it".into(),
        Reason::NoCriteria => "no acceptance criteria".into(),
        Reason::AncestorDiscoveryIncomplete { item } => format!("discovery incomplete on {item}"),
        Reason::DecisionNoRecord { decision_id, title } => {
            format!("decision #{decision_id} \"{title}\" has no record linked")
        }
        Reason::DecisionUnchecked { decision_id, title } => {
            format!("decision #{decision_id} \"{title}\" has not been checked")
        }
        Reason::DecisionProposedPrOpen { decision_id, title } => {
            format!("decision #{decision_id} \"{title}\" is proposed; its PR is linked")
        }
        Reason::DecisionRecordNotFound { decision_id, title } => {
            format!("decision #{decision_id} \"{title}\": record not found on the default branch")
        }
        Reason::DecisionRepoNotConfigured { decision_id, title } => {
            format!("decision #{decision_id} \"{title}\": record in an unchecked repository")
        }
        Reason::DecisionNotAccepted {
            decision_id,
            title,
            status,
            superseded_by,
        } => match superseded_by {
            Some(s) => {
                format!("decision #{decision_id} \"{title}\": superseded by {s}, re-point")
            }
            None => format!("decision #{decision_id} \"{title}\" is {status}"),
        },
        Reason::BlockedBy { story } => format!("blocked by {}", mark(Level::Story, story)),
        Reason::ResolutionRequired => "a resolution is required to move to done".into(),
        Reason::CompletedNeedsActiveOrigin => {
            "completed needs the story to have been in active work".into()
        }
        Reason::OverrideNeedsReason => "an override needs a reason".into(),
        Reason::AutomationCannotOverride => "automation tokens cannot override".into(),
        Reason::SprintActive => "a sprint is already active; close it first".into(),
        Reason::VerdictMissing { goals } => format!(
            "every goal needs a verdict: {}",
            goals
                .iter()
                .map(|g| g.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Reason::SprintNotActive => "the sprint is not active".into(),
        Reason::SprintNotDraft => "only a draft sprint's goals can be removed".into(),
        Reason::SprintClosed => "the sprint is closed".into(),
        Reason::InteractiveOnly => "only an interactive user may do this".into(),
    }
}

pub fn reasons(rs: &[Reason]) -> String {
    rs.iter()
        .map(|r| format!("  - {}", reason(r)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Inserts `lines`, indented by two spaces like the detail lines, right after the first line of `text`, keeping the
/// rest; appends them when `text` is one line.
pub fn with_urgency(text: String, lines: Vec<String>) -> String {
    if lines.is_empty() {
        return text;
    }
    let block = lines
        .iter()
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n");
    match text.split_once('\n') {
        Some((first, rest)) => format!("{first}\n{block}\n{rest}"),
        None => format!("{text}\n{block}"),
    }
}

/// " · <chip>" for a story's promise, or nothing.
fn story_chip(s: &StoryDto) -> String {
    s.promise
        .as_ref()
        .map(|p| format!(" · {}", promise::chip_text(p, true)))
        .unwrap_or_default()
}

/// " · <chip>" for an epic's own promise, or nothing.
fn epic_chip(e: &EpicDto) -> String {
    e.promise
        .as_ref()
        .map(|p| format!(" · {}", promise::epic_chip_text(p)))
        .unwrap_or_default()
}

/// `story` for `story show` and `story context`: with the urgent lines, then the promise line, after the first line.
pub fn story_show(s: &StoryDto, now: DateTime<Utc>) -> String {
    let mut lines = urgent_lines(s.urgent_mark.as_ref(), &s.urgent_sources, &s.key, now);
    lines.extend(s.promise.as_ref().map(promise::line_text));
    with_urgency(story(s), lines)
}

/// A recipient as the answer names it: ` · to <recipient>`, or nothing.
fn to_text(to: Option<&str>) -> String {
    to.filter(|t| !t.is_empty())
        .map(|t| format!(" · to {t}"))
        .unwrap_or_default()
}

/// The answer to promising a story, confirming what was written. One promise: `<key> · <chip>[ · to <recipient>]`,
/// the recipient being the story's own. When its epic's earlier date counts, the chip names the epic and the drawer
/// line follows, indented, giving both promises with the story's own recipient (no `to` on the first line, which
/// would read as the epic's).
pub fn story_promise_answer(s: &StoryDto) -> String {
    let key = mark(Level::of_kind(s.kind), &s.key);
    match &s.promise {
        Some(p) if p.holder == PromiseHolder::Epic => with_urgency(
            format!("{key} · {}", promise::chip_text(p, true)),
            vec![promise::line_text(p)],
        ),
        Some(p) => format!(
            "{key} · {}{}",
            promise::chip_text(p, true),
            to_text(p.own.as_ref().and_then(|f| f.to.as_deref()))
        ),
        None => format!("{key} · no promise"),
    }
}

/// The answer to `story promise --clear`, from the story read before the DELETE and the one it answered: `<key> · no
/// promise to clear` when the story had no promise of its own (its epic's is not the story's to clear); `<key> · own
/// promise cleared; still promised by <day> via <epic>` when its epic's promise still applies; else `<key> · promise
/// cleared`. If the answer still carries an own promise (set again in between), it is shown as promising shows it.
pub fn story_promise_cleared(before: &StoryDto, after: &StoryDto) -> String {
    let key = mark(Level::of_kind(after.kind), &after.key);
    if before
        .promise
        .as_ref()
        .and_then(|p| p.own.as_ref())
        .is_none()
    {
        return format!("{key} · no promise to clear");
    }
    match &after.promise {
        Some(p) if p.own.is_some() => story_promise_answer(after),
        Some(p) => match &p.epic {
            Some(e) => format!(
                "{key} · own promise cleared; still promised by {} via {}",
                promise::day_label(e.fact.on),
                e.key
            ),
            None => format!("{key} · promise cleared"),
        },
        None => format!("{key} · promise cleared"),
    }
}

/// The answer to `epic promise --clear`, from the epic read before the DELETE: `<key> · promise cleared`, or `<key> · no
/// promise to clear` when it had none.
pub fn epic_promise_cleared(before: &EpicDto, after: &EpicDto) -> String {
    let key = mark(Level::Epic, &after.key);
    if before.promise.is_none() {
        format!("{key} · no promise to clear")
    } else if after.promise.is_some() {
        epic_promise_answer(after)
    } else {
        format!("{key} · promise cleared")
    }
}

/// The answer to promising an epic: `<key> · <chip>[ · to <recipient>]`.
pub fn epic_promise_answer(e: &EpicDto) -> String {
    let key = mark(Level::Epic, &e.key);
    match &e.promise {
        Some(p) => {
            format!(
                "{key} · {}{}",
                promise::epic_chip_text(p),
                to_text(p.fact.to.as_deref())
            )
        }
        None => format!("{key} · no promise"),
    }
}

/// `initiative` for `initiative show`: with the urgent lines after the first line.
pub fn initiative_show(i: &InitiativeDto, now: DateTime<Utc>) -> String {
    with_urgency(
        initiative(i),
        urgent_lines(i.urgent_mark.as_ref(), &i.urgent_sources, &i.key, now),
    )
}

/// `epic` for `epic show`: with the urgent lines, then the promise line, after the first line.
pub fn epic_show(e: &EpicDto, now: DateTime<Utc>) -> String {
    let mut lines = urgent_lines(e.urgent_mark.as_ref(), &e.urgent_sources, &e.key, now);
    lines.extend(e.promise.as_ref().map(promise::epic_line_text));
    with_urgency(epic(e), lines)
}

/// The answer to marking an item urgent: its mark line (`!` after the key once the server counts it urgent), then
/// the urgent lines. The server's `sources` decide urgency; nothing here reads the mark's timestamps.
pub fn urgent_answer(
    level: Level,
    key: &Key,
    own: Option<&UrgentMarkDto>,
    sources: &[ItemRefDto],
    now: DateTime<Utc>,
) -> String {
    let first = format!("{}{}", mark(level, key), urgent_marker(sources));
    with_urgency(first, urgent_lines(own, sources, key, now))
}

/// The war room's rows, one per line, or its empty text.
/// `<glyph> <KEY> <title> · in <initiative title> · <own mark text>[ · done]`. An initiative row's `initiative` is the
/// row itself, so only epic and story rows name one.
pub fn war_room_rows<'a>(
    rows: impl IntoIterator<Item = &'a WarRoomEntryDto>,
    now: DateTime<Utc>,
) -> String {
    let lines: Vec<String> = rows
        .into_iter()
        .map(|r| {
            let mut parts = vec![format!(
                "{} {}",
                mark(Level::of_item(r.item.item), &r.item.key),
                r.title
            )];
            if r.item.item != ItemType::Initiative
                && let Some(i) = &r.initiative
            {
                parts.push(format!("in {}", i.title));
            }
            parts.push(own_mark_text(&r.mark, now));
            if r.mark.done {
                parts.push("done".into());
            }
            parts.join(" · ")
        })
        .collect();
    if lines.is_empty() {
        "The war room is empty.".into()
    } else {
        lines.join("\n")
    }
}

pub fn story(s: &StoryDto) -> String {
    let mut out = format!(
        "{}{} [{}] {}{}\n  kind: {:?}  version: {}",
        mark(Level::of_kind(s.kind), &s.key),
        urgent_marker(&s.urgent_sources),
        state_word(s.delivery),
        s.title,
        story_chip(s),
        s.kind,
        s.version
    );
    if let Some(e) = &s.epic {
        let _ = write!(out, "  epic: {}", mark(Level::Epic, e));
    }
    if let Some(o) = &s.owner {
        let _ = write!(out, "  owner: {o}");
    }
    if let Some(r) = &s.resolution {
        let _ = write!(out, "  resolution: {r:?}");
    }
    if !s.labels.is_empty() {
        let _ = write!(out, "  labels: {}", s.labels.join(","));
    }
    if let Some(r) = &s.external_ref {
        let _ = write!(out, "  from: {r}");
    }
    for c in &s.criteria {
        let _ = write!(out, "\n  [{}] {}", if c.done { "x" } else { " " }, c.text);
    }
    if !s.prs.is_empty() {
        let _ = write!(out, "\n  Pull requests:");
        for p in &s.prs {
            let _ = write!(
                out,
                "\n    {}#{} [{}] {} {}",
                p.repo,
                p.number,
                pr_status_word(p.status),
                p.title,
                p.url
            );
        }
    }
    out
}

fn pr_status_word(s: PrStatus) -> &'static str {
    match s {
        PrStatus::Draft => "draft",
        PrStatus::Open => "open",
        PrStatus::Merged => "merged",
        PrStatus::Closed => "closed",
    }
}

pub fn story_row(s: &StoryDto, r: &Readiness) -> String {
    let ready_word = if r.ready {
        "ready".to_string()
    } else {
        r.reasons
            .first()
            .map(reason)
            .unwrap_or_else(|| "not ready".into())
    };
    format!(
        "{}{}\t{}\t{}\t{}{}",
        mark(Level::of_kind(s.kind), &s.key),
        urgent_marker(&s.urgent_sources),
        state_word(s.delivery),
        ready_word,
        s.title,
        story_chip(s)
    )
}

pub fn readiness(r: &Readiness) -> String {
    if r.ready {
        "ready".into()
    } else {
        reasons(&r.reasons)
    }
}

fn check_text(c: &CheckResult) -> String {
    match c {
        CheckResult::Unchecked => "unchecked".into(),
        CheckResult::Accepted { commit, .. } => {
            format!("accepted @ {}", commit.chars().take(8).collect::<String>())
        }
        CheckResult::NotAccepted {
            status,
            superseded_by,
            ..
        } => match superseded_by {
            Some(s) => format!("superseded by {s}, re-point"),
            None => status.clone(),
        },
        CheckResult::ProposedPrOpen => "proposed (PR linked)".into(),
        CheckResult::NotFound => "record not found".into(),
    }
}

pub fn decision(d: &RequiredDecisionDto) -> String {
    let mut out = format!(
        "#{} on {} \"{}\" decider: {} check: {} version: {}",
        d.id,
        mark(Level::of_item(d.on_type), &d.on),
        d.title,
        d.decider,
        check_text(&d.check),
        d.version
    );
    if let Some(due) = d.due {
        let _ = write!(out, " due: {due}");
    }
    if let Some(r) = &d.record {
        let _ = write!(out, " record: {}:{} ({})", r.repo, r.path, r.record_id);
    }
    if let Some(pr) = &d.pr_url {
        let _ = write!(out, " pr: {pr}");
    }
    if let Some(r) = d.reversibility {
        let _ = write!(
            out,
            " reversibility: {}",
            match r {
                Reversibility::OneWay => "one-way",
                Reversibility::TwoWay => "two-way",
            }
        );
    }
    if let Some(c) = &d.closed {
        let _ = match c.kind {
            Some(ClosureKind::Decided) => write!(out, " closed: decided: {}", c.reason),
            Some(ClosureKind::Withdrawn) => write!(out, " closed: withdrawn: {}", c.reason),
            None => write!(out, " closed: {}", c.reason),
        };
    }
    let _ = write!(
        out,
        " gates {}",
        match d.on_type {
            ItemType::Initiative => "every story under this initiative",
            ItemType::Epic => "the stories in this epic",
            ItemType::Story => "this story",
        }
    );
    out
}

/// The decision queue: the summary line, then one line per entry in the server's order.
pub fn queue(q: &DecisionQueueDto) -> String {
    let s = &q.summary;
    let mut out = format!(
        "open: {} · blocking: {} ({} stories) · overdue: {}",
        s.open, s.blocking, s.blocked_stories, s.overdue
    );
    for e in &q.entries {
        let _ = write!(
            out,
            "\n[{}] {} blocks: {}",
            match e.group {
                QueueGroup::Blocking => "blocking",
                QueueGroup::Open => "open",
                QueueGroup::Satisfied => "satisfied",
                QueueGroup::Closed => "closed",
            },
            decision(&e.decision),
            e.blocks.len()
        );
    }
    out
}

pub fn initiative(i: &InitiativeDto) -> String {
    let mut out = format!(
        "{}{} {} (horizon: {}, version: {})",
        mark(Level::Initiative, &i.key),
        urgent_marker(&i.urgent_sources),
        i.title,
        i.horizon,
        i.version
    );
    if let Some(m) = &i.target_month {
        let _ = write!(out, " target: {m}");
    }
    if let Some(o) = &i.owner {
        let _ = write!(out, " owner: {o}");
    }
    if let Some(r) = &i.external_ref {
        let _ = write!(out, " from: {r}");
    }
    if let Some(p) = i.progress {
        let _ = write!(out, " progress: {:.0}%", p * 100.0);
    }
    let _ = write!(
        out,
        " discovery: {}",
        if i.discovery_complete.is_some() {
            "complete"
        } else {
            "open"
        }
    );
    out
}

pub fn epic(e: &EpicDto) -> String {
    let mut out = format!(
        "{}{} {}{} (version: {})",
        mark(Level::Epic, &e.key),
        urgent_marker(&e.urgent_sources),
        e.title,
        epic_chip(e),
        e.version
    );
    if let Some(i) = &e.initiative {
        let _ = write!(out, " initiative: {}", mark(Level::Initiative, i));
    }
    if let Some(r) = &e.external_ref {
        let _ = write!(out, " from: {r}");
    }
    let _ = write!(
        out,
        " discovery: {}",
        if e.discovery_complete.is_some() {
            "complete"
        } else {
            "open"
        }
    );
    out
}

/// Advisory findings under an initiative or an epic; empty when there are none.
pub fn findings(fs: &[Finding]) -> String {
    if fs.is_empty() {
        return String::new();
    }
    let mut out = "findings (advisory, never block a move):".to_string();
    for f in fs {
        let line = match f {
            Finding::EmptyBrief => "the brief is empty".to_string(),
            Finding::BriefCitesWithoutDecision { cites } => format!(
                "the brief cites {} but no required decision is on this item or above it",
                cites.join(", ")
            ),
            Finding::DiscoveryCompleteWithOpenDecision { decision_id, title } => {
                format!("discovery is complete while decision #{decision_id} \"{title}\" is open")
            }
            Finding::DiscoveryCompleteWithoutStories => {
                "discovery is complete and there are no stories under it".to_string()
            }
        };
        let _ = write!(out, "\n  - {line}");
    }
    out
}

/// The decisions under an initiative, in its `show`: each where it lives; empty when there are none.
pub fn decisions_block(ds: &[RequiredDecisionDto]) -> String {
    if ds.is_empty() {
        return String::new();
    }
    let mut out = "decisions (on it, its epics and their stories):".to_string();
    for d in ds {
        let _ = write!(out, "\n  - {}", decision(d));
    }
    out
}

/// An initiative's or an epic's comments under its `show`; empty when there are none.
pub fn comments(cs: &[CommentDto]) -> String {
    if cs.is_empty() {
        return String::new();
    }
    let mut out = "comments:".to_string();
    for c in cs {
        let _ = write!(out, "\n  - {}", comment(c));
    }
    out
}

pub fn comment(c: &CommentDto) -> String {
    format!("{} {}: {}", c.at.format("%Y-%m-%d %H:%M"), c.author, c.body)
}

/// The context bundle as Markdown at a given instant. Every required decision's record path appears verbatim so an agent
/// can open the file; the story's urgent lines follow its title line.
pub fn context_markdown(c: &StoryContextDto, now: DateTime<Utc>) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "# Story\n\n{}\n", story_show(&c.story, now));
    if !c.story.description.is_empty() {
        let _ = writeln!(o, "{}\n", c.story.description);
    }
    let _ = writeln!(o, "## Readiness\n\n{}\n", readiness(&c.readiness));
    if let Some(i) = &c.initiative {
        let _ = writeln!(
            o,
            "## Initiative brief\n\n{}\n\n{}\n",
            initiative(i),
            i.brief
        );
    }
    if let Some(e) = &c.epic {
        let _ = writeln!(o, "## Epic brief\n\n{}\n\n{}\n", epic(e), e.brief);
    }
    let _ = writeln!(o, "## Required decisions\n");
    if c.required_decisions.is_empty() {
        let _ = writeln!(o, "none\n");
    }
    for d in &c.required_decisions {
        let _ = writeln!(
            o,
            "- {} (decider: {}) check: {}",
            d.title,
            d.decider,
            check_text(&d.check)
        );
        if let Some(r) = &d.record {
            let _ = writeln!(
                o,
                "  record: {} (repo {}, id {})",
                r.path, r.repo, r.record_id
            );
        }
        if let Some(pr) = &d.pr_url {
            let _ = writeln!(o, "  pr: {pr}");
        }
    }
    let _ = writeln!(o, "\n## Blockers\n");
    if c.blockers.is_empty() {
        let _ = writeln!(o, "none");
    }
    for b in &c.blockers {
        let _ = writeln!(
            o,
            "- {} [{}] {}",
            mark(Level::of_kind(b.kind), &b.key),
            state_word(b.delivery),
            b.title
        );
    }
    let _ = writeln!(o, "\n## Criteria\n");
    if c.story.criteria.is_empty() {
        let _ = writeln!(o, "none");
    }
    for k in &c.story.criteria {
        let _ = writeln!(o, "- [{}] {}", if k.done { "x" } else { " " }, k.text);
    }
    let _ = writeln!(o, "\n## Comments\n");
    if c.comments.is_empty() {
        let _ = writeln!(o, "none");
    }
    for k in &c.comments {
        let _ = writeln!(o, "- {}", comment(k));
    }
    o
}

/// The `reeve import jira` report: what the plan holds and why anything is left out, then (after `--apply`) what was
/// created, what already existed, what failed and what was skipped because its parent failed.
pub fn import_report(r: &crate::commands::import::Report) -> String {
    use crate::commands::import::SKIPPED_COMMENTS;
    /// Past this many lines a group prints its count and the first lines only.
    const LISTED: usize = 20;
    let p = &r.planned;
    let mut out = format!(
        "Planned: {} initiatives, {} epics, {} stories, {} comments ({} warnings)",
        p.initiatives, p.epics, p.stories, p.comments, p.warnings
    );
    let tally = |m: &std::collections::BTreeMap<String, usize>| {
        m.iter()
            .map(|(k, n)| format!("{k} {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if !p.by_kind.is_empty() {
        let _ = write!(out, "\n  by kind: {}", tally(&p.by_kind));
        let _ = write!(out, "\n  by delivery: {}", tally(&p.by_delivery));
    }
    if r.warnings.is_empty() {
        out.push_str("\n\nWarnings: none");
    } else {
        out.push_str("\n\nWarnings:");
        for g in &r.warnings {
            let _ = write!(out, "\n  {} ({}):", g.group, g.count);
            let shown = if g.group == SKIPPED_COMMENTS {
                LISTED.min(g.lines.len())
            } else {
                g.lines.len()
            };
            for l in &g.lines[..shown] {
                let _ = write!(out, "\n    {l}");
            }
            if shown < g.lines.len() {
                let _ = write!(out, "\n    ... and {} more", g.lines.len() - shown);
            }
        }
    }
    out.push_str("\n\nLink types seen:");
    if r.link_types_seen.is_empty() {
        out.push_str(" none");
    }
    for (t, n) in &r.link_types_seen {
        let _ = write!(out, "\n  {t}: {n}");
    }
    out.push_str("\n\nGrandfathered (created in their current state, labelled legacy-import):");
    for (what, keys) in [
        ("stories", &r.grandfathered_stories),
        ("epics marked discovery complete", &r.marked_epics),
        (
            "initiatives marked discovery complete",
            &r.marked_initiatives,
        ),
    ] {
        let list = if keys.is_empty() {
            "none".to_string()
        } else {
            keys.join(", ")
        };
        let _ = write!(out, "\n  {what}: {list}");
    }
    match &r.apply {
        None => out.push_str("\n\nRehearsal: nothing written. Run again with --apply to import."),
        Some(a) => {
            out.push_str("\n\nApplied:");
            for (what, n) in [("created", a.created), ("existing", a.existing)] {
                let _ = write!(
                    out,
                    "\n  {what}: {} initiatives, {} epics, {} stories, {} comments",
                    n.initiatives, n.epics, n.stories, n.comments
                );
            }
            let _ = write!(out, "\n  failed: {}", a.failed.len());
            for f in &a.failed {
                let code = f.code_word();
                let _ = write!(
                    out,
                    "\n    {} ({}): {code}: {}",
                    f.jira_key, f.step, f.message
                );
            }
            let _ = write!(out, "\n  skipped: {}", a.skipped.len());
            for (k, why) in &a.skipped {
                let _ = write!(out, "\n    {k}: {why}");
            }
            if let Some(why) = &a.stopped {
                let _ = write!(out, "\n  stopped after {why}: nothing more was sent");
            }
        }
    }
    out
}

/// The membership rule (kb/ADR-0014 R2), stated once wherever a sprint's counts are shown.
pub const MEMBERSHIP_RULE: &str =
    "Counts the work completed in this sprint; work started but not finished shows as in flight.";

/// The promise rule (kb/ADR-0025 R8), stated under a sprint's promise list, as the web's sprint history states it.
pub const PROMISE_RULE: &str =
    "A promise is to a date: work done after it counts as late, even inside the sprint.";

/// Whether the counts at close differ from the derived ones, number by number: totals, and each goal's numbers keyed by
/// `goal_id` (the vector order and the serialised text never matter). A goal on one side only is a difference.
/// `blocked_decisions` is a live measure, not part of the window's history, so it never counts as a difference.
pub fn counts_differ(now: &SprintCountsDto, at_close: &SprintCountsDto) -> bool {
    use std::collections::BTreeMap;
    let by_goal = |c: &SprintCountsDto| {
        c.goals
            .iter()
            .map(|g| (g.goal_id, (g.completed, g.in_flight, g.todo, g.in_review)))
            .collect::<BTreeMap<_, _>>()
    };
    now.completed != at_close.completed
        || now.upkeep_completed != at_close.upkeep_completed
        || by_goal(now) != by_goal(at_close)
}

/// A number now, followed by its value at close when that differs.
fn now_then(now: u32, then: Option<u32>) -> String {
    match then {
        Some(t) if t != now => format!("{now} (at close: {t})"),
        _ => now.to_string(),
    }
}

/// "blocked by 1 decision", "blocked by 3 decisions".
fn blocked_by(n: u32) -> String {
    format!("blocked by {n} decision{}", if n == 1 { "" } else { "s" })
}

fn sprint_state_word(s: SprintState) -> &'static str {
    match s {
        SprintState::Draft => "draft",
        SprintState::Active => "active",
        SprintState::Closed => "closed",
    }
}

fn confidence_word(c: Confidence) -> &'static str {
    match c {
        Confidence::OnTrack => "on track",
        Confidence::AtRisk => "at risk",
        Confidence::OffTrack => "off track",
    }
}

fn verdict_word(v: Verdict) -> &'static str {
    match v {
        Verdict::Met => "met",
        Verdict::PartlyMet => "partly met",
        Verdict::Missed => "missed",
    }
}

fn warning_text(w: SprintWarning) -> &'static str {
    match w {
        SprintWarning::NoGoals => "the sprint has no goals",
        SprintWarning::MoreThanThreeGoals => "more than three goals thins focus",
    }
}

/// Advice from the goal count; empty when there is none.
pub fn warnings(ws: &[SprintWarning]) -> String {
    ws.iter()
        .map(|w| format!("warning: {}", warning_text(*w)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn with_note(word: &str, note: &Option<String>) -> String {
    match note.as_deref() {
        Some(n) if !n.is_empty() => format!("{word} ({n})"),
        _ => word.to_string(),
    }
}

/// One goal: its outcome, check, owner, notes, confidence (and a pending suggestion), verdict and links.
pub fn goal(g: &GoalDto) -> String {
    let mut out = format!(
        "goal #{} {} (sprint #{}, version: {})\n  we'll know when: {}\n  owner: {}",
        g.id, g.outcome, g.sprint_id, g.version, g.check, g.owner
    );
    if !g.notes.is_empty() {
        let _ = write!(out, "\n  notes: {}", g.notes);
    }
    if let Some(c) = g.confidence {
        let _ = write!(
            out,
            "\n  confidence: {}",
            with_note(confidence_word(c), &g.confidence_note)
        );
    }
    if let Some(c) = g.suggested_confidence {
        let _ = write!(
            out,
            "\n  suggested: {}{} (pending; a person records or discards it)",
            with_note(confidence_word(c), &g.suggested_note),
            g.suggested_by
                .as_deref()
                .map(|b| format!(" by {b}"))
                .unwrap_or_default()
        );
    }
    if let Some(v) = g.verdict {
        let _ = write!(
            out,
            "\n  verdict: {}",
            with_note(verdict_word(v), &g.verdict_note)
        );
    }
    if !g.links.is_empty() {
        let _ = write!(
            out,
            "\n  links: {}",
            g.links
                .iter()
                .map(|k| k.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    out
}

/// A sprint with its goals and warnings (no counts): the answer to open and close.
pub fn sprint(s: &SprintDto) -> String {
    let mut out = format!(
        "sprint #{} [{}] {} – {} (version: {})",
        s.id,
        sprint_state_word(s.state),
        s.starts_on,
        s.ends_on,
        s.version
    );
    for g in &s.goals {
        let _ = write!(out, "\n{}", indent(&goal(g)));
    }
    if !s.warnings.is_empty() {
        let _ = write!(out, "\n{}", warnings(&s.warnings));
    }
    out
}

fn indent(s: &str) -> String {
    s.lines()
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `reeve sprint show`: the sprint, the membership rule above its counts, each goal with its numbers, the upkeep share,
/// the demo list, and the promises due in the sprint (kb/ADR-0025 R8) with their range and rule, all as the server sent
/// them. `closed_entry` is a closed sprint's `closed` entry counts: where they differ from the derived ones
/// (`counts_differ`), each differing number shows as `<now> (at close: <then>)`. "Blocked by N decisions" is live for
/// the active sprint; a closed sprint shows only its value at close, from the entry, never the recomputed one. A draft
/// has no window, so it shows no counts.
pub fn sprint_view(v: &SprintViewDto, closed_entry: Option<&SprintCountsDto>) -> String {
    let s = &v.sprint;
    let closed = s.state == SprintState::Closed;
    let entry = closed_entry.filter(|_| closed);
    let at_close = entry.filter(|e| counts_differ(&v.counts, e));
    let mut out = format!(
        "sprint #{} [{}] {} – {} (version: {})",
        s.id,
        sprint_state_word(s.state),
        s.starts_on,
        s.ends_on,
        s.version
    );
    let counted = s.state != SprintState::Draft;
    if counted {
        let _ = write!(out, "\n{MEMBERSHIP_RULE}");
        if at_close.is_some() {
            out.push_str("\nchanged since close: items moved between parents or reopened");
        }
    }
    let numbers = |p: &GoalProgressDto| {
        format!(
            "completed: {}  in flight: {}  in review: {}  todo: {}",
            p.completed, p.in_flight, p.in_review, p.todo
        )
    };
    for g in &s.goals {
        let _ = write!(out, "\n{}", indent(&goal(g)));
        if !counted {
            continue;
        }
        let now = v.counts.goals.iter().find(|p| p.goal_id == g.id);
        let then = at_close.and_then(|a| a.goals.iter().find(|p| p.goal_id == g.id));
        match now {
            Some(p) => {
                let t = |f: fn(&GoalProgressDto) -> u32| then.map(f);
                let blocked = if closed {
                    entry
                        .and_then(|a| a.goals.iter().find(|x| x.goal_id == g.id))
                        .map(|x| format!("  {} at close", blocked_by(x.blocked_decisions)))
                        .unwrap_or_default()
                } else {
                    format!("  {}", blocked_by(p.blocked_decisions))
                };
                let missing = if at_close.is_some() && then.is_none() {
                    "  (not at close)"
                } else {
                    ""
                };
                let _ = write!(
                    out,
                    "\n    completed: {}  in flight: {}  in review: {}  todo: {}{blocked}{missing}",
                    now_then(p.completed, t(|x| x.completed)),
                    now_then(p.in_flight, t(|x| x.in_flight)),
                    now_then(p.in_review, t(|x| x.in_review)),
                    now_then(p.todo, t(|x| x.todo)),
                );
            }
            None => {
                if let Some(x) = then {
                    let _ = write!(out, "\n    (removed since close) at close: {}", numbers(x));
                }
            }
        }
    }
    // A goal the closed entry counted that the sprint no longer has.
    if let Some(a) = at_close {
        for x in a
            .goals
            .iter()
            .filter(|x| !s.goals.iter().any(|g| g.id == x.goal_id))
        {
            let _ = write!(
                out,
                "\n  goal #{} (removed since close) at close: {}",
                x.goal_id,
                numbers(x)
            );
        }
    }
    if counted {
        let c = &v.counts;
        let share = if c.completed > 0 {
            format!(
                "{}%; ",
                (c.upkeep_completed as f64 * 100.0 / c.completed as f64).round()
            )
        } else {
            String::new()
        };
        let _ = write!(
            out,
            "\n{} of {} completed stories were upkeep ({share}team line {}%)",
            now_then(c.upkeep_completed, at_close.map(|a| a.upkeep_completed)),
            now_then(c.completed, at_close.map(|a| a.completed)),
            v.upkeep_line
        );
        if !v.demo.is_empty() {
            out.push_str("\ndone in the last 7 days:");
            for d in &v.demo {
                let _ = write!(out, "\n  {} {}", mark(Level::Story, &d.key), d.title);
            }
        }
        if v.promises.is_empty() {
            out.push_str("\nNo promises due in this sprint.");
        } else {
            let _ = write!(
                out,
                "\n{}:",
                promise::promises_heading(v.promises_after, s.ends_on)
            );
            for p in &v.promises {
                let _ = write!(
                    out,
                    "\n  {} {} · {}",
                    mark(Level::Story, &p.key),
                    p.title,
                    promise::sprint_row_text(p)
                );
            }
            let _ = write!(out, "\n{PROMISE_RULE}");
        }
    }
    if !s.warnings.is_empty() {
        let _ = write!(out, "\n{}", warnings(&s.warnings));
    }
    out
}

/// `reeve sprint ls`: the active sprint, the draft and the closed sprints (newest first), each with its goals, their
/// confidence and verdicts. No counts: `reeve sprint show <id>` gives those.
pub fn sprint_list(x: &SprintsDto) -> String {
    let line = |s: &SprintDto| {
        let mut out = format!(
            "#{} [{}] {} – {}",
            s.id,
            sprint_state_word(s.state),
            s.starts_on,
            s.ends_on
        );
        for g in &s.goals {
            let _ = write!(out, "\n  goal #{} {}", g.id, g.outcome);
            if let Some(c) = g.confidence {
                let _ = write!(out, " [{}]", confidence_word(c));
            }
            if let Some(v) = g.verdict {
                let _ = write!(
                    out,
                    " verdict: {}",
                    with_note(verdict_word(v), &g.verdict_note)
                );
            }
        }
        out
    };
    let mut parts = vec![];
    if let Some(a) = &x.active {
        parts.push(line(&a.sprint));
    }
    if let Some(d) = &x.draft {
        parts.push(line(d));
    }
    parts.extend(x.closed.iter().map(line));
    if parts.is_empty() {
        return "no sprints yet: `reeve goal add` starts the first draft".into();
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn dto(prs: Vec<PullRequestDto>) -> StoryDto {
        StoryDto {
            key: "ABC-1".parse().unwrap(),
            title: "Match".into(),
            kind: Kind::Story,
            epic: None,
            description: String::new(),
            criteria: vec![],
            owner: None,
            labels: vec![],
            rank: 1,
            blocked_by: vec![],
            delivery: DeliveryState::Todo,
            resolution: None,
            external_ref: None,
            version: 1,
            prs,
            goals: vec![],
            urgent_mark: None,
            urgent_sources: vec![],
            promise: None,
        }
    }

    #[test]
    fn story_lists_its_pull_requests_and_omits_the_block_when_none() {
        assert!(!story(&dto(vec![])).contains("Pull requests"));
        let out = story(&dto(vec![PullRequestDto {
            repo: "acme/portal".into(),
            number: 42,
            url: "https://github.com/acme/portal/pull/42".into(),
            title: "fix".into(),
            status: PrStatus::Merged,
            updated_at: chrono::Utc.with_ymd_and_hms(2026, 10, 26, 9, 0, 0).unwrap(),
        }]));
        assert!(out.contains("\n  Pull requests:\n    acme/portal#42 [merged] fix https://github.com/acme/portal/pull/42"), "{out}");
    }

    /// Every Reeve key in text output carries its level's glyph (the default, non-ASCII mode).
    #[test]
    fn keys_carry_their_level_glyph() {
        let bug = StoryDto {
            kind: Kind::Bug,
            epic: Some("ABC-2".parse().unwrap()),
            ..dto(vec![])
        };
        let out = story(&bug);
        assert!(out.starts_with("▲ ABC-1 [todo]"), "{out}");
        assert!(out.contains("epic: ■ ABC-2"), "{out}");
        let ready = Readiness {
            ready: true,
            reasons: vec![],
        };
        assert!(story_row(&dto(vec![]), &ready).starts_with("● ABC-1\t"));
        let e = EpicDto {
            key: "ABC-2".parse().unwrap(),
            title: "Hosting".into(),
            initiative: Some("ABC-3".parse().unwrap()),
            brief: String::new(),
            discovery_complete: None,
            external_ref: None,
            version: 1,
            urgent_mark: None,
            urgent_sources: vec![],
            promise: None,
        };
        let out = epic(&e);
        assert!(
            out.starts_with("■ ABC-2 Hosting") && out.contains("initiative: ◆ ABC-3"),
            "{out}"
        );
        assert_eq!(
            reason(&Reason::BlockedBy {
                story: "ABC-4".parse().unwrap()
            }),
            "blocked by ● ABC-4"
        );
        assert_eq!(
            reason(&Reason::Untriaged),
            "untriaged: no epic yet; triage places it"
        );
    }

    #[test]
    fn findings_render_one_line_each_and_nothing_when_none() {
        assert_eq!(findings(&[]), "");
        let out = findings(&[
            Finding::EmptyBrief,
            Finding::BriefCitesWithoutDecision {
                cites: vec!["Jira".into(), "decided".into()],
            },
            Finding::DiscoveryCompleteWithOpenDecision {
                decision_id: 7,
                title: "Pick a host".into(),
            },
            Finding::DiscoveryCompleteWithoutStories,
        ]);
        assert_eq!(
            out,
            "findings (advisory, never block a move):\n  - the brief is empty\n  - the brief cites Jira, decided but no required decision is on this item or above it\n  - discovery is complete while decision #7 \"Pick a host\" is open\n  - discovery is complete and there are no stories under it"
        );
    }

    #[test]
    fn each_item_shows_its_external_reference_only_when_it_has_one() {
        assert!(!story(&dto(vec![])).contains("from:"));
        let s = StoryDto {
            external_ref: Some("SCRUM-481".into()),
            ..dto(vec![])
        };
        assert!(story(&s).contains("  from: SCRUM-481"), "{}", story(&s));
        let e = EpicDto {
            key: "ABC-2".parse().unwrap(),
            title: "Hosting".into(),
            initiative: None,
            brief: String::new(),
            discovery_complete: None,
            external_ref: Some("SCRUM-480".into()),
            version: 1,
            urgent_mark: None,
            urgent_sources: vec![],
            promise: None,
        };
        assert!(epic(&e).contains(" from: SCRUM-480"), "{}", epic(&e));
        let i = InitiativeDto {
            key: "ABC-1".parse().unwrap(),
            title: "Leave the old host".into(),
            brief: String::new(),
            horizon: "Now".into(),
            target_month: None,
            owner: None,
            discovery_complete: None,
            progress: None,
            external_ref: Some("JPD-30".into()),
            version: 1,
            urgent_mark: None,
            urgent_sources: vec![],
        };
        assert!(
            initiative(&i).contains(" from: JPD-30"),
            "{}",
            initiative(&i)
        );
    }

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc
            .with_ymd_and_hms(2026, 11, 10, 12, 0, 0)
            .unwrap()
    }

    fn ref_(key: &str, item: ItemType) -> ItemRefDto {
        ItemRefDto {
            key: key.parse().unwrap(),
            item,
        }
    }

    fn own_mark(active: bool, done: bool) -> UrgentMarkDto {
        UrgentMarkDto {
            at: chrono::Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap(),
            by: "dana@example.com".into(),
            reason: None,
            expires_at: chrono::Utc
                .with_ymd_and_hms(2026, 11, 20, 12, 0, 0)
                .unwrap(),
            active,
            done,
        }
    }

    fn ready() -> Readiness {
        Readiness {
            ready: true,
            reasons: vec![],
        }
    }

    fn epic_dto() -> EpicDto {
        EpicDto {
            key: "ABC-2".parse().unwrap(),
            title: "Hosting".into(),
            initiative: None,
            brief: String::new(),
            discovery_complete: None,
            external_ref: None,
            version: 1,
            urgent_mark: None,
            urgent_sources: vec![],
            promise: None,
        }
    }

    fn initiative_dto() -> InitiativeDto {
        InitiativeDto {
            key: "ABC-3".parse().unwrap(),
            title: "Leave".into(),
            brief: String::new(),
            horizon: "Now".into(),
            target_month: None,
            owner: None,
            discovery_complete: None,
            progress: None,
            external_ref: None,
            version: 1,
            urgent_mark: None,
            urgent_sources: vec![],
        }
    }

    #[test]
    fn an_urgent_item_carries_a_bang_after_its_key_in_every_one_line_form() {
        let key: Key = "ABC-1".parse().unwrap();
        let s = StoryDto {
            urgent_sources: vec![ref_("ABC-1", ItemType::Story)],
            ..dto(vec![])
        };
        assert_eq!(
            story_row(&s, &ready()),
            format!("{} !\ttodo\tready\tMatch", mark(Level::Story, &key))
        );
        assert!(
            story(&s).starts_with(&format!("{} !", mark(Level::Story, &key))),
            "{}",
            story(&s)
        );
        // Not urgent: unchanged.
        assert_eq!(
            story_row(&dto(vec![]), &ready()),
            format!("{}\ttodo\tready\tMatch", mark(Level::Story, &key))
        );
        let i = InitiativeDto {
            urgent_sources: vec![ref_("ABC-40", ItemType::Epic)],
            ..initiative_dto()
        };
        let ik: Key = "ABC-3".parse().unwrap();
        assert!(
            initiative(&i).starts_with(&format!("{} ! ", mark(Level::Initiative, &ik))),
            "{}",
            initiative(&i)
        );
        let e = EpicDto {
            urgent_sources: vec![ref_("ABC-1", ItemType::Story)],
            ..epic_dto()
        };
        let ek: Key = "ABC-2".parse().unwrap();
        assert!(
            epic(&e).starts_with(&format!("{} ! ", mark(Level::Epic, &ek))),
            "{}",
            epic(&e)
        );
    }

    #[test]
    fn a_done_storys_active_mark_gets_no_bang() {
        let s = StoryDto {
            delivery: DeliveryState::Done,
            urgent_mark: Some(own_mark(true, true)),
            urgent_sources: vec![],
            ..dto(vec![])
        };
        let key: Key = "ABC-1".parse().unwrap();
        assert_eq!(
            story_row(&s, &ready()),
            format!("{}\tdone\tready\tMatch", mark(Level::Story, &key))
        );
        assert!(
            story(&s).starts_with(&format!("{} [done]", mark(Level::Story, &key))),
            "{}",
            story(&s)
        );
    }

    #[test]
    fn story_show_puts_the_urgent_lines_right_after_the_first_line() {
        let s = StoryDto {
            criteria: vec![CriterionDto {
                text: "It works".into(),
                done: false,
            }],
            urgent_mark: Some(own_mark(true, false)),
            urgent_sources: vec![ref_("ABC-1", ItemType::Story)],
            ..dto(vec![])
        };
        let plain = story(&s);
        let mut want: Vec<&str> = plain.lines().collect();
        want.insert(1, "  urgent · marked by dana, 2 d ago · expires in 10 d");
        assert_eq!(story_show(&s, now()), want.join("\n"));
        assert!(plain.contains("\n  [ ] It works"), "{plain}");
    }

    #[test]
    fn urgent_answer_is_the_mark_line_then_the_urgent_lines() {
        let key: Key = "ABC-1".parse().unwrap();
        let m = own_mark(true, false);
        let out = urgent_answer(
            Level::Story,
            &key,
            Some(&m),
            &[ref_("ABC-1", ItemType::Story)],
            now(),
        );
        assert_eq!(
            out,
            format!(
                "{} !\n  urgent · marked by dana, 2 d ago · expires in 10 d",
                mark(Level::Story, &key)
            )
        );
        // Nothing urgent by the server's count: no bang, and the expired own mark gets its line.
        let m = own_mark(false, false);
        let out = urgent_answer(Level::Epic, &key, Some(&m), &[], now());
        assert!(out.starts_with(&mark(Level::Epic, &key)), "{out}");
        assert!(!out.lines().next().unwrap().ends_with('!'), "{out}");
        // A done story's active mark: urgency comes from the sources (none), never from the mark.
        let m = own_mark(true, true);
        let out = urgent_answer(Level::Story, &key, Some(&m), &[], now());
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], mark(Level::Story, &key), "{out}");
        assert_eq!(lines[1], "  urgent mark · done · expires in 10 d", "{out}");
        assert_eq!(lines.len(), 2, "{out}");
    }

    fn war_room_row(
        key: &str,
        item: ItemType,
        initiative: Option<&str>,
        done: bool,
    ) -> WarRoomEntryDto {
        WarRoomEntryDto {
            item: ref_(key, item),
            title: format!("t-{key}"),
            initiative: initiative.map(|t| WarRoomInitiativeDto {
                key: "ABC-2".parse().unwrap(),
                title: t.into(),
                horizon: "h".into(),
            }),
            mark: own_mark(true, done),
        }
    }

    #[test]
    fn war_room_rows_name_the_initiative_once_and_never_for_an_initiative_row() {
        let rows = [
            war_room_row("ABC-2", ItemType::Initiative, Some("Payments"), false),
            war_room_row("ABC-40", ItemType::Epic, Some("Payments"), false),
            war_room_row("ABC-9", ItemType::Story, None, true),
        ];
        let out = war_room_rows(&rows, now());
        let lines: Vec<&str> = out.lines().collect();
        let tail = "marked by dana, 2 d ago · expires in 10 d";
        assert_eq!(
            lines[0],
            format!(
                "{} t-ABC-2 · {tail}",
                mark(Level::Initiative, &rows[0].item.key)
            )
        );
        assert_eq!(
            lines[1],
            format!(
                "{} t-ABC-40 · in Payments · {tail}",
                mark(Level::Epic, &rows[1].item.key)
            )
        );
        assert_eq!(
            lines[2],
            format!(
                "{} t-ABC-9 · {tail} · done",
                mark(Level::Story, &rows[2].item.key)
            )
        );
        assert_eq!(war_room_rows(&[], now()), "The war room is empty.");
    }

    #[test]
    fn with_urgency_appends_when_the_text_is_one_line_and_changes_nothing_without_lines() {
        assert_eq!(with_urgency("a".into(), vec!["u".into()]), "a\n  u");
        assert_eq!(
            with_urgency("a\nb".into(), vec!["u".into(), "v".into()]),
            "a\n  u\n  v\nb"
        );
        assert_eq!(with_urgency("a\nb".into(), vec![]), "a\nb");
    }

    #[test]
    fn initiative_and_epic_show_add_their_urgent_lines() {
        let i = InitiativeDto {
            urgent_sources: vec![ref_("ABC-40", ItemType::Epic)],
            ..initiative_dto()
        };
        assert_eq!(
            initiative_show(&i, now()),
            format!("{}\n  urgent · via epic ABC-40", initiative(&i))
        );
        let e = EpicDto {
            urgent_sources: vec![ref_("ABC-9", ItemType::Story)],
            ..epic_dto()
        };
        assert_eq!(
            epic_show(&e, now()),
            format!("{}\n  urgent · via story ABC-9", epic(&e))
        );
    }

    #[test]
    fn context_puts_the_stories_urgent_line_right_after_its_title_line() {
        let c = StoryContextDto {
            story: StoryDto {
                urgent_sources: vec![ref_("ABC-40", ItemType::Epic)],
                ..dto(vec![])
            },
            epic: None,
            initiative: None,
            required_decisions: vec![],
            blockers: vec![],
            comments: vec![],
            readiness: ready(),
            goals: vec![],
        };
        let out = context_markdown(&c, now());
        let lines: Vec<&str> = out.lines().collect();
        let at = lines
            .iter()
            .position(|l| l.contains("[todo] Match"))
            .unwrap();
        assert_eq!(lines[at + 1], "  urgent · via epic ABC-40", "{out}");
        assert!(lines[at].contains(" !"), "{out}");
    }

    fn progress(goal_id: i64, completed: u32, in_flight: u32) -> GoalProgressDto {
        GoalProgressDto {
            goal_id,
            completed,
            in_flight,
            ..Default::default()
        }
    }

    fn counts(goals: Vec<GoalProgressDto>) -> SprintCountsDto {
        SprintCountsDto {
            goals,
            completed: 3,
            upkeep_completed: 1,
        }
    }

    /// kb/ADR-0014 R6: "at close" shows only when a number differs, compared per goal_id, never by order or by text.
    #[test]
    fn counts_differ_compares_numbers_per_goal_not_order() {
        let a = counts(vec![progress(1, 1, 2), progress(2, 0, 1)]);
        assert!(!counts_differ(&a, &a.clone()), "equal counts");
        let reordered = counts(vec![progress(2, 0, 1), progress(1, 1, 2)]);
        assert!(
            !counts_differ(&a, &reordered),
            "the same goals in another order"
        );
        let changed = counts(vec![progress(1, 2, 2), progress(2, 0, 1)]);
        assert!(counts_differ(&a, &changed), "one goal's completed differs");
        let one_side = counts(vec![progress(1, 1, 2)]);
        assert!(counts_differ(&a, &one_side), "a goal on one side only");
        assert!(
            counts_differ(&one_side, &a),
            "a goal on the other side only"
        );
        let totals = SprintCountsDto {
            upkeep_completed: 2,
            ..a.clone()
        };
        assert!(counts_differ(&a, &totals), "a total differs");
    }

    /// Every compared number on its own makes a difference: each goal field and each total.
    #[test]
    fn counts_differ_compares_every_number() {
        let a = counts(vec![progress(1, 1, 2)]);
        let goal_fields: [fn(&mut GoalProgressDto); 4] = [
            |g| g.completed += 1,
            |g| g.in_flight += 1,
            |g| g.todo += 1,
            |g| g.in_review += 1,
        ];
        for (i, f) in goal_fields.iter().enumerate() {
            let mut b = a.clone();
            f(&mut b.goals[0]);
            assert!(counts_differ(&a, &b), "goal field {i}");
        }
        let total_fields: [fn(&mut SprintCountsDto); 2] =
            [|c| c.completed += 1, |c| c.upkeep_completed += 1];
        for (i, f) in total_fields.iter().enumerate() {
            let mut b = a.clone();
            f(&mut b);
            assert!(counts_differ(&a, &b), "total field {i}");
        }
    }

    /// A goal on one side only is marked: not at close, or removed since close; one decision is singular.
    #[test]
    fn a_goal_on_one_side_only_is_marked() {
        let entry = SprintCountsDto {
            goals: vec![progress(99, 1, 0)],
            completed: 4,
            upkeep_completed: 1,
        };
        let mut now = progress(12, 1, 1);
        now.blocked_decisions = 1;
        let out = sprint_view(&view(SprintState::Closed, vec![now.clone()]), Some(&entry));
        assert!(out.contains("(not at close)"), "{out}");
        assert!(
            out.contains("goal #99 (removed since close) at close: completed: 1"),
            "{out}"
        );
        let active = sprint_view(&view(SprintState::Active, vec![now]), None);
        assert!(active.contains("blocked by 1 decision"), "{active}");
        assert!(!active.contains("blocked by 1 decisions"), "{active}");
    }

    /// "Blocked by N decisions" is live, not window history: it alone never makes a difference.
    #[test]
    fn counts_differ_ignores_blocked_decisions() {
        let a = counts(vec![progress(1, 1, 2)]);
        let mut b = a.clone();
        b.goals[0].blocked_decisions = 3;
        assert!(!counts_differ(&a, &b));
    }

    /// A closed sprint shows blocked-by-decisions from its closed entry, labelled at close; the active one the live value.
    #[test]
    fn a_closed_sprint_shows_blocked_decisions_at_close_only() {
        let mut live = progress(12, 1, 1);
        live.blocked_decisions = 0;
        let mut then = progress(12, 1, 1);
        then.blocked_decisions = 2;
        let entry = SprintCountsDto {
            goals: vec![then],
            completed: 4,
            upkeep_completed: 1,
        };
        let out = sprint_view(&view(SprintState::Closed, vec![live.clone()]), Some(&entry));
        assert!(out.contains("blocked by 2 decisions at close"), "{out}");
        assert!(!out.contains("blocked by 0"), "{out}");
        assert!(!out.contains("(at close:"), "{out}");
        assert!(!out.contains("changed since close"), "{out}");
        let active = sprint_view(&view(SprintState::Active, vec![live]), None);
        assert!(active.contains("blocked by 0 decisions"), "{active}");
        assert!(!active.contains("at close"), "{active}");
    }

    fn goal_dto(id: i64) -> GoalDto {
        GoalDto {
            id,
            sprint_id: 4,
            outcome: "Bookings work".into(),
            check: "a customer books".into(),
            owner: "dan@example.com".into(),
            notes: String::new(),
            confidence: Some(Confidence::AtRisk),
            confidence_note: Some("slow".into()),
            confidence_at: None,
            suggested_confidence: Some(Confidence::OnTrack),
            suggested_note: Some("2 in review".into()),
            suggested_at: None,
            suggested_by: Some("bot@example.com".into()),
            verdict: None,
            verdict_note: None,
            version: 3,
            links: vec!["ABC-1".parse().unwrap()],
        }
    }

    fn view(state: SprintState, goals: Vec<GoalProgressDto>) -> SprintViewDto {
        SprintViewDto {
            sprint: SprintDto {
                id: 4,
                state,
                starts_on: "2026-11-03".parse().unwrap(),
                ends_on: "2026-11-17".parse().unwrap(),
                opened_at: None,
                closed_at: None,
                version: 2,
                goals: vec![goal_dto(12)],
                warnings: vec![],
            },
            counts: SprintCountsDto {
                goals,
                completed: 4,
                upkeep_completed: 1,
            },
            upkeep_line: 20,
            demo: vec![DemoStoryDto {
                key: "ABC-7".parse().unwrap(),
                title: "Demo me".into(),
            }],
            promises: vec![],
            promises_after: None,
        }
    }

    /// A sprint lists its promises under the range heading, each row the key, title, recipient and state words, then the
    /// rule that a promise is to a date; a counted sprint without any says so, and a draft shows neither.
    #[test]
    fn sprint_view_lists_the_promises_due_in_the_sprint() {
        let mut v = view(SprintState::Closed, vec![progress(12, 1, 1)]);
        v.promises_after = Some("2026-11-02".parse().unwrap());
        v.promises = vec![
            reeve_protocol::SprintPromiseDto {
                key: "ABC-12".parse().unwrap(),
                title: "Export".into(),
                on: "2026-11-10".parse().unwrap(),
                to: Some("Acme".into()),
                holder: PromiseHolder::Own,
                epic_key: None,
                state: reeve_protocol::PromiseState::Kept,
                done_on: Some("2026-11-09".parse().unwrap()),
                days: None,
            },
            reeve_protocol::SprintPromiseDto {
                key: "ABC-13".parse().unwrap(),
                title: "Import".into(),
                on: "2026-11-14".parse().unwrap(),
                to: None,
                holder: PromiseHolder::Epic,
                epic_key: Some("ABC-7".parse().unwrap()),
                state: reeve_protocol::PromiseState::Late,
                done_on: Some("2026-11-16".parse().unwrap()),
                days: Some(2),
            },
        ];
        let out = sprint_view(&v, None);
        let want = format!(
            "Promises due 3–17 Nov:\n  {} Export · to Acme · Kept · 10 Nov\n  {} Import · Done 2 d late · 14 Nov · via ABC-7\n{PROMISE_RULE}",
            mark(Level::Story, "ABC-12".parse::<Key>().unwrap()),
            mark(Level::Story, "ABC-13".parse::<Key>().unwrap()),
        );
        assert!(out.contains(&want), "{out}");
        assert!(!out.contains("No promises due"), "{out}");

        let none = sprint_view(&view(SprintState::Active, vec![progress(12, 1, 1)]), None);
        assert!(none.contains("No promises due in this sprint."), "{none}");
        assert!(!none.contains(PROMISE_RULE), "{none}");

        let draft = sprint_view(&view(SprintState::Draft, vec![]), None);
        assert!(!draft.contains("romises"), "{draft}");
    }

    /// The rule line sits above the counts; a number that changed since the close shows both, an unchanged one once.
    #[test]
    fn sprint_view_states_the_rule_and_shows_at_close_only_where_it_differs() {
        let v = view(SprintState::Closed, vec![progress(12, 1, 1)]);
        let at = SprintCountsDto {
            goals: vec![progress(12, 1, 2)],
            completed: 4,
            upkeep_completed: 1,
        };
        let out = sprint_view(&v, Some(&at));
        let rule = out.find(MEMBERSHIP_RULE).unwrap_or_else(|| panic!("{out}"));
        assert!(rule < out.find("completed:").unwrap(), "{out}");
        assert!(out.contains("in flight: 1 (at close: 2)"), "{out}");
        assert!(out.contains("completed: 1 "), "{out}");
        assert!(!out.contains("completed: 1 (at close"), "{out}");
        assert!(
            out.contains("1 of 4 completed stories were upkeep (25%; team line 20%)"),
            "{out}"
        );
        assert!(out.contains("changed since close"), "{out}");
        let same = sprint_view(&v, None);
        assert!(!same.contains("at close"), "{same}");
        assert!(same.contains(MEMBERSHIP_RULE), "{same}");
        assert!(same.contains("suggested: on track"), "{same}");
        assert!(same.contains("ABC-7 Demo me"), "{same}");
        // A draft has no window: no counts, so no rule line.
        let draft = sprint_view(&view(SprintState::Draft, vec![]), None);
        assert!(!draft.contains(MEMBERSHIP_RULE), "{draft}");
        assert!(draft.contains("Bookings work"), "{draft}");
    }
}
