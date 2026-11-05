---
id: kb/ADR-0013
title: "ADR-0013: A required decision carries a reversibility; a one-way decision is decided only by its accepted record"
description: "Each required decision carries a planner-side one-way or two-way classification and every close states decided or withdrawn; a one-way decision cannot be closed as decided, only withdrawn, because hard-to-undo calls must end in an accepted record."
tags: [adr, reeve, decisions, lorekeeper]
status: accepted
date: 2026-11-03
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve-protocol/src/**", "reeve:crates/reeve/src/commands/decision.rs", "reeve:web/src/lib/components/Decision*", "reeve:crates/reeve/src/git.rs", "reeve:web/src/lib/decisions.ts", "reeve:web/src/routes/decisions/**", "lorekeeper:references/reeve-cli.md", "lorekeeper:skills/planning-initiative/**", "lorekeeper:skills/refining-epic/**"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0003, kb/ADR-0004]
related: []
implements: []
rfc:
---

# ADR-0013: A required decision carries a reversibility; a one-way decision is decided only by its accepted record

## Status
Proposed 2026-11-03.
Accepted 2026-11-03 by Daniel Thyselius.

## Context
A required decision is a planner object with a title, a decider and an optional due date. An accepted record satisfies it, or a person closes it with a reason (kb/ADR-0004 R2). Closing lifts the gate the same way satisfaction does (kb/ADR-0003). Nothing records whether a close means the choice was made or that it is no longer needed. The planner also cannot tell a hard-to-undo choice from a reversible one. Records written with Lorekeeper already carry `reversibility: one-way | two-way` in their front matter, but a required decision is created before its record is written, and a two-way decision may never get one. The team is moving to decision-first planning. In that model a one-way choice is written down by one owner and accepted by a human, and a two-way choice is made quickly by a person and logged.

## Facts relied on
- F1 A required decision has title, decider, optional due, reference, PR url, check result and close fields (closed_at, closed_by, closed_reason), and no reversibility or closure kind — source: `crates/reeve-planning/migrations/1001_planning.sql` (`required_decisions`).
- F2 Readiness treats a closed or an accepted decision as not gating — source: `crates/reeve-planning/src/domain/readiness.rs`.
- F3 The check result's `not_accepted` variant carries the record's front-matter `status` and `superseded_by` and the default branch's commit, and `accepted` carries only that commit; the commit comes from resolving the remote default branch, not from front matter — source: `crates/reeve-protocol/src/enums.rs` (`CheckResult`), `crates/reeve/src/git.rs`.
- F4 Creating a required decision, directly or through a plan, logs `required`; an edit logs `edited` with `fields`, `from` and `to` — source: `crates/reeve-planning/src/handler/decisions.rs`, `crates/reeve-planning/src/handler/plans.rs`.
- F5 Lorekeeper's record template has a `reversibility` front-matter key — source: Lorekeeper `references/adr-template.md`.

## Considered options
- **Read reversibility from the record only** — one source of truth, but a decision without a record yet has no classification; the chosen planner field, with the record's value reported in the check result and a mismatch shown, classifies from creation and keeps disagreement visible.
- **A label and sort order only, with one undifferentiated close** — the least change, but a one-way call could still be made in a close reason, and the log could not tell a decision made from one dropped.
- **Require an accepted record for every decided close, whatever its reversibility** — the strongest written trail, but a reversible call then costs a record and a review, which is the process weight the two-way path exists to avoid.
- **Require a record only for one-way decisions, and a written outcome in the close for two-way ones** — chosen: ceremony follows reversibility, at the cost of a person classifying each decision by hand and the classification drifting from the record unless the mismatch is acted on.

## Decision
We decided that every required decision carries a planner-side reversibility and every close states whether the decision was decided or withdrawn, and that the planner refuses to close a one-way decision as decided, because hard-to-undo calls must end in a record a human accepted, while reversible calls stay one quick written step.

- **R1** A required decision MUST carry a reversibility that is `one_way`, `two_way` or unset. A create or a plan MAY set it, recorded in the `required` entry; any later change MUST be a versioned edit logged as `edited` with the old and new values. For closing and satisfaction, unset MUST behave exactly as two-way; the interface MAY present unset as a prompt to classify.
- **R2** The check MUST read the record's `reversibility` front-matter value (`one-way`, `two-way`) and report it inside the check result in the planner's own classification; a missing or unrecognised value MUST be reported as absent and MUST NOT change whether the record counts as accepted; a change in the reported value alone MUST count as a new outcome. The check MUST NOT change the decision's own reversibility; a person MAY adopt the reported value through a versioned edit. A mismatch MUST be shown as a warning only, and only when both values are set and differ.
- **R3** Every close MUST carry a kind, `decided` (the reason states what was decided) or `withdrawn` (the reason states why it is no longer needed), and MUST store it; closing a one-way decision as `decided` MUST be refused.
- **R4** A story's readiness and its readiness reasons MUST NOT depend on a decision's reversibility, on the reversibility its record reports or on its closure kind: an open unsatisfied decision gates whatever its classification, a closed decision of either kind stops gating, and satisfaction stays as kb/ADR-0003 R3 defines it.

## Consequences
- One-way decisions end in an accepted record or an explicit withdrawal, and the log tells the two apart. Two-way decisions end with the decision written in the close.
- R2 extends kb/ADR-0004 R4 by one parsed key and keeps R1 intact, because the value travels in the check result. A record without the key reports nothing and never blocks work.
- Negative: a withdrawal still lifts the gate, and so does reclassifying a one-way decision as two-way and deciding it; both leave a trail (`withdrawn`, or `edited` then `decided`) and the card lists the stories released, but nothing prevents either.
- Negative: every close caller must now pick a kind, existing decisions start unclassified, and someone has to classify them before the distinction is useful.

## Assumptions and invalidation triggers
- *Assumes one-way decisions rarely end without a record.* Trigger: at the end of a calendar quarter (UTC), of the distinct decisions that were one-way at any point in the quarter and were withdrawn, closed as decided or satisfied during it (each counted once, by its state at quarter end), more than a quarter ended withdrawn or decided, with at least four in the count; a smaller quarter is skipped ⇒ supersede R3 with a stricter rule. We review this count at each quarter's first planning sync.
- *Assumes the planner-side value is accurate enough.* Trigger: more than three open decisions have shown an unbroken mismatch for over 30 days, measured from the activity log (from the `checked` or `edited` entry that created the mismatch to the `checked`, `edited` or `linked` entry that ended it, or to now), not from `checked_at` ⇒ revisit R2 (for example, let the record's value win once a record is linked). Reviewed with the count above.

## Later observations
<!-- append-only, accepted records only; newest last -->
- 2026-11-04 (public copy, one-time edit): See also named an internal design document. The cited material is not in this repository; the sources now describe it in words or point to the public reeve-runner repository that holds it. No fact, rule or decision changed; the decision holds.

## See also
- kb/ADR-0004 (required decisions; R2 and R4, which this extends), kb/ADR-0003 (readiness).
- The decisions-first-class design of 2026-11-03 (internal, not published).
