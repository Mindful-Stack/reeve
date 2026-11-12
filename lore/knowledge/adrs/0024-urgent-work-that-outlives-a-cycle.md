---
id: kb/ADR-0024
title: "ADR-0024: An urgent mark lasts until its work is done or a person clears it, and turns stale after a cycle"
description: "An urgent mark stays active until its story is done or a person clears it, and after one sprint length it turns stale and asks the person who set it to renew, clear or promise it, because a mark that expires lets unfinished urgent work fall silently back into the backlog."
tags: [adr, reeve, planning, urgency]
status: accepted
date: 2026-11-11
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: two-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve-protocol/**", "reeve:crates/reeve-server/**", "reeve:crates/reeve/**", "reeve:web/**"]
supersedes: [kb/ADR-0017]
superseded_by:
depends_on: [kb/ADR-0002, kb/ADR-0010, kb/ADR-0014, kb/ADR-0018]
related: [kb/ADR-0012, kb/ADR-0015, kb/ADR-0016]
implements: []
rfc:
---

# ADR-0024: An urgent mark lasts until its work is done or a person clears it, and turns stale after a cycle

## Status
Proposed 2026-11-11.
Accepted 2026-11-11 by Daniel Thyselius.

## Context
kb/ADR-0017 made urgency a mark that lasts one cycle unless renewed. In the first war room (2026-11-10) the team used
marks two ways: for "do now" fires ("We are losing customers") and for work promised to a sprint goal. When a mark
expires, the item stops showing as urgent on the board, in the backlog and in the drawer; only the war room still lists
it as expired. Work that was urgent and is not finished in time then looks like every other backlog item, and nobody is
asked about it. The opposite failure is also real: a mark that never ends turns into a standing priority that nobody
reviews, which is what kb/ADR-0017 guarded against.

"Do now" is not "done this sprint". A commitment to finish something by a date is a separate concept, a promised-by date
on the item (planned, not yet recorded); an urgent mark that is no longer "now" but still owed belongs there. Only stories
have a done state; epics and initiatives have none, and kb/ADR-0017 refused to add one for urgency.

## Facts relied on
- F1 Only stories have a delivery state and a resolution; initiatives and epics have neither — source: `crates/reeve-planning/migrations/1001_planning.sql`; kb/ADR-0002 R4.
- F2 A mark is active until its timestamp plus the configured sprint length, and a done story's mark is inert — source: `crates/reeve-planning/src/domain/urgency.rs` `expires_at`; `crates/reeve-planning/src/handler/urgent.rs`.
- F3 The war room lists active marks first and expired marks after them, and re-mark renews every listed expired mark — source: `reeve war-room ls|re-mark`; kb/ADR-0017, Later observations.
- F4 The sprint length is instance configuration — source: kb/ADR-0016 R2.

## Considered options
- **Keep the one-cycle expiry (kb/ADR-0017 R2)**: every mark is reviewed at least once a cycle, but an unfinished mark stops showing as urgent everywhere except the war room, so urgent work falls back into the backlog unless someone reads that list.
- **A mark that never ends until done or cleared, with no review**: urgent work never disappears, but marks on epics and initiatives, which are never done (F1), stay forever, and urgency turns into a second priority system nobody prunes.
- **Expiry with a reminder** (a notification when a mark expires): reviews stay timely, but the item still stops showing as urgent in the meantime, and Reeve has no notification channel to carry the reminder.
- **Active until done or cleared, stale after one cycle and asking for an answer** (chosen): nothing urgent disappears without a person deciding so, and nothing stays urgent for longer than a cycle without someone confirming it.

## Decision
We decided that an urgent mark stays active until its story is done or a person clears it, and that after one sprint
length it turns stale, still urgent, and asks the person who set it to renew it, clear it or turn it into a promise,
because a mark that expires lets unfinished urgent work fall silently back into the backlog, while a mark nobody reviews
becomes a standing priority.

- **R1** The urgent mark MUST be stored only on the marked item, as a timestamp, a person and an optional reason; no derived urgency or staleness MAY be stored.
- **R2** A mark MUST stay active until a person clears it or, on a story, until the story is done; a mark MUST NOT end by time; marking again MUST replace the mark, which restarts its age; no done state for epics or initiatives MAY be introduced for it.
- **R3** A mark MUST be stale exactly when it is older than the configured sprint length; staleness MUST be computed on read; a stale mark MUST make items urgent exactly as an active mark does.
- **R4** Every view that shows a stale mark MUST show its age and offer to renew or clear it; once items can carry a promised-by date, it MUST also offer turning the mark into one, which sets the date and clears the mark in one action.
- **R5** An item MUST be shown as urgent exactly when a mark sits on it, on an ancestor or on a descendant, except a done story; a mark on a done story MUST NOT make any item urgent; urgency MUST NOT spread to siblings, and derived urgency MUST NOT spread further.
- **R6** Marking, renewing and clearing MUST require an interactive token and MUST be logged under their own actions, never `override`.
- **R7** Marking and clearing MUST NOT take or change the item's version; delivery validation, readiness, progress, goals and sprint views MUST NOT depend on urgency or staleness; the GitHub webhook and the importer MUST NOT read or write it.

## Consequences
- The war room lists active marks, then stale marks with their age, so its review step becomes a question that waits for an answer instead of a list of marks that have already stopped counting.
- The person who set a stale mark sees the question on their own home page once that page exists; until then, the war room is the only place it is asked.
- Marks on epics and initiatives end only when a person clears them; staleness is what keeps them reviewed.
- The CLI's and web's "expired" wording becomes "stale", and re-mark becomes renew on stale marks; the TUI changes with them.
- Negative: a mark nobody answers stays urgent indefinitely, and the board can fill with stale urgency if the war room is not reviewed.
- Negative: turning a mark into a promise waits for the promised-by date; until it ships, the only answers are renew and clear.
- Negative: an unfinished urgent item is still not shown in the sprint view or the sprint review (R7), so a sprint can close with stale marks that its verdicts never mention.

## Assumptions and invalidation triggers
- *Assumes urgent work is rare.* Trigger: more than three stale marks on two consecutive planning days ⇒ revisit with a cap or an automatic end.
- *Assumes the person who set a mark answers for it.* Trigger: stale marks whose setter has left the team or not answered within a further cycle ⇒ revisit who is asked.
- *Assumes one sprint length is the right age for the question.* Trigger: most stale marks are renewed without change over two months ⇒ revisit the age.
- *Assumes a person should decide what is urgent.* Trigger: the team asks for automated escalation (an incident hook, a CI failure) ⇒ revisit R6.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0017 (replaced), kb/ADR-0014 (sprints and goals; promises are not goals), kb/ADR-0002.
