---
id: kb/ADR-0027
title: "ADR-0027: Unmerged work blocks other people, never its own owner"
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
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve-protocol/**", "reeve:crates/reeve/**", "reeve:web/src/**", "lorekeeper:skills/**", "lorekeeper:references/**"]
supersedes: [kb/ADR-0018]
superseded_by:
depends_on: [kb/ADR-0002, kb/ADR-0004, kb/ADR-0028]
related: [kb/ADR-0014]
implements: []
rfc:
---

# ADR-0027: Unmerged work blocks other people, never its own owner

## Status
Proposed 2026-11-12.

## Context
This record replaces kb/ADR-0018. It changes R5 ("unfinished MUST mean delivery not done") and adds R6; R1 to R4 and kb/ADR-0018's assumptions carry over unchanged.

What changed: work is now routinely built by one person or agent as a run of dependent stories, often several inside one pull request and often across stacked pull requests that build on each other before they merge. A dependent cannot start until its blocker is done, and done comes only after the blocker has merged and been checked; agents may not override readiness (R2). In the team's own instance, a run of dependent stories was implemented across stacked pull requests while every story stayed in todo.

The opposite reading, "a started blocker is finished", would let anyone start a dependent and build on code that has not merged, without its author knowing. The person who owns the blocker is the one who can build on it safely. Stories carry an owner, and moving an unowned story into in progress already makes the mover its owner, after readiness has been judged.

## Facts relied on
- F1 Readiness lists every open blocker as a BlockedBy reason, read as a set of keys with no owner or delivery state, and its input carries no owner of the story itself — source: `crates/reeve-planning/src/domain/readiness.rs` (`ReadinessInput`, `open_blockers`), `crates/reeve-protocol` (`Reason::BlockedBy`), `crates/reeve-planning/src/repo/reads.rs`.
- F2 Moving an unowned story into in progress makes the mover its owner, after readiness has been computed — source: `crates/reeve-planning/src/domain/items.rs` (`Story::move_to`), `crates/reeve-planning/src/handler/moves.rs`.
- F3 An automation token acts as the email of the person who created it, so a person and their agent are the same owner — source: `crates/reeve-server/src/auth/mod.rs`.

## Considered options
- **Owner-scoped: a blocker in todo blocks everyone; in progress or in review it blocks only dependents with a different owner; merged (testing or done) it blocks nobody** — the owner keeps working without overrides, and everyone else waits for the merge.
- **Keep "unfinished means not done", and a person overrides each dependent started early** — no change to the planner, and every start on unmerged work is logged with a reason.
- **A started blocker is finished, for everyone** — the simplest change; nothing needs an override.
- **A per-story "stacked on" label set by the person stacking** — explicit about intent.

## Decision
We decided to compute a story's readiness on every read as: it has an epic, whatever its kind; every initiative and epic above it has its discovery mark and no open unsatisfied required decision; every open required decision on the story is satisfied; it has at least one acceptance criterion; and no story that blocks it is unfinished, where a blocker in todo is unfinished for everyone, a blocker in progress or in review is unfinished only for a dependent with a different owner, and a blocker in testing or done is finished; the computation returns reasons on failure, gates entering any active state from todo or done, allows an override only from an interactive token with a written reason, and allows none for automation or agents, because a person must be able to build on their own unmerged work while nobody else starts on it unknowingly, and merged work is safe for anyone to build on.

- **R1** The planner MUST NOT persist a readiness value; every readiness answer MUST be derived from current facts and MUST include the failing reasons; no named maturity level other than ready MAY be exposed on stories.
- **R2** An interactive user MAY move a non-ready story from todo or done into an active state only by supplying a written reason, which MUST be written to the activity log; automation tokens and agent principals MUST NOT override.
- **R3** A required decision MUST be satisfied only when its last check found the referenced record accepted on the repository's default branch; no record, proposed with PR open, not found, and any other status MUST each yield a distinct reason; a closed required decision MUST be ignored, and a satisfied one MUST stay under check so a later supersession unsatisfies it.
- **R4** An item of kind story, bug or chore without an epic MUST be untriaged and MUST be not ready with reason untriaged, first among its reasons; the untriaged filter MUST list exactly the items without an epic that are not done; untriaged MUST mean "needs placement" and MUST NOT be read as a record that a person triaged the item; the planner MUST NOT store a separate triaged flag.
- **R5** A blocking story MUST count as unfinished when its delivery is todo; when it is in progress or in review it MUST count as unfinished unless the dependent and the blocker have the same non-empty owner; when it is in testing or done it MUST NOT count as unfinished. Every blocks edge MUST be judged by this rule on its own, cycles included, and cycles MUST NOT be prevented.
- **R6** When a story with no owner is moved from todo or done into an active state, the gate MUST judge it as owned by the mover, and the move MUST make the mover its owner.

## Consequences
- Readiness depends on owner as well as delivery: changing the owner of a story or of its blocker can change whether it is ready. An unowned dependent reads as blocked in lists while its blocker's owner can still start it, because a list has no mover.
- Owner is a gate input anyone can edit: setting oneself as owner of a colleague's dependent passes the gate without an override reason. The activity log still records the owner change.
- Someone who builds on a colleague's unmerged work on purpose needs an override with a reason, which records the agreement. Handing a run of stories over means reassigning them.
- A merged blocker frees its dependents before its acceptance criteria are verified; a dependent started then may need rework if the blocker's check fails.
- Moving an unowned story straight into in review or testing now assigns an owner too.
- The BlockedBy reason names the blocker's delivery and owner, so a reader sees whether asking the owner or waiting for the merge unblocks it; every client that renders the reason changes with it.
- A cycle of stories owned by one person, all started, no longer blocks itself; a cycle with any story in todo still does.

## Assumptions and invalidation triggers
- *Assumes one owner per run of dependent stories is the common case.* Trigger: in a quarter, more than ten overrides start a story whose only failing reason was a blocker in progress or in review ⇒ review the decision for shared ownership.
- *Assumes merged work is safe to build on.* Trigger: in a quarter, more than two blockers return from testing to in progress after a dependent started ⇒ review whether testing still blocks others.
- *Assumes the team keeps placing every bug and chore under an epic.* Trigger: the team asks for an epic-less bug or chore to be ready, or any standing loose-work epic holds more than 30 open items or gets a required decision ⇒ split that epic, and if it recurs, supersede R4 with an explicit triaged mark.
- *Assumes triage happens often enough that reports do not age unplaced.* Trigger: in any two-week sprint, more than five untriaged items are older than seven days at the sprint's close ⇒ name a triage owner or schedule, and revisit whether the reporter should place.
- *Assumes the record check runs often enough that results are current.* Trigger: staleness between acceptance and check is routinely noticed ⇒ shorten the schedule and surface check age.
- *Assumes the loose gate is enough to change behaviour.* Trigger: override rate stays above half of starts after one quarter ⇒ supersede with the strict gate.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0018 (replaced), kb/ADR-0002 (R4 delivery rules, unchanged), kb/ADR-0004, kb/ADR-0014, kb/ADR-0028 (when stories move; testing means merged).
