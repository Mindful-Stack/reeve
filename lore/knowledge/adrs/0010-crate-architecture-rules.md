---
id: kb/ADR-0010
title: "ADR-0010: The planner keeps one crate per bounded context, starting with planning, identity and activity, with consumer-defined ports, private adapters behind factories, no cross-context foreign keys and handler-owned time"
description: "The planner carries Reeve's architecture rules forward as a recorded decision with exactly three bounded contexts, planning, identity and activity, one crate each: ports defined by the consuming crate, adapters private behind factories, a CI isolation lint, no foreign keys across contexts and the handler owning the clock, because these rules exist so that parallel agents can work the codebase without colliding and more contexts than that were found to be ceremony."
tags: [adr, reeve, architecture, crates, ports]
status: accepted
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/**", "reeve:Cargo.toml", "reeve:scripts/check-context-isolation.sh"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0006]
related: [kb/ADR-0007]
implements: []
rfc:
---

# ADR-0010: The planner keeps one crate per bounded context, starting with planning, identity and activity, with consumer-defined ports, private adapters behind factories, no cross-context foreign keys and handler-owned time

## Status
Proposed 2026-10-25.
Accepted 2026-10-26 by Daniel Thyselius.

## Context
Reeve's domain-driven refactor was motivated by a concrete failure: a single large handlers file meant two agents working unrelated contexts collided on the same file. The resulting rules are documented as standards prose in the knowledge base but never recorded as a decision, so a successor could drift from them without anyone noticing. An earlier draft of the planner proposed seven crates, with separate decision and discussion contexts; review found that required decisions and comments are planning objects and that separate contexts for them added boundaries to police without a second team or a second deployable to justify them.

## Facts relied on
- F1 The refactor's stated motivation was enabling parallel agent work on unrelated contexts — source: reeve-runner's `docs/superpowers/specs/2026-05-18-domain-driven-refactor-design.md` §Context.
- F2 The five rules and their enforcement by a grep lint are documented — source: `lore/knowledge/general/architecture.md`, `scripts/check-context-isolation.sh`.

## Considered options
- **One crate, modules per context** — less ceremony; nothing stops a cross-context import.
- **Three contexts, one crate each, consumer-defined ports, private adapters, isolation lint, no cross-context foreign keys, handler-owned time** — the compiler and CI enforce the boundaries that matter and no others.
- **A crate per aggregate, with event sourcing and projections** — full audit history; demonstrated to over-reach at this scale.

## Decision
We decided to carry Reeve's architecture rules into the planner with exactly three bounded contexts, planning, identity and activity, because the rules exist to let several agents work the codebase in parallel, and three contexts are the fewest that keep identity and the audit log out of the planning crate.

- **R1** Bounded contexts MUST each be one crate and the server MUST be the only composition root; planning, identity and activity are the starting set, and adding a context MUST be recorded as a later observation on this record rather than a supersession.
- **R2** A cross-context port MUST be defined in the consuming crate and implemented in the supplying crate; adapters MUST be crate-private and reachable only through factories returning trait objects.
- **R3** Tables MUST be owned by one context; foreign keys MUST NOT cross contexts; references MUST be by id or key; initiatives, epics, stories, required decisions and comments MUST all live in the planning context, and the key sequence MUST be owned there.
- **R4** Handlers MUST own time by passing a single timestamp into repositories; repositories MUST NOT read the clock.
- **R5** CI MUST run the context-isolation lint; the activity log MUST be an append-only table written through a port that takes the caller's transaction so that a mutation and its entry commit together, events MUST be published only after commit, and the planner MUST NOT adopt event sourcing or asynchronous projections.

## Consequences
- The planning crate is the large one; modules inside it are organised by aggregate so that agents still work separate files.
- Rollups such as initiative progress are computed on read, which costs a query but cannot drift.
- Within the planning context, foreign keys between its tables are permitted; the rule concerns context boundaries.

## Assumptions and invalidation triggers
- *Assumes the dependency rules, not the crate count, are what protect correctness.* Trigger: a boundary rule itself needs to change ⇒ supersede; a new context is an observation.

## Later observations
<!-- append-only, accepted records only; newest last -->
- 2026-11-04 (kb/ADR-0014): R3's list of planning tables grows by `sprints`, `goals` and `goal_links`, owned by the planning context with foreign keys inside it only; goal links refer to items by key. Planning never reads the activity table: a sprint's counts at close live in its `closed` activity entry and the CLI and web compose them. No trigger fired; the decision holds.
- 2026-11-04 (public copy, one-time edit): F1's source now names the public reeve-runner repository that holds the cited design; F3 and a sentence of the context, about a third-party prototype the design research studied, are left out of this copy (the numbering is kept). The cited material is not in this repository; the sources now describe it in words or point to the public reeve-runner repository that holds it. No rule or decision changed; the decision holds.
- 2026-11-09 (kb/ADR-0015): R3's identity tables grow by `allowed_entries` and `admin_grants`, owned by the identity context with no foreign key leaving it. No trigger fired; the decision holds.
- 2026-11-10 (https://github.com/Mindful-Stack/reeve/pull/15): planning's single `PlanningRepo` port became ten per-aggregate ports (`KeyRepo`, `InitiativeRepo`, `EpicRepo`, `StoryRepo`, `DecisionRepo`, `CommentRepo`, `PullRequestRepo`, `SprintRepo`, `UrgencyRepo`, `PlanningReads`), all implemented by one crate-private adapter and returned together in `PlanningPorts` by `make_planning_ports`. R2 holds: the adapter is reachable only through the factory, and mutations still take the caller's transaction (R5). F2's lint gained call-site rules. No trigger fired; the decision holds.

## See also
- kb/ADR-0006, kb/ADR-0007.
