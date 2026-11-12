---
id: kb/ADR-0025
title: "ADR-0025: A promise is a dated, display-only commitment on a story or epic, classified against sprints on read"
description: "We decided that a person can promise a story or epic by a calendar date, stored on that item only and classified against the sprints' planned ends on read, because a commitment to someone outside the team must stay visible when missed without becoming a sprint field or a gate."
tags: [adr, reeve, planning, promises]
status: accepted
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
related: [kb/ADR-0004, kb/ADR-0015, kb/ADR-0018, kb/ADR-0024]
implements: []
rfc:
---

# ADR-0025: A promise is a dated, display-only commitment on a story or epic, classified against sprints on read

## Status
Proposed 2026-11-11.
Accepted 2026-11-12 by Daniel Thyselius.

## Context
The team tells people outside it when work will be done: a customer bug "this sprint", an epic "next sprint". The
planner has no place for that. Stories carry no sprint field (kb/ADR-0014 R1), so that unfinished work is not carried
over silently from sprint to sprint. The urgent mark means "do now": it lasts until its story is done or a person
clears it, and a stale mark asks to be turned into a promise (kb/ADR-0024 R2, R4). It stays out of sprint views
(kb/ADR-0024 R7), so it cannot show which commitments a sprint kept. Sprints have planned dates (`starts_on`,
`ends_on`) and a real window (`opened_at` to `closed_at`); the next sprint exists as a draft without a window, and
planned dates of neighbouring sprints may touch, overlap or leave gaps. Only stories have a done state, dropped is a
resolution of done, and no done state for epics may be introduced (kb/ADR-0002 R4, kb/ADR-0024 R2). Readiness and
delivery must not depend on sprints (kb/ADR-0014 R4, kb/ADR-0018 R1). A required decision already carries its own
optional due date (kb/ADR-0004 R2).

## Facts relied on
- F1 Sprints store planned `starts_on`/`ends_on` dates besides the `opened_at`/`closed_at` window; the schema requires only `ends_on > starts_on` and notes the planned dates exist "for display and promise shortcuts only" — source: `crates/reeve-planning/migrations/1008_sprints.sql`.
- F2 At most one sprint is draft and at most one is active — source: `crates/reeve-planning/migrations/1008_sprints.sql` (`sprints_one_draft_idx`, `sprints_one_active_idx`).
- F3 A story's completion time is `done_at`; stories imported already done have none — source: `crates/reeve-planning/migrations/1005_story_done_at.sql`.
- F4 The decision queue places a decision in "needed this sprint" or "needed next sprint" with a reason per lane, and the first rule that holds decides today — source: `crates/reeve-planning/src/domain/queue.rs` (`needed_in`); `crates/reeve-protocol/src/enums.rs` (`NeededIn`, `NeededInReason`).

## Considered options
- **Promised for a sprint** (this, next, or a chosen sprint): matches how people phrase it, but it is the stored sprint field kb/ADR-0014 R1 forbids, and a missed promise would have to be carried or dropped.
- **The urgent mark as the promise**: no new data, but it mixes "do now" with a commitment to a date, and kb/ADR-0024 R7 keeps it out of sprint views.
- **A date edited as an ordinary item field**: one edit path, but every promise bumps the item's version and conflicts with concurrent edits, and the people-only rule would sit inside the general edit.
- **A promises table with its history**: every past promise queryable, but the activity log already holds that history, and the table would be a second source of truth.
- **A date stored on the promised item by its own action, classified on read** (chosen): one write per promise, nothing stored that can go stale, and a missed date is visible without carrying anything over.

## Decision
We decided that a person can promise a story or an epic by a calendar date, stored only on that item by a dedicated
action and classified against the sprints' planned ends on read, because a commitment to someone outside the team must
stay visible when it is missed (an overdue promise keeps its date until a person changes it) without becoming a sprint
field, a carried-over bucket or a gate.

