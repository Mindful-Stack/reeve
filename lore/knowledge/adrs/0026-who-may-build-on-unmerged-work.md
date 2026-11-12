---
id: kb/ADR-0026
title: "ADR-0026: Unmerged work blocks other people, never its own owner"
description: "We decided that a blocking story still in todo blocks every dependent, that one in progress or in review blocks only dependents with a different owner, and that a merged one blocks nobody, because a person must be able to build on their own unmerged work while nobody else starts on it unknowingly."
tags: [adr, reeve, readiness, gate, domain, delivery]
status: proposed
date: 2026-11-12
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve/**", "reeve:web/src/**", "lorekeeper:skills/**", "lorekeeper:references/**"]
supersedes: [kb/ADR-0018]
superseded_by:
depends_on: [kb/ADR-0002, kb/ADR-0004]
related: [kb/ADR-0014, kb/ADR-0027]
implements: []
rfc:
---

# ADR-0026: Unmerged work blocks other people, never its own owner

## Status
Proposed 2026-11-12.

## Context
This record replaces kb/ADR-0018 and changes one clause of it, R5's "unfinished MUST mean delivery not done". R1 to R4 carry over unchanged.

What changed: work is now routinely built by one person or agent as a run of dependent stories, often across stacked pull requests that build on each other before they merge, and often several dependent stories inside one pull request. In one initiative of about thirty stories and forty blocks edges, three stacked pull requests implemented thirteen stories in two days. A dependent could not start until its blocker was done, and done came only after the blocker's pull request merged and was checked; agents may not override readiness (R2), so the stories stayed in todo while the work happened.

The opposite reading, "a started blocker is finished", would let anyone start a dependent and build on code that has not merged, without its author knowing. The person who owns the blocker is the one who can build on it safely. Stories carry an owner, and moving an unowned story into in progress already makes the mover its owner, after readiness has been judged.

## Facts relied on
- F1 Readiness lists every open blocker as a BlockedBy reason, read as a set of keys with no owner or delivery state, and its input carries no owner of the story itself — source: `crates/reeve-planning/src/domain/readiness.rs` (`ReadinessInput`, `open_blockers`, `Reason::BlockedBy`), `crates/reeve-planning/src/repo/reads.rs`.
- F2 Moving an unowned story into in progress makes the mover its owner, after readiness has been computed — source: `crates/reeve-planning/src/domain/items.rs` (`Story::move_to`), `crates/reeve-planning/src/handler/moves.rs`.
- F3 Only an interactive user may override readiness, with a written reason — source: kb/ADR-0018 R2.
- F4 Pull-request status never moves a story — source: kb/ADR-0002 R4.

## Considered options
- **Keep "unfinished means not done", and a person overrides each dependent started early** — no change to the planner, and every start on unmerged work is logged with a reason.
- **A started blocker is finished, for everyone** — the simplest change; nothing needs an override.
- **A per-story "stacked on" label set by the person stacking** — explicit about intent.
- **Owner-scoped: a blocker in todo blocks everyone; in progress or in review it blocks only dependents with a different owner; merged (testing or done) it blocks nobody** — the owner keeps working without overrides, and everyone else waits for the merge.

## Decision
We decided to compute a story's readiness on every read as kb/ADR-0018 did, except that a blocking story counts as unfinished while it is in todo, and while it is in progress or in review for every dependent whose owner differs from the blocker's, because a person must be able to build on their own unmerged work while nobody else starts on it unknowingly, and merged work is safe for anyone to build on.

- **R1** The planner MUST NOT persist a readiness value; every readiness answer MUST be derived from current facts and MUST include the failing reasons; no named maturity level other than ready MAY be exposed on stories.
- **R2** An interactive user MAY move a non-ready story from todo or done into an active state only by supplying a written reason, which MUST be written to the activity log; automation tokens and agent principals MUST NOT override.
- **R3** A required decision MUST be satisfied only when its last check found the referenced record accepted on the repository's default branch; no record, proposed with PR open, not found, and any other status MUST each yield a distinct reason; a closed required decision MUST be ignored, and a satisfied one MUST stay under check so a later supersession unsatisfies it.
- **R4** An item of kind story, bug or chore without an epic MUST be untriaged and MUST be not ready with reason untriaged, first among its reasons; the untriaged filter MUST list exactly the items without an epic that are not done; untriaged MUST mean "needs placement" and MUST NOT be read as a record that a person triaged the item; the planner MUST NOT store a separate triaged flag.
- **R5** A blocking story MUST count as unfinished when its delivery is todo; when it is in progress or in review it MUST count as unfinished unless the dependent and the blocker have the same non-empty owner; when it is in testing or done it MUST NOT count as unfinished. Every blocks edge MUST be judged by this rule on its own, cycles included, and cycles MUST NOT be prevented.
- **R6** When a story with no owner is moved into an active state, the gate MUST judge it as owned by the mover, and the move MUST make the mover its owner.

## Consequences
- Readiness depends on owner as well as delivery: changing the owner of a story or of its blocker can change whether it is ready. An unowned dependent still reads as blocked in lists while its blocker's owner can start it, because a list has no mover.
- Someone who builds on a colleague's unmerged work on purpose still needs an override with a reason, which records the agreement. Pairing on one run of stories works when the stories share an owner; handing a run over means reassigning the stories.
- A merged blocker frees its dependents before its acceptance criteria are verified; a dependent started then may need rework if the blocker's check fails.
- The readiness input gains the story's own owner and each open blocker's delivery and owner; the blocking rule stays one pure function in the domain, and the BlockedBy reason gains the blocker's delivery and owner so a reader sees whether asking the owner, or waiting for the merge, unblocks it. Every place that renders the reason (CLI, TUI, web) changes, as does Lorekeeper's CLI reference, in the same release.
- A cycle of stories owned by one person, all started, no longer blocks itself; a cycle with any story in todo still does.

## Assumptions and invalidation triggers
- *Assumes one owner per run of dependent stories is the common case.* Trigger: in a quarter, more than ten overrides start a story whose only failing reason was a blocker in progress or in review ⇒ review the decision for shared ownership.
- *Assumes merged work is safe to build on.* Trigger: in a quarter, more than two blockers return from testing to in progress after a dependent started ⇒ review whether testing still blocks others.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0018 (replaced), kb/ADR-0003 (replaced by kb/ADR-0018), kb/ADR-0002 (R4 delivery rules, unchanged), kb/ADR-0004, kb/ADR-0014, kb/ADR-0027 (when stories move).
