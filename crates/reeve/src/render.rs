//! Short text renderers; `--json` bypasses all of these.
use crate::glyphs::{Level, mark};
use crate::promise;
use crate::relations::{BlockerLine, relations, web_links};
use crate::terminal::{block, line};
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
            format!(
                "decision #{decision_id} \"{}\" has no record linked",
                line(title)
            )
        }
        Reason::DecisionUnchecked { decision_id, title } => {
            format!(
                "decision #{decision_id} \"{}\" has not been checked",
                line(title)
            )
        }
        Reason::DecisionProposedPrOpen { decision_id, title } => {
            format!(
                "decision #{decision_id} \"{}\" is proposed; its PR is linked",
                line(title)
            )
        }
        Reason::DecisionRecordNotFound { decision_id, title } => {
            format!(
                "decision #{decision_id} \"{}\": record not found on the default branch",
                line(title)
            )
        }
        Reason::DecisionRepoNotConfigured { decision_id, title } => {
            format!(
                "decision #{decision_id} \"{}\": record in an unchecked repository",
                line(title)
            )
        }
        Reason::DecisionNotAccepted {
            decision_id,
            title,
            status,
            superseded_by,
        } => match superseded_by {
            Some(s) => {
                format!(
                    "decision #{decision_id} \"{}\": superseded by {}, re-point",
                    line(title),
                    line(s)
                )
            }
            None => format!(
                "decision #{decision_id} \"{}\" is {}",
                line(title),
                line(status)
            ),
        },
        Reason::BlockedBy {
            story,
            delivery,
            owner,
        } => blocked_by_sentence(&mark(Level::Story, story), *delivery, owner.as_deref()),
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

/// `text`, then `block` on the lines after it when there is one.
fn with_block(text: String, block: Option<String>) -> String {
    match block {
        Some(block) => format!("{text}\n{block}"),
        None => text,
    }
}

/// `story` for `story show`: with the urgent lines, then the promise line, after the first line, and the
/// relations block and the web links block last. A blocker is printed by its key and nothing else: a `StoryDto` holds no more of it, and
/// whether it holds the story back is the readiness reasons' to say, not this block's.
pub fn story_show(s: &StoryDto, now: DateTime<Utc>) -> String {
    story_with_relations(s, &[], now)
}

/// `story_show` for `story context`, which has the blocker stories: a blocker found in `blockers` is printed with
/// its title, state and owner.
pub fn story_with_relations(s: &StoryDto, blockers: &[StoryDto], now: DateTime<Utc>) -> String {
    let mut lines = urgent_lines(s.urgent_mark.as_ref(), &s.urgent_sources, &s.key, now);
    lines.extend(s.promise.as_ref().map(promise::line_text));
    let blocked_by: Vec<BlockerLine<'_>> = s
        .blocked_by
        .iter()
        .map(|key| BlockerLine {
            key,
            detail: blockers.iter().find(|b| b.key == *key),
        })
        .collect();
    with_block(
        with_block(
            with_urgency(story(s), lines),
            relations(&blocked_by, s.links.as_deref().unwrap_or(&[])),
        ),
        web_links(s.web_links.as_deref().unwrap_or(&[])),
    )
}