- **R1** A promise MUST be stored only on the promised story or epic, as a calendar date, the person who set it, when, and an optional short recipient; no sprint reference and no derived promise state MAY be stored; initiatives MUST NOT be promised.
- **R2** Setting, changing and clearing a promise MUST be its own action that requires an interactive token, MUST NOT take or change the item's version, and MUST be logged under its own actions, never `override`; a done story MUST NOT be promised, and a new date MUST NOT be before the request's UTC date; the GitHub webhook and the importer MUST NOT read or write promises.
- **R3** An epic's promise MUST apply to each of its stories that was not done when the epic's promise was set; a story's effective date MUST be the earlier of its own and the one it gets from its epic; both MUST remain visible.
- **R4** A story's promise state MUST be derived on read from its effective date, its delivery and the request's UTC date: kept when done on or before the date (the UTC day of `done_at`), late when done after it, dropped when dropped, overdue when unfinished after the date, otherwise open; a done story without `done_at` MUST have no state; an epic MUST NOT have a state of its own.
- **R5** An open promise's due period MUST be derived on read by one function from its date and the active and draft sprints' planned `ends_on`: this sprint when on or before the active `ends_on`, next sprint when on or before the draft `ends_on`, otherwise later; every view, filter and queue rule that classifies promises MUST use it.
- **R6** A sprint shortcut MUST store the sprint's planned `ends_on` at the time of saving and MUST NOT follow later edits to the sprint; a shortcut whose sprint does not exist or whose `ends_on` is past MUST be refused; no date MAY be projected from the sprint length.
- **R7** Delivery validation, readiness, progress and rank MUST NOT depend on promises; a decision blocking a story with an overdue or open promise MUST be needed no later than this sprint when overdue or due this sprint, and no later than next sprint when due next sprint; promises MUST NOT write a decision's due date.
- **R8** Kept, late, overdue and dropped MUST NOT be stored, at a sprint's close or elsewhere; a sprint's promises MUST be derived on read as the effective dates after the previous sprint's planned `ends_on` and up to its own.

## Consequences
- The story list gains a `promised` filter (kb/ADR-0021 R3: additive); item DTOs gain a computed promise; the decision queue gains a promise lane reason. Each is a wire change. The new queue reason breaks decoding in CLIs built before it, so the CLI ships in the same release and the reason enum gains a catch-all for later additions.
- Accepted enumerations grow, each recorded there as a later observation: kb/ADR-0002 R6's vocabulary ("promise", "overdue", "late"), kb/ADR-0015 R7's interactive-only actions (R2's), kb/ADR-0021 R2 and R4's filter list (`promised`), kb/ADR-0014's planned `ends_on` (now also classifying promises).
- kb/ADR-0024 R4's conversion of a stale mark can set a promise and clear the mark in one transaction; an urgent initiative cannot be converted (R1), recorded on kb/ADR-0024 as an observation.
- Negative: work finished after a sprint's planned end but before it closed counts as that sprint's work (kb/ADR-0014 R2) while its promise is late; views must say the promise is to a date.
- Negative: editing a promise date changes past sprints' derived lists; nothing records what a list showed at close.
- Negative: agents cannot promise; an agent can only suggest one in a comment.
- Negative: days are UTC days, so work finished shortly after midnight local time counts on the previous day.

## Assumptions and invalidation triggers
- *Assumes promises stay few compared with goal work.* Trigger: promised stories outnumber goal-linked stories in two consecutive sprints ⇒ revisit whether goals still lead planning (kb/ADR-0014).
- *Assumes a calendar date is precise enough.* Trigger: the team needs promises to a time of day or across time zones ⇒ revisit R1 and R4.
- *Assumes stakeholders read promises inside the planner.* Trigger: a request to publish promises outside the planner ⇒ new record.
- *Assumes derived past lists are good enough for retros.* Trigger: a retro disputes what a closed sprint's promises were because a date was edited ⇒ revisit R8.

## Later observations
<!-- append-only, accepted records only; newest last -->
- 2026-11-12: What was built beyond the rules: a request that changes nothing writes and logs nothing; `to: null` keeps the recipient; keeping the date keeps `promised_at` and changes only `by`; the story's own date wins a tie with its epic's; the row lock is `FOR NO KEY UPDATE`; a draft sprint lists no promises; a sprint's list starts after `previous_end` (the greatest planned end among the other non-draft sprints below its own, ties to the lower id), carried as `promises_after`, and `holder` and `today` are on the wire; and the CLI and the web tests share their word rows through `web/src/lib/promise.fixture.json` (the runtime tables are separate). F4 describes the queue as it was before this record was built: `needed_in` now takes the earliest window across the candidates, with rule order only as the tie-break, and `NeededInReason` gained `Promise` and `Unknown`. No trigger fired; the decision holds.

## See also
- kb/ADR-0014, kb/ADR-0024.
