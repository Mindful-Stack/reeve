---
id: kb/ADR-0025
title: "ADR-0025: A promise is a dated, display-only commitment on a story or epic, classified against sprints on read"
description: "We decided that a person can promise a story or epic by a calendar date, stored on that item only and classified against the sprints' planned dates on read, because a commitment to someone outside the team must stay visible when missed without becoming a sprint field or a gate."
tags: [adr, reeve, planning, promises]
status: proposed
date: 2026-11-11
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: one-way
blast_radius: shared
sensitivity: [contract]
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve-protocol/**", "reeve:crates/reeve-server/**", "reeve:crates/reeve/**", "reeve:web/**"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0002, kb/ADR-0010, kb/ADR-0014, kb/ADR-0021]
related: [kb/ADR-0004, kb/ADR-0015, kb/ADR-0018]
implements: []
rfc:
---

# ADR-0025: A promise is a dated, display-only commitment on a story or epic, classified against sprints on read

## Status
Proposed 2026-11-11.

## Context
The team tells people outside it when work will be done: a customer bug "this sprint", an epic "next sprint". The
planner has no place for that. Stories carry no sprint field (kb/ADR-0014 R1), so that unfinished work is not carried
over silently from sprint to sprint. The urgent mark means "do now": it lasts until its story is done or a person
clears it, and a stale mark asks to be turned into a promise (kb/ADR-0024 R2, R4). It stays out of sprint views
(kb/ADR-0024 R7), so it cannot show which commitments a sprint kept. Sprints have planned dates (`starts_on`,
`ends_on`) and a real window (`opened_at` to `closed_at`); the next sprint exists as a draft without a window. Only
stories have a done state, and no done state for epics may be introduced (kb/ADR-0002 R4, kb/ADR-0024 R2). Readiness
and delivery must not depend on sprints (kb/ADR-0014 R4, kb/ADR-0018 R1). A required decision already carries its own
optional due date (kb/ADR-0004 R2).

## Facts relied on
- F1 Sprints store planned `starts_on`/`ends_on` dates besides the `opened_at`/`closed_at` window, and the schema notes the planned dates exist "for display and promise shortcuts only" — source: `crates/reeve-planning/migrations/1008_sprints.sql`.
- F2 At most one sprint is draft and at most one is active — source: `crates/reeve-planning/migrations/1008_sprints.sql` (`sprints_one_draft_idx`, `sprints_one_active_idx`).
- F3 A story's completion time is `done_at`; stories imported already done have none — source: `crates/reeve-planning/migrations/1005_story_done_at.sql`.
- F4 The decision queue places a decision in "needed this sprint" or "needed next sprint" with a reason per lane — source: `crates/reeve-protocol/src/enums.rs` (`NeededIn`, `NeededInReason`).

## Considered options
- **Promised for a sprint** (this, next, or a chosen sprint): matches how people phrase it, but it is the stored sprint field kb/ADR-0014 R1 forbids, and a slipped promise would have to be carried or dropped.
- **The urgent mark as the promise**: no new data, but it mixes "do now" with a commitment to a date, and kb/ADR-0024 R7 keeps it out of sprint views.
- **A date edited as an ordinary item field**: one edit path, but every promise bumps the item's version and conflicts with concurrent edits, and the people-only rule would sit inside the general edit.
- **A promises table with its history**: every past promise queryable, but the activity log already holds that history, and the table would be a second source of truth.
- **A date stored on the promised item by its own action, classified on read** (chosen): one write per promise, nothing stored that can go stale, and a missed date is visible without carrying anything over.

## Decision
We decided that a person can promise a story or an epic by a calendar date, stored only on that item by a dedicated
action and classified against the sprints' planned dates on read, because a commitment to someone outside the team
must stay visible when it is missed (a slipped promise keeps its date until a person changes it) without becoming a
sprint field, a carried-over bucket or a gate.

- **R1** A promise MUST be stored only on the promised story or epic, as a UTC calendar date, the person who set it, when, and an optional short recipient; no sprint reference and no derived promise state MAY be stored; initiatives MUST NOT be promised.
- **R2** Setting, changing and clearing a promise MUST be its own action that requires an interactive token, MUST NOT take or change the item's version, and MUST be logged under its own actions, never `override`; a done or dropped story MUST NOT be promised; the GitHub webhook and the importer MUST NOT read or write promises.
- **R3** A promise's window MUST be derived on read by one function from its date, today and the active and draft sprints' planned `ends_on`: past, this sprint, next sprint or later; every view, filter and queue rule that classifies promises MUST use it.
- **R4** A story's outcome MUST be derived on read from its effective date, the earlier of its own and its epic's: kept when done on or before it, broken when done after it, slipped when unfinished and past, otherwise open; a story's own promise and its epic's MUST both remain visible; an epic MUST NOT have an outcome of its own.
- **R5** A sprint shortcut MUST store the sprint's planned `ends_on` at the time of saving and MUST NOT follow later edits to the sprint; with no such sprint the shortcut MUST be refused; no date MAY be projected from the sprint length.
- **R6** Delivery validation, readiness, progress and rank MUST NOT depend on promises; the decision queue MAY place a decision blocking a promised story in the earliest lane its window gives, and MUST NOT write a decision's due date.
- **R7** Kept, broken and slipped MUST NOT be stored, at a sprint's close or elsewhere; a closed sprint's promises MUST be derived on read from the effective dates inside its planned dates.

## Consequences
- The story list gains a `promised` filter (kb/ADR-0021 R3: additive); item DTOs gain a computed promise; the decision queue gains a promise lane reason. Each is a wire change.
- Accepted enumerations grow, each recorded there as a later observation: kb/ADR-0002 R6's vocabulary ("promise", "slipped"), kb/ADR-0015 R7's interactive-only actions (R2's).
- kb/ADR-0024 R4's conversion of a stale mark can set a promise and clear the mark in one transaction; an urgent initiative cannot be converted (R1).
- Negative: work finished after a sprint's planned end but before it closed counts as that sprint's work (kb/ADR-0014 R2) while its promise is broken; views must say the promise is to a date.
- Negative: editing a promise date changes past sprints' derived lists; nothing records what a list showed at close.
- Negative: agents cannot promise; an agent can only suggest one in a comment.
- Negative: dates are UTC calendar days, so work finished shortly after midnight local time counts on the previous day.

## Assumptions and invalidation triggers
- *Assumes promises stay few compared with goal work.* Trigger: promised stories outnumber goal-linked stories in two consecutive sprints ⇒ revisit whether goals still lead planning (kb/ADR-0014).
- *Assumes a calendar date is precise enough.* Trigger: the team needs promises to a time of day or across time zones ⇒ revisit R1 and R3.
- *Assumes stakeholders read promises inside the planner.* Trigger: a request to publish promises outside the planner ⇒ new record.
- *Assumes derived past lists are good enough for retros.* Trigger: a retro disputes what a closed sprint's promises were because a date was edited ⇒ revisit R7.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0014, kb/ADR-0024.