/// A recipient as the answer names it: ` · to <recipient>`, or nothing.
fn to_text(to: Option<&str>) -> String {
    to.filter(|t| !t.is_empty())
        .map(|t| format!(" · to {}", line(t)))
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

/// The answer to `story promise --clear`, from the DELETE's answer alone: `<key> · promise cleared`, or `<key> · no
/// promise to clear` when the story had no promise of its own; when its epic's promise still applies, `<key> · own
/// promise cleared; still promised by <day> via <epic>` or `<key> · no own promise to clear; promised by <day> via
/// <epic>` (the epic's promise is not the story's to clear).
pub fn story_promise_cleared(c: &StoryPromiseCleared) -> String {
    let key = mark(Level::of_kind(c.story.kind), &c.story.key);
    let epic = c.story.promise.as_ref().and_then(|p| p.epic.as_ref());
    match (c.cleared, epic) {
        (true, None) => format!("{key} · promise cleared"),
        (false, None) => format!("{key} · no promise to clear"),
        (true, Some(e)) => format!(
            "{key} · own promise cleared; still promised by {} via {}",
            promise::day_label(e.fact.on),
            e.key
        ),
        (false, Some(e)) => format!(
            "{key} · no own promise to clear; promised by {} via {}",
            promise::day_label(e.fact.on),
            e.key
        ),
    }
}

/// The answer to `epic promise --clear`, from the DELETE's answer: `<key> · promise cleared`, or `<key> · no promise to
/// clear` when it had none.
pub fn epic_promise_cleared(c: &EpicPromiseCleared) -> String {
    let key = mark(Level::Epic, &c.epic.key);
    if c.cleared {
        format!("{key} · promise cleared")
    } else {
        format!("{key} · no promise to clear")
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

/// `initiative` for `initiative show`: with the urgent lines after the first line, and the relations block
/// and the web links block last.
pub fn initiative_show(i: &InitiativeDto, now: DateTime<Utc>) -> String {
    with_block(
        with_block(
            with_urgency(
                initiative(i),
                urgent_lines(i.urgent_mark.as_ref(), &i.urgent_sources, &i.key, now),
            ),
            relations(&[], i.links.as_deref().unwrap_or(&[])),
        ),
        web_links(i.web_links.as_deref().unwrap_or(&[])),
    )
}

/// `epic` for `epic show`: with the urgent lines, then the promise line, after the first line, and the relations
/// block and the web links block last.
pub fn epic_show(e: &EpicDto, now: DateTime<Utc>) -> String {
    let mut lines = urgent_lines(e.urgent_mark.as_ref(), &e.urgent_sources, &e.key, now);
    lines.extend(e.promise.as_ref().map(promise::epic_line_text));
    with_block(
        with_block(
            with_urgency(epic(e), lines),
            relations(&[], e.links.as_deref().unwrap_or(&[])),
        ),
        web_links(e.web_links.as_deref().unwrap_or(&[])),
    )
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
                line(&r.title)
            )];
            if r.item.item != ItemType::Initiative
                && let Some(i) = &r.initiative
            {
                parts.push(format!("in {}", line(&i.title)));
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
        line(&s.title),
        story_chip(s),
        s.kind,
        s.version
    );
    if let Some(e) = &s.epic {
        let _ = write!(out, "  epic: {}", mark(Level::Epic, e));
    }
    if let Some(o) = &s.owner {
        let _ = write!(out, "  owner: {}", line(o));
    }
    if let Some(r) = &s.resolution {
        let _ = write!(out, "  resolution: {r:?}");
    }
    if !s.labels.is_empty() {
        let labels: Vec<_> = s.labels.iter().map(|l| line(l)).collect();
        let _ = write!(out, "  labels: {}", labels.join(","));
    }
    if let Some(r) = &s.external_ref {
        let _ = write!(out, "  from: {}", line(r));
    }
    for c in &s.criteria {
        let _ = write!(
            out,
            "\n  [{}] {}",
            if c.done { "x" } else { " " },
            line(&c.text)
        );
    }
    if !s.prs.is_empty() {
        let _ = write!(out, "\n  Pull requests:");
        for p in &s.prs {
            let _ = write!(
                out,
                "\n    {}#{} [{}] {} {}",
                line(&p.repo),
                p.number,
                pr_status_word(p.status),
                line(&p.title),
                line(&p.url)
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

/// The delivery state as prose ("in review"), where `state_word` is the hyphenated CLI token.
pub fn state_phrase(s: DeliveryState) -> &'static str {
    match s {
        DeliveryState::Todo => "todo",
        DeliveryState::InProgress => "in progress",
        DeliveryState::InReview => "in review",
        DeliveryState::Testing => "testing",
        DeliveryState::Done => "done",
    }
}

/// kb/ADR-0027's BlockedBy in full: what the blocker is doing and who can unblock it.
fn blocked_by_sentence(story: &str, delivery: DeliveryState, owner: Option<&str>) -> String {
    let state = state_phrase(delivery);
    let owner = owner.map(line);
    match (delivery, owner.as_deref()) {
        (DeliveryState::Todo, None) => {
            format!("blocked by {story}: todo, not yet started by anyone")
        }
        (DeliveryState::Todo, Some(o)) => format!("blocked by {story}: todo, owned by {o}"),
        (_, Some(o)) => format!(
            "blocked by {story}: {state}, owned by {o}; it unblocks when merged, or ask its owner"
        ),
        (_, None) => format!("blocked by {story}: {state}, no owner; it unblocks when merged"),
    }
}

pub fn story_row(s: &StoryDto, r: &Readiness) -> String {
    let ready_word = if r.ready {
        "ready".to_string()
    } else {
        r.reasons
            .first()
            .map(|r| match r {
                Reason::BlockedBy {
                    story, delivery, ..
                } => format!(
                    "blocked by {} ({})",
                    mark(Level::Story, story),
                    state_phrase(*delivery)
                ),
                other => reason(other),
            })
            .unwrap_or_else(|| "not ready".into())
    };
    format!(
        "{}{}\t{}\t{}\t{}{}",
        mark(Level::of_kind(s.kind), &s.key),
        urgent_marker(&s.urgent_sources),
        state_word(s.delivery),
        ready_word,
        line(&s.title),
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
            format!(
                "accepted @ {}",
                line(&commit.chars().take(8).collect::<String>())
            )
        }
        CheckResult::NotAccepted {
            status,
            superseded_by,
            ..
        } => match superseded_by {
            Some(s) => format!("superseded by {}, re-point", line(s)),
            None => line(status).into_owned(),
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
        line(&d.title),
        line(&d.decider),
        check_text(&d.check),
        d.version
    );
    if let Some(due) = d.due {
        let _ = write!(out, " due: {due}");
    }
    if let Some(r) = &d.record {
        let _ = write!(
            out,
            " record: {}:{} ({})",
            line(&r.repo),
            line(&r.path),
            line(&r.record_id)
        );
    }
    if let Some(pr) = &d.pr_url {
        let _ = write!(out, " pr: {}", line(pr));
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
            Some(ClosureKind::Decided) => write!(out, " closed: decided: {}", line(&c.reason)),
            Some(ClosureKind::Withdrawn) => write!(out, " closed: withdrawn: {}", line(&c.reason)),
            None => write!(out, " closed: {}", line(&c.reason)),
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
        line(&i.title),
        line(&i.horizon),
        i.version
    );
    if let Some(m) = &i.target_month {
        let _ = write!(out, " target: {}", line(m));
    }
    if let Some(o) = &i.owner {
        let _ = write!(out, " owner: {}", line(o));
    }
    if let Some(r) = &i.external_ref {
        let _ = write!(out, " from: {}", line(r));
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
        line(&e.title),
        epic_chip(e),
        e.version
    );
    if let Some(i) = &e.initiative {
        let _ = write!(out, " initiative: {}", mark(Level::Initiative, i));
    }
    if let Some(r) = &e.external_ref {
        let _ = write!(out, " from: {}", line(r));
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
        let text = match f {
            Finding::EmptyBrief => "the brief is empty".to_string(),
            Finding::BriefCitesWithoutDecision { cites } => format!(
                "the brief cites {} but no required decision is on this item or above it",
                cites.iter().map(|c| line(c)).collect::<Vec<_>>().join(", ")
            ),
            Finding::DiscoveryCompleteWithOpenDecision { decision_id, title } => {
                format!(
                    "discovery is complete while decision #{decision_id} \"{}\" is open",
                    line(title)
                )
            }
            Finding::DiscoveryCompleteWithoutStories => {
                "discovery is complete and there are no stories under it".to_string()
            }
        };
        let _ = write!(out, "\n  - {text}");
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
        // The header's text starts at column 4, past `  - `; the body's further lines start two further in.
        let _ = write!(out, "\n  - {}", comment_continued(c, "      "));
    }
    out
}

/// One comment, as `comment ls` lists them: its header (`<time> <author>: `) and its body, whose lines after the first
/// continue two spaces right of the header's column, so none can pass for another comment.
pub fn comment(c: &CommentDto) -> String {
    comment_continued(c, "  ")
}

/// A comment whose body's lines after the first start with `continuation`, which the caller sets right of the column
/// its comment headers' text (the time) starts at: stored text cannot take leading spaces away, so no line of a body
/// can start where a comment does, nor put a time in the column a header's time is in. The body is normalised first, so a lone `\r` is a line break here and never one the guard makes
/// after the layout. An empty line of the body stays empty.
fn comment_continued(c: &CommentDto, continuation: &str) -> String {
    let body = block(&c.body);
    let mut lines = body.split('\n');
    let mut out = format!(
        "{} {}: {}",
        c.at.format("%Y-%m-%d %H:%M"),
        line(&c.author),
        lines.next().unwrap_or_default()
    );
    for l in lines {
        if l.is_empty() {
            out.push('\n');
        } else {
            let _ = write!(out, "\n{continuation}{l}");
        }
    }
    out
}

/// The context bundle as Markdown at a given instant. Every required decision's record path appears as stored, unless it
/// holds a character that could drive the terminal; `--json` is the exact form. The story's urgent lines follow its
/// title line.
pub fn context_markdown(c: &StoryContextDto, now: DateTime<Utc>) -> String {
    let mut o = String::new();
    let _ = writeln!(
        o,
        "# Story\n\n{}\n",
        story_with_relations(&c.story, &c.blockers, now)
    );
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
            line(&d.title),
            line(&d.decider),
            check_text(&d.check)
        );
        if let Some(r) = &d.record {
            let _ = writeln!(
                o,
                "  record: {} (repo {}, id {})",
                line(&r.path),
                line(&r.repo),
                line(&r.record_id)
            );
        }
        if let Some(pr) = &d.pr_url {
            let _ = writeln!(o, "  pr: {}", line(pr));
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
            line(&b.title)
        );
    }
    let _ = writeln!(o, "\n## Criteria\n");
    if c.story.criteria.is_empty() {
        let _ = writeln!(o, "none");
    }
    for k in &c.story.criteria {
        let _ = writeln!(
            o,
            "- [{}] {}",
            if k.done { "x" } else { " " },
            line(&k.text)
        );
    }
    let _ = writeln!(o, "\n## Comments\n");
    if c.comments.is_empty() {
        let _ = writeln!(o, "none");
    }
    for k in &c.comments {
        // A list item whose text starts at column 2: the body's further lines start two further in, four spaces, which
        // CommonMark still reads as a continuation of the item's paragraph (an indented code block cannot interrupt a
        // paragraph, and it would need six after a blank line).
        let _ = writeln!(o, "- {}", comment_continued(k, "    "));
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
            // Each warning is one line by construction, so the whole line is one-line text.
            for l in &g.lines[..shown] {
                let _ = write!(out, "\n    {}", line(l));
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
        let _ = write!(out, "\n  {}: {n}", line(t));
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
            keys.iter().map(|k| line(k)).collect::<Vec<_>>().join(", ")
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
                    line(&f.jira_key),
                    f.step,
                    line(&f.message)
                );
            }
            let _ = write!(out, "\n  skipped: {}", a.skipped.len());
            for (k, why) in &a.skipped {
                let _ = write!(out, "\n    {}: {}", line(k), line(why));
            }
            if let Some(why) = &a.stopped {
                let _ = write!(
                    out,
                    "\n  stopped after {}: nothing more was sent",
                    line(why)
                );
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
        Some(n) if !n.is_empty() => format!("{word} ({})", line(n)),
        _ => word.to_string(),
    }
}

/// One goal: its outcome, check, owner, notes, confidence (and a pending suggestion), verdict and links.
pub fn goal(g: &GoalDto) -> String {
    let mut out = format!(
        "goal #{} {} (sprint #{}, version: {})\n  we'll know when: {}\n  owner: {}",
        g.id,
        line(&g.outcome),
        g.sprint_id,
        g.version,
        line(&g.check),
        line(&g.owner)
    );
    if !g.notes.is_empty() {
        // Normalised here, before `indent` splits the goal into lines: a lone `\r` the guard turned into a line break
        // after the layout would start a line at column 0. Each line after the first continues four spaces right of
        // the goal's header, so none can pass for a goal or start where a warning does.
        let notes = block(&g.notes);
        let mut lines = notes.lines();
        let _ = write!(out, "\n  notes: {}", lines.next().unwrap_or_default());
        for l in lines {
            if l.is_empty() {
                out.push('\n');
            } else {
                let _ = write!(out, "\n    {l}");
            }
        }
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
                .map(|b| format!(" by {}", line(b)))
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
                let _ = write!(out, "\n  {} {}", mark(Level::Story, &d.key), line(&d.title));
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
                    line(&p.title),
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
    let row = |s: &SprintDto| {
        let mut out = format!(
            "#{} [{}] {} – {}",
            s.id,
            sprint_state_word(s.state),
            s.starts_on,
            s.ends_on
        );
        for g in &s.goals {
            let _ = write!(out, "\n  goal #{} {}", g.id, line(&g.outcome));
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
        parts.push(row(&a.sprint));
    }
    if let Some(d) = &x.draft {
        parts.push(row(d));
    }
    parts.extend(x.closed.iter().map(row));
    if parts.is_empty() {
        return "no sprints yet: `reeve goal add` starts the first draft".into();
    }
    parts.join("\n")
}

/// A goal's initiatives in one line: `ABC-1 Now`, `ABC-5 Later !` for one outside the first bucket, `no initiative !`
/// when the links reach none. Without a configured bucket (`marked` false) nothing carries the mark.
fn goal_initiatives(g: &GoalGatesDto, marked: bool) -> String {
    let mark = |outside: bool| if marked && outside { " !" } else { "" };
    if g.initiatives.is_empty() {
        return format!("no initiative{}", mark(true));
    }
    g.initiatives
        .iter()
        .map(|i| format!("{} {}{}", i.key, line(&i.horizon), mark(!i.in_first_bucket)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// One goal's lines under its sprint: outcome, initiatives (or that the planning answer lacks the goal), owner, counts
/// when given (the active sprint's, under the membership rule, kb/ADR-0014 R2) and what its work waits on.
fn planned_goal(
    out: &mut String,
    g: &GoalDto,
    gates: Option<&GoalGatesDto>,
    counts: Option<&SprintCountsDto>,
    marked: bool,
) {
    let _ = write!(out, "\n  goal #{} {}", g.id, line(&g.outcome));
    match gates {
        Some(gg) => {
            let _ = write!(out, "\n    initiatives: {}", goal_initiatives(gg, marked));
        }
        None => out.push_str("\n    initiatives: (not in the planning answer)"),
    }
    let _ = write!(out, "\n    owner: {}", line(&g.owner));
    if let Some(c) = counts.and_then(|c| c.goals.iter().find(|c| c.goal_id == g.id)) {
        let _ = write!(
            out,
            "\n    completed: {}  in flight: {}  in review: {}  todo: {}",
            c.completed, c.in_flight, c.in_review, c.todo
        );
    }
    let Some(gg) = gates else { return };
    let n = gg.discoveries.len() + gg.decisions.len();
    if n == 0 {
        return;
    }
    let _ = write!(out, "\n    waits on {n}:");
    for d in &gg.discoveries {
        let item = match d.item {
            ItemType::Initiative => "initiative",
            // Discovery is a mark on initiatives and epics (kb/ADR-0002 R3): the server never gates on a story.
            ItemType::Epic | ItemType::Story => "epic",
        };
        let _ = write!(
            out,
            "\n      {} {}: {item} in discovery",
            d.key,
            line(&d.title)
        );
    }
    for d in &gg.decisions {
        let _ = write!(out, "\n      {} (on {})", reason(&d.reason), d.on);
    }
}

/// The first bucket's initiatives with the goals that reach them, each goal's outcome and sprint from the planning
/// answer itself.
fn bucket_section(bucket: &str, p: &SprintPlanningDto) -> String {
    let mut out = format!("{} initiatives", line(bucket));
    if p.first_bucket_initiatives.is_empty() {
        out.push_str("\n  none");
    }
    for i in &p.first_bucket_initiatives {
        let mut notes = vec![];
        if let Some(m) = &i.target_month {
            notes.push(line(m).into_owned());
        }
        if i.discovery_complete.is_none() {
            notes.push("in discovery".into());
        }
        let notes = if notes.is_empty() {
            String::new()
        } else {
            format!(" ({})", notes.join(", "))
        };
        let _ = write!(out, "\n  {} {}{notes}", i.key, line(&i.title));
        if i.goals.is_empty() {
            out.push_str("\n    no goal");
        }
        for g in &i.goals {
            let which = if g.active {
                "this sprint"
            } else {
                "next sprint"
            };
            let _ = write!(out, "\n    #{} {} ({which})", g.id, line(&g.outcome));
        }
    }
    out
}

/// `reeve sprint planning`: the active sprint ("This sprint") and the draft ("Next sprint"), each goal with the
/// initiatives its links reach, its owner, its counts when its sprint is active (under the membership rule,
/// kb/ADR-0014 R2) and what its work waits on; then the first horizon bucket's initiatives with their goals. The mark
/// `!` is for display: a goal may link to any initiative or none (kb/ADR-0014 R3).
pub fn sprint_planning(x: &SprintsDto, p: &SprintPlanningDto) -> String {
    let marked = p.first_bucket.is_some();
    let gates = |id: i64| p.goals.iter().find(|g| g.goal_id == id);
    let mut parts = vec![];
    if let Some(a) = &x.active {
        let s = &a.sprint;
        let mut out = format!(
            "This sprint #{} {} – {}\n{MEMBERSHIP_RULE}",
            s.id, s.starts_on, s.ends_on
        );
        for g in &s.goals {
            planned_goal(&mut out, g, gates(g.id), Some(&a.counts), marked);
        }
        parts.push(out);
    }
    if let Some(d) = &x.draft {
        let mut out = format!("Next sprint #{} {} – {}", d.id, d.starts_on, d.ends_on);
        for g in &d.goals {
            planned_goal(&mut out, g, gates(g.id), None, marked);
        }
        parts.push(out);
    }
    if parts.is_empty() {
        parts.push("no sprints yet: `reeve goal add` starts the first draft".into());
    }
    if let Some(bucket) = &p.first_bucket {
        parts.push(bucket_section(bucket, p));
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hostile::{Paths, assert_no_stored_text, assert_one_line, assert_shown};
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
            links: None,
            web_links: None,
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
            links: None,
            web_links: None,
        };
        let out = epic(&e);
        assert!(
            out.starts_with("■ ABC-2 Hosting") && out.contains("initiative: ◆ ABC-3"),
            "{out}"
        );
        let blocked = |delivery, owner: Option<&str>| {
            reason(&Reason::BlockedBy {
                story: "ABC-4".parse().unwrap(),
                delivery,
                owner: owner.map(str::to_string),
            })
        };
        assert_eq!(
            blocked(DeliveryState::Todo, None),
            "blocked by ● ABC-4: todo, not yet started by anyone"
        );
        assert_eq!(
            blocked(DeliveryState::Todo, Some("dan@example.com")),
            "blocked by ● ABC-4: todo, owned by dan@example.com"
        );
        assert_eq!(
            blocked(DeliveryState::InReview, Some("dan@example.com")),
            "blocked by ● ABC-4: in review, owned by dan@example.com; it unblocks when merged, or ask its owner"
        );
        assert_eq!(
            blocked(DeliveryState::InProgress, None),
            "blocked by ● ABC-4: in progress, no owner; it unblocks when merged"
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
            links: None,
            web_links: None,
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
            links: None,
            web_links: None,
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
            links: None,
            web_links: None,
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
            links: None,
            web_links: None,
        }
    }

    fn fact() -> PromiseFactDto {
        PromiseFactDto {
            on: "2026-11-20".parse().unwrap(),
            by: "dan@example.com".into(),
            at: now(),
            to: Some("Anna".into()),
        }
    }

    /// A story's promise in one of the three shapes `promise::line_text` words differently.
    fn promised(own: bool, epic: bool) -> StoryPromiseDto {
        StoryPromiseDto {
            own: own.then(fact),
            epic: epic.then(|| EpicPromiseRefDto {
                key: "ABC-2".parse().unwrap(),
                fact: fact(),
            }),
            on: "2026-11-20".parse().unwrap(),
            holder: if own {
                PromiseHolder::Own
            } else {
                PromiseHolder::Epic
            },
            state: Some(PromiseState::Open),
            due: Some(PromiseDue::ThisSprint),
            days: None,
        }
    }

    fn marked() -> UrgentMarkDto {
        UrgentMarkDto {
            reason: Some("customer waiting".into()),
            ..own_mark(true, false)
        }
    }

    fn web_link() -> WebLinkDto {
        WebLinkDto {
            id: 1,
            title: "Spec".into(),
            url: "https://example.com/spec".into(),
        }
    }

    /// A story with every list holding an entry and every optional field set, but its resolution (it is not done) and
    /// what `promise` leaves out: `story_paths` names those, from the shape `promise` was built in.
    fn full_story(promise: StoryPromiseDto) -> StoryDto {
        StoryDto {
            title: "Booking".into(),
            epic: Some("ABC-2".parse().unwrap()),
            description: "Long text".into(),
            criteria: vec![CriterionDto {
                text: "a customer books".into(),
                done: true,
            }],
            owner: Some("dan@example.com".into()),
            labels: vec!["web".into(), "api".into()],
            blocked_by: vec!["ABC-8".parse().unwrap()],
            external_ref: Some("SCRUM-1".into()),
            goals: vec![GoalRefDto {
                id: 12,
                outcome: "Bookings work".into(),
                via: GoalVia::Epic,
            }],
            urgent_mark: Some(marked()),
            urgent_sources: vec![ref_("ABC-1", ItemType::Story)],
            promise: Some(promise),
            links: Some(vec![item_link(
                ItemLinkType::Follows,
                ItemLinkDirection::Out,
                "ABC-9",
                ItemType::Story,
                "Earlier",
            )]),
            web_links: Some(vec![web_link()]),
            ..dto(vec![PullRequestDto {
                repo: "acme/portal".into(),
                number: 5,
                url: "https://github.com/acme/portal/pull/5".into(),
                title: "fix".into(),
                status: PrStatus::Open,
                updated_at: now(),
            }])
        }
    }

    /// What `full_story(promised(own, epic))` leaves empty, from the inputs that built it, so a fixture that loses a
    /// field fails the fullness check. No long text: the description is long only to a renderer that prints it
    /// (`story context`), which names it itself; to every other one it is one-line and must not be printed raw.
    fn story_paths(own: bool, epic: bool) -> Paths {
        let mut paths = Paths::none()
            .absent("/resolution", "a story not done has no resolution")
            .absent("/promise/days", "an open promise counts no days");
        if !own {
            paths = paths.absent("/promise/own", "the epic's promise alone");
        }
        if !epic {
            paths = paths.absent("/promise/epic", "the story's own promise alone");
        }
        paths
    }

    fn blocker_paths() -> Paths {
        story_paths(true, false)
    }

    /// A sprint goal with every optional field set.
    fn full_sprint_goal() -> GoalDto {
        GoalDto {
            id: 12,
            sprint_id: 4,
            outcome: "Bookings work".into(),
            check: "a customer books".into(),
            owner: "dan@example.com".into(),
            notes: "Long text".into(),
            confidence: Some(Confidence::AtRisk),
            confidence_note: Some("the host is late".into()),
            confidence_at: Some(now()),
            suggested_confidence: Some(Confidence::OffTrack),
            suggested_note: Some("two stories slipped".into()),
            suggested_at: Some(now()),
            suggested_by: Some("eva@example.com".into()),
            verdict: Some(Verdict::PartlyMet),
            verdict_note: Some("half of it".into()),
            version: 1,
            created_at: now(),
            links: vec!["ABC-1".parse().unwrap()],
        }
    }

    /// A blocker: a full story of its own, in progress.
    fn blocker() -> StoryDto {
        StoryDto {
            key: "ABC-8".parse().unwrap(),
            title: "Hosting".into(),
            owner: Some("eva@example.com".into()),
            delivery: DeliveryState::InProgress,
            blocked_by: vec!["ABC-9".parse().unwrap()],
            ..full_story(promised(true, false))
        }
    }

    /// The three promise shapes, each with the recipient and setter fields its words print in `show`, and those the
    /// promise answer prints.
    /// A promise shape as `promised`'s inputs (own, epic), the fields `show` prints and those the answer prints.
    type PromiseShape = ((bool, bool), Vec<&'static str>, Vec<&'static str>);

    /// Each shape is given as `promised`'s inputs (own, epic).
    fn promise_shapes() -> Vec<PromiseShape> {
        vec![
            (
                (true, true),
                vec!["/promise/own/to", "/promise/epic/fact/to"],
                vec!["/promise/own/to"],
            ),
            (
                (true, false),
                vec!["/promise/own/to", "/promise/own/by"],
                vec!["/promise/own/to"],
            ),
            (
                (false, true),
                vec!["/promise/epic/fact/to", "/promise/epic/fact/by"],
                vec!["/promise/epic/fact/by", "/promise/epic/fact/to"],
            ),
        ]
    }

    /// A story's one-line fields, in every form a story is printed in: `show`, its row, the promise answer and
    /// `story context`'s form with the blocker itself.
    #[test]
    fn a_storys_fields_stay_on_their_lines() {
        for ((own, epic), promise_fields, answer_fields) in promise_shapes() {
            let paths = story_paths(own, epic);
            let s = full_story(promised(own, epic));
            let shown = assert_one_line(&s, &paths, story);
            assert_shown(
                &shown,
                &[
                    "/title",
                    "/owner",
                    "/labels/0",
                    "/labels/1",
                    "/external_ref",
                    "/criteria/0/text",
                    "/prs/0/repo",
                    "/prs/0/title",
                    "/prs/0/url",
                ],
            );
            let shown = assert_one_line(&s, &paths, |s| story_show(s, now()));
            assert_shown(
                &shown,
                &[
                    "/urgent_mark/by",
                    "/urgent_mark/reason",
                    "/links/0/label",
                    "/links/0/item/title",
                    "/web_links/0/title",
                    "/web_links/0/url",
                ],
            );
            assert_shown(&shown, &promise_fields);
            let not_ready = Readiness {
                ready: false,
                reasons: text_reasons(),
            };
            assert_shown(
                &assert_one_line(
                    &(s.clone(), not_ready),
                    &paths
                        .clone()
                        .under("/0")
                        .and(reasons_paths().under("/1/reasons")),
                    |(s, r)| story_row(s, r),
                ),
                &["/0/title"],
            );
            assert_eq!(
                assert_one_line(&s, &paths, story_promise_answer),
                answer_fields
            );
            let shown = assert_one_line(
                &(s, vec![blocker()]),
                &paths.under("/0").and(blocker_paths().under("/1/*")),
                |(s, b)| story_with_relations(s, b, now()),
            );
            assert_shown(&shown, &["/1/0/title", "/1/0/owner"]);
        }
    }

    /// Every reason that carries text.
    fn text_reasons() -> Vec<Reason> {
        let title = || "Pick a host".to_string();
        vec![
            Reason::DecisionNoRecord {
                decision_id: 7,
                title: title(),
            },
            Reason::DecisionUnchecked {
                decision_id: 7,
                title: title(),
            },
            Reason::DecisionProposedPrOpen {
                decision_id: 7,
                title: title(),
            },
            Reason::DecisionRecordNotFound {
                decision_id: 7,
                title: title(),
            },
            Reason::DecisionRepoNotConfigured {
                decision_id: 7,
                title: title(),
            },
            Reason::DecisionNotAccepted {
                decision_id: 7,
                title: title(),
                status: "proposed".into(),
                superseded_by: None,
            },
            Reason::DecisionNotAccepted {
                decision_id: 7,
                title: title(),
                status: "superseded".into(),
                superseded_by: Some("kb/ADR-0009".into()),
            },
            Reason::BlockedBy {
                story: "ABC-8".parse().unwrap(),
                delivery: DeliveryState::Todo,
                owner: Some("eva@example.com".into()),
            },
            Reason::BlockedBy {
                story: "ABC-8".parse().unwrap(),
                delivery: DeliveryState::InProgress,
                owner: Some("eva@example.com".into()),
            },
        ]
    }

    /// Every reason's text: the seven titles, a status, a successor and two blockers' owners, eleven fields in all.
    /// (The superseded reason prints its successor, not its status.)
    #[test]
    fn a_reasons_text_stays_on_its_line() {
        assert_eq!(
            assert_one_line(&text_reasons(), &reasons_paths(), |rs| reasons(rs)).len(),
            11
        );
        let r = Readiness {
            ready: false,
            reasons: text_reasons(),
        };
        assert_eq!(
            assert_one_line(&r, &reasons_paths().under("/reasons"), readiness).len(),
            11
        );
    }

    /// What `text_reasons` leaves empty.
    fn reasons_paths() -> Paths {
        Paths::none().absent("/5/superseded_by", "a proposed record has no successor")
    }

    /// A required decision closed as `closed` says (`None`: by a server that does not say how), with every other
    /// optional field set; `decision_paths` names what `check` and `closed` leave empty.
    fn full_decision(check: CheckResult, closed: Option<ClosureKind>) -> RequiredDecisionDto {
        RequiredDecisionDto {
            id: 7,
            on: "ABC-1".parse().unwrap(),
            on_type: ItemType::Story,
            title: "Pick a host".into(),
            decider: "dan@example.com".into(),
            due: Some("2026-11-30".parse().unwrap()),
            reversibility: Some(Reversibility::OneWay),
            record: Some(RecordRefDto {
                repo: "acme/kb".into(),
                path: "adrs/0009-host.md".into(),
                record_id: "kb/ADR-0009".into(),
                browse_url: Some("https://github.com/acme/kb/blob/HEAD/adrs/0009-host.md".into()),
            }),
            pr_url: Some("https://github.com/acme/kb/pull/9".into()),
            check,
            checked_at: Some(now()),
            closed: Some(ClosedDto {
                at: now(),
                by: "dan@example.com".into(),
                reason: "chosen".into(),
                kind: closed,
                via: Some(TokenKind::Interactive),
            }),
            created_at: now(),
            version: 2,
        }
    }

    /// What `full_decision(check, closed)` leaves empty, from the inputs that built it, so a fixture that loses a field
    /// fails the fullness check.
    fn decision_paths(check: &CheckResult, closed: Option<ClosureKind>) -> Paths {
        let mut paths = Paths::none();
        if let CheckResult::NotAccepted {
            superseded_by: None,
            ..
        } = check
        {
            paths = paths.absent("/check/superseded_by", "a proposed record has no successor");
        }
        if closed.is_none() {
            paths = paths.absent("/closed/kind", "a server that does not say how it closed");
        }
        paths
    }

    /// The check results that print text, each with the field it prints.
    fn checks() -> [(CheckResult, &'static str); 3] {
        let commit = || "0123456789abcdef".to_string();
        [
            (
                CheckResult::Accepted {
                    commit: commit(),
                    reversibility: Some(Reversibility::OneWay),
                },
                "/check/commit",
            ),
            (
                CheckResult::NotAccepted {
                    status: "proposed".into(),
                    superseded_by: None,
                    commit: commit(),
                    reversibility: Some(Reversibility::TwoWay),
                },
                "/check/status",
            ),
            (
                CheckResult::NotAccepted {
                    status: "superseded".into(),
                    superseded_by: Some("kb/ADR-0010".into()),
                    commit: commit(),
                    reversibility: Some(Reversibility::OneWay),
                },
                "/check/superseded_by",
            ),
        ]
    }

    /// A required decision is one row wherever it is listed: its title, decider, record, pull request, check and
    /// closing reason cannot break it, whichever way it was closed.
    #[test]
    fn a_decisions_fields_stay_on_its_row() {
        for ((check, check_field), closed) in checks().into_iter().flat_map(|c| {
            [
                Some(ClosureKind::Decided),
                Some(ClosureKind::Withdrawn),
                None,
            ]
            .map(|k| (c.clone(), k))
        }) {
            let paths = decision_paths(&check, closed);
            let d = full_decision(check, closed);
            let fields = [
                "/title",
                "/decider",
                "/record/repo",
                "/record/path",
                "/record/record_id",
                "/pr_url",
                "/closed/reason",
                check_field,
            ];
            assert_shown(&assert_one_line(&d, &paths, decision), &fields);
            let listed = assert_one_line(&vec![d.clone(), d], &paths.clone().under("/*"), |ds| {
                decisions_block(ds)
            });
            for i in ["/0", "/1"] {
                let under: Vec<String> = fields.iter().map(|f| format!("{i}{f}")).collect();
                assert_shown(
                    &listed,
                    &under.iter().map(String::as_str).collect::<Vec<_>>(),
                );
            }
        }
    }

    fn discovered() -> DiscoveryMarkDto {
        DiscoveryMarkDto {
            at: now(),
            by: "dan@example.com".into(),
        }
    }

    /// An initiative with every optional field set and every list holding an entry; its brief is long text.
    fn full_initiative() -> InitiativeDto {
        InitiativeDto {
            title: "Leave the old host".into(),
            brief: "Long text".into(),
            target_month: Some("2026-12".into()),
            owner: Some("dan@example.com".into()),
            discovery_complete: Some(discovered()),
            progress: Some(0.5),
            external_ref: Some("JPD-30".into()),
            urgent_mark: Some(marked()),
            urgent_sources: vec![ref_("ABC-3", ItemType::Initiative)],
            links: Some(vec![item_link(
                ItemLinkType::RelatesTo,
                ItemLinkDirection::Out,
                "ABC-9",
                ItemType::Epic,
                "Other",
            )]),
            web_links: Some(vec![web_link()]),
            ..initiative_dto()
        }
    }

    fn full_epic() -> EpicDto {
        EpicDto {
            title: "Hosting".into(),
            brief: "Long text".into(),
            initiative: Some("ABC-3".parse().unwrap()),
            discovery_complete: Some(discovered()),
            external_ref: Some("SCRUM-480".into()),
            urgent_mark: Some(marked()),
            urgent_sources: vec![ref_("ABC-2", ItemType::Epic)],
            promise: Some(EpicPromiseDto {
                fact: fact(),
                due: Some(PromiseDue::ThisSprint),
                past: false,
            }),
            links: Some(vec![item_link(
                ItemLinkType::Duplicates,
                ItemLinkDirection::In,
                "ABC-9",
                ItemType::Epic,
                "Other",
            )]),
            web_links: Some(vec![web_link()]),
            ..epic_dto()
        }
    }

    /// An initiative's and an epic's one-line fields, in their one-line form, in `show` and in the promise answer;
    /// findings; the war room's rows.
    #[test]
    fn an_initiatives_and_an_epics_fields_stay_on_their_lines() {
        let i = full_initiative();
        assert_shown(
            &assert_one_line(&i, &Paths::none(), initiative),
            &[
                "/title",
                "/horizon",
                "/target_month",
                "/owner",
                "/external_ref",
            ],
        );
        assert_shown(
            &assert_one_line(&i, &Paths::none(), |i| initiative_show(i, now())),
            &[
                "/urgent_mark/reason",
                "/links/0/label",
                "/links/0/item/title",
                "/web_links/0/url",
            ],
        );
        let e = full_epic();
        assert_shown(
            &assert_one_line(&e, &Paths::none(), epic),
            &["/title", "/external_ref"],
        );
        assert_shown(
            &assert_one_line(&e, &Paths::none(), |e| epic_show(e, now())),
            &[
                "/urgent_mark/by",
                "/promise/fact/to",
                "/promise/fact/by",
                "/links/0/item/title",
            ],
        );
        assert_shown(
            &assert_one_line(&e, &Paths::none(), epic_promise_answer),
            &["/promise/fact/to"],
        );
        let fs = vec![
            Finding::BriefCitesWithoutDecision {
                cites: vec!["Jira".into(), "decided".into()],
            },
            Finding::DiscoveryCompleteWithOpenDecision {
                decision_id: 7,
                title: "Pick a host".into(),
            },
        ];
        assert_eq!(
            assert_one_line(&fs, &Paths::none(), |fs| findings(fs)).len(),
            3
        );
        let rows = vec![WarRoomEntryDto {
            mark: marked(),
            ..war_room_row("ABC-1", ItemType::Story, Some("Leave"), false)
        }];
        assert_shown(
            &assert_one_line(&rows, &Paths::none(), |rows| war_room_rows(rows, now())),
            &[
                "/0/title",
                "/0/initiative/title",
                "/0/mark/by",
                "/0/mark/reason",
            ],
        );
    }

    /// What a comment fixture (`full_comment`) leaves empty, and its long text.
    fn comment_paths() -> Paths {
        Paths::none()
            .long(&["/body"])
            .absent("/on_decision", "a comment on a story is on nothing else")
            .absent("/on_initiative", "a comment on a story is on nothing else")
            .absent("/on_epic", "a comment on a story is on nothing else")
    }

    fn full_comment() -> CommentDto {
        CommentDto {
            id: 1,
            on_story: Some("ABC-1".parse().unwrap()),
            on_decision: None,
            on_initiative: None,
            on_epic: None,
            author: "dan@example.com".into(),
            body: "Long text".into(),
            at: now(),
            external_ref: Some("SCRUM-1#c1".into()),
        }
    }

    /// A comment's author is one-line; its body is long text and is the guard's, with its lines kept.
    #[test]
    fn a_comments_author_stays_on_its_line_and_its_body_keeps_its_lines() {
        assert_eq!(
            assert_one_line(&full_comment(), &comment_paths(), comment),
            ["/author"]
        );
        let c = CommentDto {
            body: "first\r\nsecond \u{1b}[2J\u{202e}".into(),
            ..full_comment()
        };
        assert_eq!(
            crate::terminal::block(&comment(&c)),
            "2026-11-10 12:00 dan@example.com: first\n  second \\u{1b}[2J\\u{202e}"
        );
    }

    /// `story context`: every one-line field in every section; the description, the briefs and the comment bodies
    /// are long text and the guard's.
    #[test]
    fn the_context_bundles_one_line_fields_stay_on_their_lines() {
        // The decision the fixture holds and the paths it leaves empty come from the same values.
        let (check, closed) = (checks()[1].0.clone(), Some(ClosureKind::Decided));
        let ctx = StoryContextDto {
            story: full_story(promised(true, false)),
            epic: Some(full_epic()),
            initiative: Some(full_initiative()),
            required_decisions: vec![full_decision(check.clone(), closed)],
            blockers: vec![blocker()],
            comments: vec![full_comment()],
            readiness: Readiness {
                ready: false,
                reasons: text_reasons(),
            },
            goals: vec![GoalContextDto {
                goal: full_sprint_goal(),
                starts_on: "2026-11-09".parse().unwrap(),
                ends_on: "2026-11-20".parse().unwrap(),
                via: GoalVia::Epic,
            }],
            attachments: vec![AttachmentDto {
                id: uuid::Uuid::nil(),
                url: "https://reeve.example.com/attachments/1".into(),
                filename: "plan.pdf".into(),
                content_type: "application/pdf".into(),
                byte_size: 1024,
                uploaded_by: "dan@example.com".into(),
                uploaded_at: now(),
                owner: Some(AttachmentOwnerDto {
                    item: ItemType::Story,
                    key: "ABC-1".parse().unwrap(),
                }),
            }],
        };
        // The long text this renderer prints: the story's description, the two briefs and the comment bodies. The
        // blockers' descriptions and the goals' notes are not printed, so they stay one-line.
        let paths = story_paths(true, false)
            .under("/story")
            .and(decision_paths(&check, closed).under("/required_decisions/*"))
            .and(blocker_paths().under("/blockers/*"))
            .and(comment_paths().under("/comments/*"))
            .and(reasons_paths().under("/readiness/reasons"))
            .long(&["/story/description", "/epic/brief", "/initiative/brief"]);
        let shown = assert_one_line(&ctx, &paths, |c| context_markdown(c, now()));
        assert_shown(
            &shown,
            &[
                "/story/title",
                "/story/criteria/0/text",
                "/blockers/0/title",
                "/epic/title",
                "/initiative/title",
                "/required_decisions/0/title",
                "/required_decisions/0/decider",
                "/required_decisions/0/record/path",
                "/required_decisions/0/record/repo",
                "/required_decisions/0/record/record_id",
                "/required_decisions/0/check/status",
                "/required_decisions/0/pr_url",
                "/comments/0/author",
                "/readiness/reasons/0/title",
            ],
        );
    }

    /// Long text keeps its line breaks wherever it is printed: a renderer that forced a description, a brief or a
    /// comment's body through `line` would print `\n` here. The hostile helper cannot see that mistake (its long text
    /// holds no line break), so this test is its other half.
    #[test]
    fn long_text_keeps_its_line_breaks() {
        const TWO: &str = "first line\nsecond line";
        let body = CommentDto {
            body: TWO.into(),
            ..full_comment()
        };
        let ctx = StoryContextDto {
            story: StoryDto {
                description: TWO.into(),
                ..full_story(promised(true, false))
            },
            epic: Some(EpicDto {
                brief: TWO.into(),
                ..full_epic()
            }),
            initiative: Some(InitiativeDto {
                brief: TWO.into(),
                ..full_initiative()
            }),
            required_decisions: vec![],
            blockers: vec![],
            comments: vec![body.clone()],
            readiness: ready(),
            goals: vec![],
            attachments: vec![],
        };
        let md = context_markdown(&ctx, now());
        // The description, the initiative's brief and the epic's brief; the comment's body continues inside its list
        // item, and alone and under a `show` right of its header.
        assert_eq!(md.matches(TWO).count(), 3, "{md}");
        assert!(md.contains(": first line\n    second line\n"), "{md}");
        assert!(
            comment(&body).ends_with(": first line\n  second line"),
            "{}",
            comment(&body)
        );
        assert!(comments(std::slice::from_ref(&body)).ends_with(": first line\n      second line"));
    }

    /// An item's comments under its `show`: each author on its line; the bodies are long text.
    #[test]
    fn the_comments_authors_stay_on_their_lines() {
        let cs = vec![
            full_comment(),
            CommentDto {
                id: 2,
                ..full_comment()
            },
        ];
        assert_shown(
            &assert_one_line(&cs, &comment_paths().under("/*"), |cs| comments(cs)),
            &["/0/author", "/1/author"],
        );
    }

    /// Bodies that try to print a comment by another author where a renderer puts one, at `prefix`: after a line break
    /// and after a lone carriage return.
    fn forged_bodies(prefix: &str) -> [String; 2] {
        ["\n", "\r"].map(|br| format!("real{br}{prefix}2026-11-10 12:00 eva@example.com: forged"))
    }

    /// Two comments, the first with `body`: the one by dan carrying the forgery, a real one by eva after it.
    fn forged_thread(body: &str) -> Vec<CommentDto> {
        vec![
            CommentDto {
                body: body.into(),
                ..full_comment()
            },
            CommentDto {
                id: 2,
                author: "eva@example.com".into(),
                body: "ok".into(),
                ..full_comment()
            },
        ]
    }

    /// Whether `l`, after exactly `prefix`, starts like a comment's header: `YYYY-MM-DD HH:MM `, then an author and
    /// `: `.
    fn looks_like_a_comment_header(l: &str, prefix: &str) -> bool {
        let Some(rest) = l.strip_prefix(prefix) else {
            return false;
        };
        let shape = "dddd-dd-dd dd:dd ";
        rest.len() > shape.len()
            && rest
                .chars()
                .zip(shape.chars())
                .all(|(c, s)| if s == 'd' { c.is_ascii_digit() } else { c == s })
            && rest[shape.len()..].contains(": ")
    }

    /// Of `out`'s lines as printed, the only ones shaped like a comment header at `prefix` (where the renderer puts a
    /// comment's header) are the real comments' headers, taken from `cs`'s own times and authors, in order; and the
    /// forged line is printed at `continuation`, right of the header's column, with the prefix it imitates. The
    /// continuation is deeper than `prefix`, so a body's line starts right of the column a header's time starts at and
    /// a forged time never sits under a real one.
    fn assert_only_real_comment_headers(
        out: &str,
        prefix: &str,
        continuation: &str,
        cs: &[CommentDto],
    ) {
        assert!(
            continuation.len() > prefix.len() && continuation.trim().is_empty(),
            "{continuation:?} must be spaces right of {prefix:?}"
        );
        let printed = crate::terminal::block(out);
        let real: Vec<String> = cs
            .iter()
            .map(|c| format!("{prefix}{} {}: ", c.at.format("%Y-%m-%d %H:%M"), c.author))
            .collect();
        let headers: Vec<&str> = printed
            .lines()
            .filter(|l| looks_like_a_comment_header(l, prefix))
            .collect();
        assert_eq!(headers.len(), real.len(), "{headers:?} in\n{printed}");
        for (h, r) in headers.iter().zip(&real) {
            assert!(
                h.starts_with(r.as_str()),
                "{h:?} is not {r:?} in\n{printed}"
            );
        }
        let forged: Vec<&str> = printed.lines().filter(|l| l.contains("forged")).collect();
        assert_eq!(forged.len(), 1, "in\n{printed}");
        assert!(
            forged[0].starts_with(&format!(
                "{continuation}{prefix}2026-11-10 12:00 eva@example.com: forged"
            )),
            "{:?} in\n{printed}",
            forged[0]
        );
    }

    /// A comment's body continues indented right of its header's column in `comment ls` (one comment a line), so a
    /// line of it cannot pass for another comment. A one-line body prints as it always has.
    #[test]
    fn a_comments_body_cannot_pass_for_another_comment() {
        for body in forged_bodies("") {
            let cs = forged_thread(&body);
            let alone = cs.iter().map(comment).collect::<Vec<_>>().join("\n");
            assert_eq!(crate::terminal::block(&alone), alone, "{alone:?}");
            assert_only_real_comment_headers(&alone, "", "  ", &cs);
        }
        assert_eq!(
            comment(&full_comment()),
            "2026-11-10 12:00 dan@example.com: Long text"
        );
    }

    /// Under an item's `show` a comment's body continues right of the list's `- `, so a line of it cannot pass for
    /// another comment.
    #[test]
    fn a_comments_body_under_a_show_cannot_pass_for_another_comment() {
        for body in forged_bodies("  - ") {
            let cs = forged_thread(&body);
            let under = comments(&cs);
            assert_eq!(crate::terminal::block(&under), under, "{under:?}");
            assert_only_real_comment_headers(&under, "  - ", "      ", &cs);
        }
        assert_eq!(
            comments(&[full_comment()]),
            "comments:\n  - 2026-11-10 12:00 dan@example.com: Long text"
        );
    }

    /// In `story context` a comment is a Markdown list item, and its body's lines continue inside it, so a line of it
    /// cannot pass for another comment.
    #[test]
    fn a_comments_body_in_the_context_cannot_pass_for_another_comment() {
        for body in forged_bodies("- ") {
            let cs = forged_thread(&body);
            let ctx = StoryContextDto {
                comments: cs.clone(),
                ..bare_context()
            };
            let md = context_markdown(&ctx, now());
            assert_eq!(crate::terminal::block(&md), md, "{md:?}");
            assert_only_real_comment_headers(&md, "- ", "    ", &cs);
        }
    }

    /// A context bundle with nothing but its story.
    fn bare_context() -> StoryContextDto {
        StoryContextDto {
            story: full_story(promised(true, false)),
            epic: None,
            initiative: None,
            required_decisions: vec![],
            blockers: vec![],
            comments: vec![],
            readiness: ready(),
            goals: vec![],
            attachments: vec![],
        }
    }

    fn affected(key: &str, title: &str) -> AffectedStoryDto {
        AffectedStoryDto {
            key: key.parse().unwrap(),
            title: title.into(),
            kind: Kind::Story,
            delivery: DeliveryState::InProgress,
            owner: Some("eva@example.com".into()),
            urgent: true,
            needed_in: NeededIn::ThisSprint,
        }
    }

    /// The decision queue: each entry is its decision's row, whose fields cannot break it.
    #[test]
    fn a_queue_entrys_decision_stays_on_its_row() {
        // Through JSON: the entry's lane, window, next step and nesting take their defaults, as from an older server.
        let older: DecisionQueueEntryDto = serde_json::from_value(serde_json::json!({
            "decision": full_decision(checks()[0].0.clone(), Some(ClosureKind::Decided)),
            "group": "blocking",
            "blocks": [{"key": "ABC-1", "title": "Booking", "owner": "dan@example.com"}],
        }))
        .unwrap();
        let goal = || NeededInReason::Goal {
            goal_id: 12,
            outcome: "Bookings work".into(),
        };
        let full = DecisionQueueEntryDto {
            decision: full_decision(checks()[2].0.clone(), Some(ClosureKind::Withdrawn)),
            group: QueueGroup::Open,
            lane: QueueLane::ThisSprint,
            needed_in: NeededIn::ThisSprint,
            needed_in_reason: Some(goal()),
            next_step: Some(NextStep::ConfigureRepo {
                repo: "acme/kb".into(),
            }),
            blocks: vec![affected("ABC-4", "Checkout")],
            under_way: vec![affected("ABC-5", "Payments")],
            under: Some("ABC-3".parse().unwrap()),
        };
        let q = DecisionQueueDto {
            entries: vec![older, full],
            discoveries: vec![DiscoveryEntryDto {
                key: "ABC-3".parse().unwrap(),
                level: ItemType::Initiative,
                title: "Leave the old host".into(),
                also: Some("ABC-2".parse().unwrap()),
                lane: QueueLane::ThisSprint,
                needed_in_reason: Some(goal()),
                blocks: vec![affected("ABC-6", "Hosting")],
                under_way: vec![affected("ABC-7", "Moving")],
                at: 1,
            }],
            summary: DecisionQueueSummaryDto {
                open: 2,
                blocking: 1,
                blocked_stories: 1,
                overdue: 0,
                review: 0,
                this_sprint: 1,
                next_sprint: 0,
            },
        };
        let paths = Paths::none()
            .absent(
                "/entries/0/needed_in_reason",
                "an older server sends no window",
            )
            .absent("/entries/0/next_step", "an older server sends no next step")
            .absent(
                "/entries/0/under_way",
                "an older server sends no work under way",
            )
            .absent("/entries/0/under", "an older server sends no nesting");
        assert_shown(
            &assert_one_line(&q, &paths, queue),
            &[
                "/entries/0/decision/title",
                "/entries/0/decision/decider",
                "/entries/0/decision/record/path",
                "/entries/0/decision/closed/reason",
                "/entries/1/decision/title",
                "/entries/1/decision/check/superseded_by",
                "/entries/1/decision/closed/reason",
            ],
        );
    }

    /// The answer to marking an item urgent: the mark's author and reason, each on its line.
    #[test]
    fn an_urgent_answers_mark_stays_on_its_line() {
        let fixture = (
            "ABC-1".parse::<Key>().unwrap(),
            Some(marked()),
            vec![
                ref_("ABC-1", ItemType::Story),
                ref_("ABC-3", ItemType::Initiative),
            ],
        );
        let shown = assert_one_line(&fixture, &Paths::none(), |(key, own, sources)| {
            urgent_answer(Level::Story, key, own.as_ref(), sources, now())
        });
        assert_shown(&shown, &["/1/by", "/1/reason"]);
    }

    /// The answer to `story promise --clear` prints keys, a date and its own words only, in all four of its shapes:
    /// with every string of the story hostile, none reaches it.
    #[test]
    fn a_cleared_promise_prints_no_stored_text() {
        for (cleared, epic) in [(true, false), (false, false), (true, true), (false, true)] {
            let c = StoryPromiseCleared {
                cleared,
                story: full_story(promised(true, epic)),
            };
            assert_no_stored_text(
                &c,
                &story_paths(true, epic).under("/story"),
                story_promise_cleared,
            );
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
    fn a_list_row_shortens_only_blocked_by() {
        let r = Readiness {
            ready: false,
            reasons: vec![Reason::BlockedBy {
                story: "ABC-4".parse().unwrap(),
                delivery: DeliveryState::InReview,
                owner: Some("dan@example.com".into()),
            }],
        };
        let row = story_row(&dto(vec![]), &r);
        assert!(row.contains("blocked by ● ABC-4 (in review)"), "{row}");
        assert!(!row.contains("it unblocks"), "{row}");
        let d = Readiness {
            ready: false,
            reasons: vec![Reason::DecisionProposedPrOpen {
                decision_id: 7,
                title: "Use Postgres".into(),
            }],
        };
        let row = story_row(&dto(vec![]), &d);
        assert!(row.contains("decision #7 \"Use Postgres\""), "{row}");
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
            attachments: vec![],
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
        goal_named(id, 4, "Bookings work")
    }

    fn goal_named(id: i64, sprint_id: i64, outcome: &str) -> GoalDto {
        GoalDto {
            id,
            sprint_id,
            outcome: outcome.into(),
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
            created_at: "2026-11-03T09:00:00Z".parse().unwrap(),
            links: vec!["ABC-1".parse().unwrap()],
        }
    }

    fn bucket_goal(id: i64, outcome: &str, active: bool) -> BucketGoalDto {
        BucketGoalDto {
            id,
            outcome: outcome.into(),
            active,
        }
    }
    fn discovery_mark() -> DiscoveryMarkDto {
        DiscoveryMarkDto {
            at: "2026-11-01T10:00:00Z".parse().unwrap(),
            by: "dan@example.com".into(),
        }
    }

    fn reaches(key: &str, horizon: &str, in_first_bucket: bool) -> GoalInitiativeDto {
        GoalInitiativeDto {
            key: key.parse().unwrap(),
            title: format!("I {key}"),
            horizon: horizon.into(),
            in_first_bucket,
        }
    }
    fn gates_of(goal_id: i64, initiatives: Vec<GoalInitiativeDto>) -> GoalGatesDto {
        GoalGatesDto {
            goal_id,
            initiatives,
            discoveries: vec![],
            decisions: vec![],
            discovery_total: 0,
            decisions_total: 0,
        }
    }

    /// `reeve sprint planning`: both sprints' goals with their initiatives, the rule beside the counts, what a goal
    /// waits on, and the first bucket's initiatives with their goals.
    #[test]
    fn sprint_planning_lists_both_sprints_and_the_first_bucket() {
        // A counts row for a draft goal too: the text must not show it under the draft.
        let mut active = view(
            SprintState::Active,
            vec![progress(12, 1, 2), progress(20, 5, 5)],
        );
        active.sprint.id = 2;
        active.sprint.goals = vec![goal_named(12, 2, "Bookings work")];
        let mut draft = active.sprint.clone();
        draft.id = 3;
        draft.state = SprintState::Draft;
        draft.goals = vec![
            goal_named(20, 3, "Reports work"),
            goal_named(21, 3, "Tidy up"),
        ];
        let sprints = SprintsDto {
            active: Some(active),
            draft: Some(draft),
            closed: vec![],
            today: None,
        };
        let mut waiting = gates_of(12, vec![reaches("ABC-1", "Now", true)]);
        waiting.discoveries = vec![
            DiscoveryGateDto {
                key: "ABC-1".parse().unwrap(),
                item: ItemType::Initiative,
                title: "I ABC-1".into(),
            },
            DiscoveryGateDto {
                key: "ABC-3".parse().unwrap(),
                item: ItemType::Epic,
                title: "E ABC-3".into(),
            },
        ];
        waiting.decisions = vec![DecisionGateDto {
            on: "ABC-1".parse().unwrap(),
            reason: Reason::DecisionUnchecked {
                decision_id: 7,
                title: "Region".into(),
            },
        }];
        let p = SprintPlanningDto {
            goals: vec![
                waiting,
                gates_of(
                    20,
                    vec![
                        reaches("ABC-1", "Now", true),
                        reaches("ABC-5", "Later", false),
                    ],
                ),
                gates_of(21, vec![]),
            ],
            first_bucket: Some("Now".into()),
            first_bucket_initiatives: vec![
                BucketInitiativeDto {
                    key: "ABC-1".parse().unwrap(),
                    title: "I ABC-1".into(),
                    target_month: Some("2026-12".into()),
                    discovery_complete: None,
                    goals: vec![
                        bucket_goal(12, "Bookings work", true),
                        bucket_goal(20, "Reports work", false),
                    ],
                },
                BucketInitiativeDto {
                    key: "ABC-8".parse().unwrap(),
                    title: "I ABC-8".into(),
                    target_month: None,
                    discovery_complete: Some(discovery_mark()),
                    goals: vec![],
                },
            ],
        };
        let out = sprint_planning(&sprints, &p);
        assert!(out.contains("This sprint #2"), "{out}");
        assert!(out.contains(MEMBERSHIP_RULE), "{out}");
        assert!(
            out.contains("goal #12 Bookings work\n    initiatives: ABC-1 Now\n"),
            "{out}"
        );
        assert!(out.contains("completed: 1  in flight: 2"), "{out}");
        assert!(
            out.contains(
                "waits on 3:\n      ABC-1 I ABC-1: initiative in discovery\n      ABC-3 E ABC-3: epic in discovery\n      decision #7 \"Region\" has not been checked (on ABC-1)\nNext sprint #3"
            ),
            "{out}"
        );
        // One initiative outside the bucket marks only that one; a goal reaching none is marked too.
        assert!(
            out.contains("goal #20 Reports work\n    initiatives: ABC-1 Now, ABC-5 Later !"),
            "{out}"
        );
        assert!(
            out.contains("goal #21 Tidy up\n    initiatives: no initiative !"),
            "{out}"
        );
        assert!(
            out.contains("Now initiatives\n  ABC-1 I ABC-1 (2026-12, in discovery)\n    #12 Bookings work (this sprint)\n    #20 Reports work (next sprint)\n  ABC-8 I ABC-8\n    no goal"),
            "{out}"
        );
        // The draft has no window: no counts line under its goals.
        let next = out.split("\nNext sprint #3").nth(1).unwrap();
        assert!(!next.contains("completed:"), "{next}");
    }

    /// The two reads are two snapshots: a goal the planning answer lacks says so, and a bucket with no initiative says
    /// `none`.
    #[test]
    fn sprint_planning_says_so_when_the_two_reads_disagree() {
        let mut active = view(SprintState::Active, vec![progress(12, 0, 0)]);
        active.sprint.goals = vec![
            goal_named(12, active.sprint.id, "Bookings work"),
            goal_named(13, active.sprint.id, "Newly added"),
        ];
        let sprints = SprintsDto {
            active: Some(active),
            draft: None,
            closed: vec![],
            today: None,
        };
        let known = gates_of(12, vec![reaches("ABC-1", "Now", true)]);
        let mut p = SprintPlanningDto {
            goals: vec![known],
            first_bucket: Some("Now".into()),
            first_bucket_initiatives: vec![BucketInitiativeDto {
                key: "ABC-1".parse().unwrap(),
                title: "I ABC-1".into(),
                target_month: None,
                discovery_complete: Some(discovery_mark()),
                goals: vec![bucket_goal(12, "Bookings work", true)],
            }],
        };
        let out = sprint_planning(&sprints, &p);
        assert!(
            out.contains("goal #13 Newly added\n    initiatives: (not in the planning answer)\n"),
            "{out}"
        );
        assert!(
            out.contains("ABC-1 I ABC-1\n    #12 Bookings work (this sprint)"),
            "{out}"
        );
        assert!(!out.contains("no goal"), "{out}");
        p.first_bucket_initiatives = vec![];
        let out = sprint_planning(&sprints, &p);
        assert!(out.ends_with("Now initiatives\n  none"), "{out}");
    }

    /// Without a configured bucket nothing is marked and no bucket section prints.
    #[test]
    fn sprint_planning_marks_nothing_without_a_bucket() {
        let mut active = view(SprintState::Active, vec![progress(12, 0, 0)]);
        active.sprint.goals = vec![goal_named(12, active.sprint.id, "Bookings work")];
        let sprints = SprintsDto {
            active: Some(active),
            draft: None,
            closed: vec![],
            today: None,
        };
        let p = SprintPlanningDto {
            goals: vec![gates_of(12, vec![])],
            first_bucket: None,
            first_bucket_initiatives: vec![],
        };
        let out = sprint_planning(&sprints, &p);
        assert!(out.contains("initiatives: no initiative\n"), "{out}");
        assert!(!out.contains('!'), "{out}");
        assert!(!out.contains("initiatives\n  "), "{out}");
        let empty = sprint_planning(
            &SprintsDto {
                active: None,
                draft: None,
                closed: vec![],
                today: None,
            },
            &p,
        );
        assert_eq!(
            empty,
            "no sprints yet: `reeve goal add` starts the first draft"
        );
    }

    /// With no sprint yet the first bucket's initiatives still print: that is when someone starts planning.
    #[test]
    fn sprint_planning_lists_the_bucket_before_any_sprint_exists() {
        let p = SprintPlanningDto {
            goals: vec![],
            first_bucket: Some("Now".into()),
            first_bucket_initiatives: vec![BucketInitiativeDto {
                key: "ABC-1".parse().unwrap(),
                title: "I ABC-1".into(),
                target_month: None,
                discovery_complete: Some(discovery_mark()),
                goals: vec![],
            }],
        };
        let out = sprint_planning(
            &SprintsDto {
                active: None,
                draft: None,
                closed: vec![],
                today: None,
            },
            &p,
        );
        assert_eq!(
            out,
            "no sprints yet: `reeve goal add` starts the first draft\nNow initiatives\n  ABC-1 I ABC-1\n    no goal"
        );
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

    /// What a sprint fixture built from `full_view` leaves empty. No long text: each renderer that prints a goal's
    /// notes names them itself.
    fn sprint_absent() -> Paths {
        Paths::none()
            .absent("/closed_at", "a sprint not closed")
            .absent("/warnings", "a sprint of one goal has no warning")
    }

    /// An active sprint with a full goal, a demo story and a promise.
    fn full_view() -> SprintViewDto {
        let mut v = view(SprintState::Active, vec![progress(12, 1, 2)]);
        v.sprint.opened_at = Some("2026-11-03T09:00:00Z".parse().unwrap());
        v.sprint.goals = vec![full_sprint_goal()];
        v.promises = vec![SprintPromiseDto {
            key: "ABC-7".parse().unwrap(),
            title: "Demo me".into(),
            on: "2026-11-12".parse().unwrap(),
            to: Some("Anna".into()),
            holder: PromiseHolder::Own,
            epic_key: None,
            state: PromiseState::Open,
            done_on: None,
            days: None,
        }];
        v.promises_after = Some("2026-11-02".parse().unwrap());
        v
    }

    /// What `full_view` leaves empty.
    fn view_absent() -> Paths {
        sprint_absent()
            .under("/sprint")
            .absent("/promises/0/epic_key", "the story's own promise")
            .absent("/promises/0/done_on", "an open promise is not done")
            .absent("/promises/0/days", "an open promise counts no days")
    }

    fn draft_of(v: &SprintViewDto) -> SprintDto {
        SprintDto {
            state: SprintState::Draft,
            opened_at: None,
            ..v.sprint.clone()
        }
    }

    fn closed_of(v: &SprintViewDto) -> SprintDto {
        SprintDto {
            state: SprintState::Closed,
            closed_at: Some("2026-11-17T17:00:00Z".parse().unwrap()),
            ..v.sprint.clone()
        }
    }

    /// `sprint ls`'s and `sprint planning`'s fixture: `full_view` active, its draft, and (`closed`) a closed copy;
    /// with the paths it leaves empty, from the inputs that built it. No long text: neither renderer prints a goal's
    /// notes, so they are one-line there and must not be printed raw.
    fn full_sprints(closed: bool) -> (SprintsDto, Paths) {
        let v = full_view();
        let mut paths = view_absent().under("/active").and(
            sprint_absent()
                .under("/draft")
                .absent("/draft/opened_at", "a draft is not open"),
        );
        paths = if closed {
            paths.absent("/closed/0/warnings", "a sprint of one goal has no warning")
        } else {
            paths.absent("/closed", "no sprint closed yet")
        };
        let sprints = SprintsDto {
            active: Some(v.clone()),
            draft: Some(draft_of(&v)),
            closed: if closed { vec![closed_of(&v)] } else { vec![] },
            today: Some("2026-11-10".parse().unwrap()),
        };
        (sprints, paths)
    }

    /// A goal's one-line fields, alone and under its sprint, in `sprint show`, `sprint ls` and the open and close
    /// answers. `notes` is long text where it is printed (a goal alone, a sprint, `sprint show`).
    #[test]
    fn a_goals_fields_stay_on_their_lines() {
        let fields = [
            "/outcome",
            "/check",
            "/owner",
            "/confidence_note",
            "/suggested_note",
            "/suggested_by",
            "/verdict_note",
        ];
        assert_shown(
            &assert_one_line(&full_sprint_goal(), &Paths::none().long(&["/notes"]), goal),
            &fields,
        );
        let under: Vec<String> = fields.iter().map(|f| format!("/goals/0{f}")).collect();
        assert_shown(
            &assert_one_line(
                &full_view().sprint,
                &sprint_absent().long(&["/goals/*/notes"]),
                sprint,
            ),
            &under.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let under: Vec<String> = fields
            .iter()
            .map(|f| format!("/sprint/goals/0{f}"))
            .collect();
        let shown = assert_one_line(
            &full_view(),
            &view_absent().long(&["/sprint/goals/*/notes"]),
            |v| sprint_view(v, None),
        );
        assert_shown(
            &shown,
            &under.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        assert_shown(
            &shown,
            &["/demo/0/title", "/promises/0/title", "/promises/0/to"],
        );
        let (sprints, paths) = full_sprints(true);
        assert_shown(
            &assert_one_line(&sprints, &paths, sprint_list),
            &[
                "/active/sprint/goals/0/outcome",
                "/draft/goals/0/outcome",
                "/closed/0/goals/0/outcome",
                "/closed/0/goals/0/verdict_note",
            ],
        );
    }

    /// A goal's notes are long text and keep their line breaks, alone and indented under their sprint and in
    /// `sprint show`: a renderer that forced them through `line` would print `\n` here, which the hostile helper
    /// cannot see (its long text holds no line break). Their lines after the first continue four spaces further
    /// right than the goal's header, so none can start where a goal does.
    #[test]
    fn a_goals_notes_keep_their_line_breaks() {
        let g = GoalDto {
            notes: "first line\nsecond line".into(),
            ..full_sprint_goal()
        };
        let alone = goal(&g);
        assert!(
            alone.contains("\n  notes: first line\n    second line"),
            "{alone}"
        );
        assert_notes_continue_indented(&alone, "", &g);
        let mut v = full_view();
        v.sprint.goals = vec![g.clone()];
        // `indent` puts two spaces before each of the goal's lines, the notes' second line included.
        let under = sprint(&v.sprint);
        assert!(
            under.contains("\n    notes: first line\n      second line"),
            "{under}"
        );
        let shown = sprint_view(&v, None);
        assert!(
            shown.contains("\n    notes: first line\n      second line"),
            "{shown}"
        );
        for out in [&under, &shown] {
            assert_notes_continue_indented(out, "  ", &g);
        }

        // Notes that try to print another goal and a warning, with a line break and with a lone carriage return
        // (which `indent`, splitting on `\n` only, would leave for the guard to turn into one after the layout).
        for notes in [
            "real\ngoal #13 Forged (sprint #4, version: 1)\nwarning: forged",
            "real\rgoal #13 Forged (sprint #4, version: 1)\rwarning: forged",
        ] {
            let g = GoalDto {
                notes: notes.into(),
                ..full_sprint_goal()
            };
            let alone = goal(&g);
            // The guard has nothing left to turn into a line break: the lines printed are the lines laid out.
            assert_eq!(crate::terminal::block(&alone), alone, "{alone:?}");
            assert_only_real_goal_headers(&alone, "", &alone);
            assert_notes_continue_indented(&alone, "", &g);
            let mut v = full_view();
            v.sprint.goals = vec![g.clone()];
            for out in [sprint(&v.sprint), sprint_view(&v, None)] {
                assert_eq!(crate::terminal::block(&out), out, "{out:?}");
                assert_only_real_goal_headers(&out, "  ", &alone);
                assert_notes_continue_indented(&out, "  ", &g);
            }
        }
    }

    /// Of `out`'s lines as printed, every one between `g`'s `notes:` line and its next real field line (the line that
    /// follows `owner:` when the goal has no notes) is empty or starts four spaces right of the goal's fields, so a
    /// line of the notes cannot stand where a field does. `indent` is the goal's own column.
    fn assert_notes_continue_indented(out: &str, indent: &str, g: &GoalDto) {
        let printed = crate::terminal::block(out);
        let bare = goal(&GoalDto {
            notes: String::new(),
            ..g.clone()
        });
        let after_owner = bare
            .lines()
            .skip_while(|l| !l.starts_with("  owner: "))
            .nth(1)
            .expect("the fixture has a field after its owner");
        let next_field = format!("{indent}{after_owner}");
        let lines: Vec<&str> = printed.lines().collect();
        let notes = lines
            .iter()
            .position(|l| l.starts_with(&format!("{indent}  notes: ")))
            .expect("the notes are printed");
        let end = notes
            + 1
            + lines[notes + 1..]
                .iter()
                .position(|l| *l == next_field)
                .unwrap_or_else(|| panic!("{next_field:?} follows the notes in\n{printed}"));
        assert_eq!(
            lines.iter().filter(|l| **l == next_field).count(),
            1,
            "in\n{printed}"
        );
        let continuation = format!("{indent}    ");
        for l in &lines[notes + 1..end] {
            assert!(
                l.trim().is_empty() || l.starts_with(&continuation),
                "{l:?} is a line of the notes outside their indent in\n{printed}"
            );
        }
    }

    /// Of `out`'s lines as printed, the only ones that start with `indent` and then `goal #` (where a goal's header
    /// starts) are `goal`'s real header, `alone`'s first line, and no line starts at column 0 with `goal #` (unless
    /// that is the header's column) or `warning:`.
    fn assert_only_real_goal_headers(out: &str, indent: &str, alone: &str) {
        let printed = crate::terminal::block(out);
        let header = format!("{indent}{}", alone.lines().next().unwrap());
        let at = format!("{indent}goal #");
        let headers: Vec<&str> = printed.lines().filter(|l| l.starts_with(&at)).collect();
        assert_eq!(headers, [header.as_str()], "in\n{printed}");
        let column_0: Vec<&str> = printed
            .lines()
            .filter(|l| {
                (!indent.is_empty() && l.starts_with("goal #")) || l.starts_with("warning:")
            })
            .collect();
        assert!(column_0.is_empty(), "{column_0:?} in\n{printed}");
    }

    /// `sprint planning`: goals, the initiatives they reach, what they wait on, and the first bucket. It prints no
    /// long text.
    #[test]
    fn the_planning_views_fields_stay_on_their_lines() {
        let mut gates = gates_of(12, vec![reaches("ABC-3", "Now", false)]);
        gates.discoveries = vec![DiscoveryGateDto {
            key: "ABC-3".parse().unwrap(),
            item: ItemType::Initiative,
            title: "Leave".into(),
        }];
        gates.decisions = vec![DecisionGateDto {
            on: "ABC-1".parse().unwrap(),
            reason: Reason::DecisionNoRecord {
                decision_id: 7,
                title: "Pick a host".into(),
            },
        }];
        let planning = SprintPlanningDto {
            goals: vec![gates],
            first_bucket: Some("Now".into()),
            first_bucket_initiatives: vec![BucketInitiativeDto {
                key: "ABC-3".parse().unwrap(),
                title: "Leave".into(),
                target_month: Some("2026-12".into()),
                discovery_complete: Some(DiscoveryMarkDto {
                    at: "2026-11-01T09:00:00Z".parse().unwrap(),
                    by: "dan@example.com".into(),
                }),
                goals: vec![bucket_goal(12, "Bookings work", true)],
            }],
        };
        let (sprints, paths) = full_sprints(false);
        let shown = assert_one_line(&(sprints, planning), &paths.under("/0"), |(x, p)| {
            sprint_planning(x, p)
        });
        assert_shown(
            &shown,
            &[
                "/0/active/sprint/goals/0/outcome",
                "/0/active/sprint/goals/0/owner",
                "/0/draft/goals/0/outcome",
                "/0/draft/goals/0/owner",
                "/1/first_bucket",
                "/1/goals/0/initiatives/0/horizon",
                "/1/goals/0/discoveries/0/title",
                "/1/goals/0/decisions/0/reason/title",
                "/1/first_bucket_initiatives/0/title",
                "/1/first_bucket_initiatives/0/target_month",
                "/1/first_bucket_initiatives/0/goals/0/outcome",
            ],
        );
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

    fn item_link(
        t: ItemLinkType,
        direction: ItemLinkDirection,
        key: &str,
        level: ItemType,
        title: &str,
    ) -> ItemLinkDto {
        ItemLinkDto {
            link_type: t,
            direction,
            label: t.label(direction).to_string(),
            item: LinkedItemDto {
                key: key.parse().unwrap(),
                level,
                title: title.into(),
            },
        }
    }

    /// The block as the spec draws it: blockers first, labels padded to the longest shown, each key with its
    /// glyph. `story show` has only the blocker's key, and prints nothing after it (no claim that the blocker
    /// holds the story back: this block does not know); with no blockers and no links there is no block at all.
    #[test]
    fn show_prints_blockers_then_item_links_and_nothing_when_there_are_none() {
        use ItemLinkDirection::{In, Out};
        let links = vec![
            item_link(
                ItemLinkType::Follows,
                Out,
                "ABC-25",
                ItemType::Story,
                "Billing account",
            ),
            item_link(
                ItemLinkType::Follows,
                In,
                "ABC-31",
                ItemType::Story,
                "Dunning",
            ),
            item_link(
                ItemLinkType::RelatesTo,
                Out,
                "ABC-40",
                ItemType::Epic,
                "Reminder emails",
            ),
        ];
        let s = StoryDto {
            blocked_by: vec!["ABC-28".parse().unwrap()],
            links: Some(links.clone()),
            ..dto(vec![])
        };
        let out = story_show(&s, now());
        assert_eq!(
            out,
            format!(
                "{}\nrelations:\n  blocked by      ● ABC-28\n  follows         ● ABC-25 Billing account\n  is followed by  ● ABC-31 Dunning\n  relates to      ■ ABC-40 Reminder emails",
                story(&s)
            )
        );
        // Nothing to show: exactly the text from before this block existed.
        for links in [None, Some(vec![])] {
            let plain = StoryDto {
                links,
                ..dto(vec![])
            };
            assert_eq!(story_show(&plain, now()), story(&plain));
        }
        // An epic and an initiative have no blockers; the width follows the labels shown.
        let e = EpicDto {
            links: Some(vec![links[2].clone()]),
            ..epic_dto()
        };
        assert_eq!(
            epic_show(&e, now()),
            format!(
                "{}\nrelations:\n  relates to  ■ ABC-40 Reminder emails",
                epic(&e)
            )
        );
        let i = InitiativeDto {
            links: Some(vec![links[0].clone()]),
            ..initiative_dto()
        };
        assert_eq!(
            initiative_show(&i, now()),
            format!(
                "{}\nrelations:\n  follows  ● ABC-25 Billing account",
                initiative(&i)
            )
        );
    }

    /// `story context` has the blocker stories, so its blocker line says what the blocker is doing and whose it is,
    /// and stops there: the line ends after the owner, with no word on whether the blocker holds the story back.
    #[test]
    fn context_prints_the_blocker_with_its_title_state_and_owner() {
        let blocker = StoryDto {
            key: "ABC-28".parse().unwrap(),
            title: "Invoice template".into(),
            delivery: DeliveryState::InReview,
            owner: Some("mia@example.com".into()),
            ..dto(vec![])
        };
        let unowned = StoryDto {
            key: "ABC-29".parse().unwrap(),
            title: "Numbering".into(),
            kind: Kind::Bug,
            ..dto(vec![])
        };
        let c = StoryContextDto {
            story: StoryDto {
                blocked_by: vec![blocker.key.clone(), unowned.key.clone()],
                links: Some(vec![item_link(
                    ItemLinkType::Duplicates,
                    ItemLinkDirection::In,
                    "ABC-50",
                    ItemType::Story,
                    "Old invoice",
                )]),
                ..dto(vec![])
            },
            epic: None,
            initiative: None,
            required_decisions: vec![],
            blockers: vec![blocker, unowned],
            comments: vec![],
            readiness: ready(),
            goals: vec![],
            attachments: vec![],
        };
        let out = context_markdown(&c, now());
        assert!(
            out.contains(
                "relations:\n  blocked by        ● ABC-28 Invoice template (in review, mia@example.com)\n  blocked by        ▲ ABC-29 Numbering (todo, no owner)\n  is duplicated by  ● ABC-50 Old invoice\n"
            ),
            "{out}"
        );
        assert!(
            out.contains("\n## Blockers\n\n- ● ABC-28 [in-review] Invoice template"),
            "the existing section stays: {out}"
        );
    }

    /// The web links block comes after the relations block, on all three levels and in the context, and is
    /// absent when there are none.
    #[test]
    fn show_prints_web_links_after_the_relations() {
        let page = WebLinkDto {
            id: 1,
            title: "Scenario page".into(),
            url: "https://example.com/wiki/scenario".into(),
        };
        let block = "web links:\n  Scenario page  https://example.com/wiki/scenario";
        let s = StoryDto {
            blocked_by: vec!["ABC-28".parse().unwrap()],
            web_links: Some(vec![page.clone()]),
            ..dto(vec![])
        };
        assert_eq!(
            story_show(&s, now()),
            format!("{}\nrelations:\n  blocked by  ● ABC-28\n{block}", story(&s))
        );
        let e = EpicDto {
            web_links: Some(vec![page.clone()]),
            ..epic_dto()
        };
        assert_eq!(epic_show(&e, now()), format!("{}\n{block}", epic(&e)));
        let i = InitiativeDto {
            web_links: Some(vec![page]),
            ..initiative_dto()
        };
        assert_eq!(
            initiative_show(&i, now()),
            format!("{}\n{block}", initiative(&i))
        );
        let plain = StoryDto {
            web_links: Some(vec![]),
            ..dto(vec![])
        };
        assert_eq!(story_show(&plain, now()), story(&plain));
    }
}
