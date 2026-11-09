---
id: kb/ADR-0017
title: "ADR-0017: Urgency is a one-cycle, display-only mark on one item, spread along its lineage on read"
description: "A person can mark any initiative, epic or story urgent with an optional reason; the mark lasts one cycle unless renewed, its ancestors and descendants are urgent while it is active, computed on read, and urgency never touches delivery, readiness, progress, goals, the webhook or the importer, because a temporary focus must not become a horizon or a planning state."
tags: [adr, reeve, planning, urgency]
status: proposed
date: 2026-11-09
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: two-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve-protocol/**", "reeve:crates/reeve-server/**", "reeve:web/**"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0002, kb/ADR-0003, kb/ADR-0010]
related: [kb/ADR-0014, kb/ADR-0015, kb/ADR-0016]
implements: []
rfc:
---

# ADR-0017: Urgency is a one-cycle, display-only mark on one item, spread along its lineage on read

## Status
Proposed 2026-11-09.

## Context
Work comes up that needs the team's full focus ahead of everything in "Now". The planning page groups initiatives
into the configured horizon buckets (kb/ADR-0016 R2), a shared timeline that changes a few times a year; a temporary
horizon would take a deploy to add and another to remove, and its initiatives would lose their real horizon. Urgent
work is not always an initiative: a story can be the urgent thing, and its epic and initiative are then where the
team looks. Delivery has exactly two rules (kb/ADR-0002 R4), readiness and progress are computed on read
(kb/ADR-0003 R1), and only stories have a done state. The team plans in two-week cycles (kb/ADR-0014), and its
recorded failure is under-planning: an override that is free and never ends would be a back door around it.
An urgent mark is not a refinement state (kb/ADR-0002 R3): it says nothing about how ready an item is, only where
attention goes for now.

## Facts relied on
- F1 Only stories have a delivery state and a resolution; initiatives and epics have neither — source: `crates/reeve-planning/migrations/1001_planning.sql`; kb/ADR-0002 R4.
- F2 The discovery mark is a logged mark of time and person stored on the item — source: kb/ADR-0002 R3; `crates/reeve-planning/src/handler/initiatives.rs` `mark_discovery`.
- F3 Horizon buckets and the sprint length are instance configuration — source: kb/ADR-0016 R2.

## Considered options
- **A configured "War room" horizon** with a highlighted column: no new data, but two deploys per war room, and an urgent initiative loses its real horizon.
- **Lanes edited in the UI**: temporary without a deploy, but buckets move into the database, a rename rewrites stored horizons, and kb/ADR-0016 R2 is replaced for a need that is about focus, not the timeline.
- **A war room as a goal** (kb/ADR-0014): goals already link any item and are interactive-only, but a goal belongs to a sprint, is written at planning, and is an outcome with a check; urgency arises between plannings and says what comes first, not what the cycle will achieve.
- **A war-room entity with its own start, reason and close**: one record per event, but a second lifecycle to manage for what is, in practice, one or two items at a time.
- **A mark stored on every affected item**: simple reads, but propagated copies go stale on a reparent, a completion or a clear.
- **A mark that ends when its item is done**: needs a done state for epics and initiatives that the model does not have (F1), and revives stale marks when work reopens.
- **A mark on the marked item only, active for one cycle, lineage derived on read, display only** (chosen): one write to start, renew or end it, nothing stale, every war room reviewed at least once a cycle, and nothing in delivery depends on it.

## Decision
We decided that urgency is a mark a person sets, with an optional reason, on one initiative, epic or story; it lasts
one cycle unless renewed, its ancestors and descendants are urgent while it is active, computed on read, and it is
used for display only, because a temporary focus must not become a horizon or a planning state (F3), a stored
propagation would go stale, and an override with no end would bypass cycle planning.

- **R1** The urgent mark MUST be stored only on the marked item, as a timestamp, a person and an optional reason; no derived urgency MAY be stored.
- **R2** A mark MUST be active only until its timestamp plus the configured sprint length; marking again MUST replace the mark; no done state for epics or initiatives MAY be introduced for it.
- **R3** An item MUST be shown as urgent exactly when an active mark sits on it, on an ancestor or on a descendant, except a done story; a mark on a done story MUST NOT make any item urgent; urgency MUST NOT spread to siblings, and derived urgency MUST NOT spread further.
- **R4** Marking and clearing MUST require an interactive token and MUST be logged under their own actions, never `override`.
- **R5** Marking and clearing MUST NOT take or change the item's version; delivery validation, readiness, progress, goals and sprint views MUST NOT depend on urgency; the GitHub webhook and the importer MUST NOT read or write it.

## Consequences
- A war room starts with one mark per item; after a cycle its marks stop making anything urgent but stay listed as expired until a person clears or re-marks them, which is the review step. Clearing or renewing can be done for all listed marks at once.
- A marked story makes its epic and initiative visible as urgent; an urgent item names the marks that make it so.
- Every view that shows items reads urgency facts in the same snapshot as the items.
- Accepted enumerations grow, each recorded there as a later observation: kb/ADR-0002 R6's vocabulary ("urgent"), kb/ADR-0015 R7's interactive-only actions (R4's).
- Negative: two marked initiatives make most of their stories urgent on the board; the chip says why on each.
- Negative: agents cannot raise urgency; an incident an agent detects needs a person to mark it.
- Negative: a war room that spans a cycle is not yet visible in the sprint view or the retro.

## Assumptions and invalidation triggers
- *Assumes urgent work is rare and short-lived.* Trigger: more than three active marks on two consecutive check or planning Tuesdays ⇒ revisit with a cap or a review step.
- *Assumes one cycle is the right lifetime.* Trigger: most marks are renewed at least once over two months ⇒ revisit R2.
- *Assumes a person should decide what is urgent.* Trigger: the team asks for automated escalation (an incident hook, a CI failure) ⇒ revisit R4.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0002, kb/ADR-0014.
