---
id: kb/ADR-0026
title: "ADR-0026: Opening a sprint chooses which of the draft's goals go in; the rest stay in the next draft"
description: "We decided that the person opening the draft sprint chooses which of its goals stay out, and that those goals move with their links to the next draft, because leaving a goal out today means removing it and losing its links and wording, and moving it is not a carry-over."
tags: [adr, reeve, planning, sprints, goals]
status: accepted
date: 2026-11-12
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: two-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve-protocol/**", "reeve:crates/reeve-server/**", "reeve:web/**"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0014]
related: [kb/ADR-0002, kb/ADR-0015, kb/ADR-0016, kb/ADR-0018, kb/ADR-0025]
implements: []
rfc:
---

# ADR-0026: Opening a sprint chooses which of the draft's goals go in; the rest stay in the next draft

## Status
Proposed 2026-11-12.
Accepted 2026-11-12 by Daniel Thyselius.

## Context
kb/ADR-0014 decides active and closed sprints. The planner also keeps a draft, the next sprint, which kb/ADR-0025
relies on (its F2) but no record decides. The draft has no window, so it counts no work (kb/ADR-0014 R2). There is at
most one; it is created by the first goal added or by the opening of the previous draft, and dated at creation from
the active sprint's end, or today, plus the configured sprint length (kb/ADR-0016 R2). Its goals are created, edited
and removed by interactive tokens (kb/ADR-0014 R5), can be removed only while it is a draft, and place work in the
readiness queue's next-sprint lane without touching readiness (kb/ADR-0014 R3, kb/ADR-0018 R1). Opening turns the same
row active, so every goal it holds goes in.

The PM plans the next sprint during the current one, and people and agents link work to its goals. To leave a goal
out of a sprint today, the PM removes it before opening; its links and wording are lost and have to be written again
for a later sprint. In this record a *candidate* is a draft's goal: a role, not a stored concept, and not bound
vocabulary under kb/ADR-0002 R6 (the API and the UI keep saying goal).

## Facts relied on
- F1 At most one draft exists, created by the first goal or at an opening and dated at creation — source: `crates/reeve-planning/migrations/1008_sprints.sql` (`sprints_one_draft_idx`); `crates/reeve-planning/src/handler/sprints.rs` (`draft_in`, `open_sprint`); `crates/reeve-planning/src/domain/sprint.rs` (`draft_dates`).
- F2 A goal can be removed only while its sprint is a draft, and a draft takes no confidence — source: `crates/reeve-planning/src/domain/sprint.rs` (`allow`).
- F3 Goal writes read a goal's sprint unlocked, then lock that sprint, then the goal — source: `crates/reeve-planning/src/handler/sprints.rs` (`goal_locked`, `unlink_goal`).
- F4 Adding a goal does not change its sprint's version — source: `crates/reeve-planning/src/repo/sprints.rs` (`insert_goal`).
- F5 The queue reads draft goals by joining the sprint's state — source: `crates/reeve-planning/src/repo/reads.rs` (`goals_of_stories(.., "draft")`).

## Considered options
- **Every draft goal goes in** (today): nothing to build; but leaving a goal out means removing it and writing it again later.
- **A stored in-or-out flag on each draft goal**: the choice is visible before the opening; but it adds state to goals and asks for the choice twice.
- **A separate candidates entity promoted to goals at open**: candidates and goals never mix; but links, the queue, the CLI and the web would each handle two near-identical models.
- **The opening person names the goals to leave out; they move with their links to the next draft** (chosen): nothing new is stored, a goal added meanwhile goes in as today, and a left-out goal keeps its links and wording.

## Decision
We decided that the person opening the draft sprint chooses which of its goals stay out, and that those goals move with
their links to the next draft created by the same opening, because leaving a goal out today means removing it and
losing its links and wording, and moving a goal that never belonged to an opened sprint is not a carry-over.

- **R1** Opening a draft MUST take a set of the draft's goals to leave out, empty by default; every other goal MUST become the opened sprint's with its links, and every goal left out MUST move with its links to the draft the same opening creates; a goal that is not the draft's MUST refuse the whole opening.
- **R2** A goal MUST change sprint only by R1; an active or closed sprint's goals MUST NOT move; a goal write MUST be decided by the state of the sprint the goal belongs to when the write applies.

## Consequences
- The PM keeps more goals in the draft than one sprint takes and chooses at the opening; what stays out waits in the next draft with its links, so the queue keeps showing that work as needed next sprint.
- Leaving a goal out is not carry-over (kb/ADR-0014, Consequences): it never belonged to an opened sprint, and a missed active goal still gets a verdict and is written again on purpose (R2).
- A goal belongs to one sprint at a time (kb/ADR-0014 R3); its sprint is the one field of a goal an opening may change.
- The opening's `opened` activity entry names the goals left out, which views may count.
- An initiative's horizon and the goals do not set each other: a goal may link to an initiative on any horizon or to none (kb/ADR-0014 R3), and nothing here ties a goal to a horizon. Views may compare the two, such as a Now initiative with no goal this sprint or next, as display only.
- Negative: goal writes can no longer trust the unlocked read of a goal's sprint (F3); each must compare it with the goal's sprint under the lock before applying the sprint's state (R2).
- Negative: a goal left out at every opening rolls forward indefinitely; nothing expires it, so the draft can fill with stale goals someone must remove by hand.
- Negative: a goal's own history does not show that it moved; only the opening's entry does.

## Assumptions and invalidation triggers
- *Assumes the person opening is the one who planned, so a single choice at the opening is enough.* Trigger: the team asks to see or agree the choice before the opening ⇒ revisit R1 with a stored selection.
- *Assumes goals left out are few and short-lived.* Trigger: at an opening, the draft holds a goal created more than three sprint lengths earlier ⇒ revisit R1 with an expiry.
- *Assumes one team per instance runs goal cycles* (as kb/ADR-0014). Trigger: a second team does ⇒ supersede.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0014 (sprints and goals), kb/ADR-0016 (sprint settings), kb/ADR-0018 (readiness), kb/ADR-0025 (promises classified against sprints).
