---
id: kb/ADR-0007
title: "ADR-0007: PostgreSQL is the planner's datastore, accessed through sqlx"
description: "The planner stores everything in PostgreSQL through sqlx, replacing Reeve's SQLite-over-rusqlite, because the planner is a hosted multi-user service whose operations already run PostgreSQL and whose readiness queries join across several tables."
tags: [adr, reeve, storage, postgres]
status: accepted
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/*/src/repo.rs", "reeve:migrations/**"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0006]
related: [kb/ADR-0006]
implements: []
rfc:
---

# ADR-0007: PostgreSQL is the planner's datastore, accessed through sqlx

## Status
Proposed 2026-10-25.
Accepted 2026-10-26 by Daniel Thyselius.

## Context
Reeve stores its board in a single SQLite file read through rusqlite inside spawn_blocking, a pattern documented in its own knowledge base, which fits one daemon on one laptop. The planner is a hosted service with several concurrent users and, later, executors claiming work under optimistic concurrency. The team's services already run PostgreSQL in Docker with backups scripted on the team's hosts. SQLite with a streaming backup would serve six users, so the choice is not forced by load.

## Facts relied on
- F1 Reeve uses rusqlite with the bundled feature and an async-over-sync pattern — source: `crates/reeved/Cargo.toml`, `lore/knowledge/frameworks/rust/`.
- F2 The team's main product runs with a local PostgreSQL compose file in production — source: that product's production compose file (internal).
- F3 Readiness is computed on read across decisions, dependencies and criteria — source: kb/ADR-0003.

## Considered options
- **SQLite with a streaming backup** — one file, no second container, adequate for the team's size.
- **PostgreSQL through sqlx** — matches existing operations, native row versioning and concurrent writers, compile-time checked queries.

## Decision
We decided to use PostgreSQL accessed through sqlx as the planner's only datastore, because the planner is a hosted multi-user service, we already operate PostgreSQL, and sqlx's compile-time query checking fits the compiler-first stack.

- **R1** Every context's repository adapter MUST target PostgreSQL through sqlx; no second storage engine MAY be introduced for planning data.
- **R2** Schema changes MUST be sqlx migrations committed with the code that needs them.
- **R3** Each bounded context MUST own its tables; cross-context references MUST be by key or id without foreign keys, per kb/ADR-0010.

## Consequences
- Tests need a real PostgreSQL per run, which is slower than in-memory SQLite.
- The async-over-sync rusqlite pattern and its knowledge node are retired for the planner.
- A second container to operate, already the norm for the team's services.

## Assumptions and invalidation triggers
- *Assumes the team's hosts keep running PostgreSQL in Docker.* Trigger: a move to a managed database or a different host model ⇒ note, R1 still holds.

## Later observations
<!-- append-only, accepted records only; newest last -->
- 2026-11-04 (public copy, one-time edit): F2's source was a local path, and the context and first assumption named the hosting provider. The cited material is not in this repository; the sources now describe it in words or point to the public reeve-runner repository that holds it. No fact, rule or decision changed; the decision holds.
- 2026-11-10 (https://github.com/Mindful-Stack/reeve/pull/15): the planning context's adapter moved from `crates/reeve-planning/src/repo.rs` to `crates/reeve-planning/src/repo/*.rs`, which the `scope` pattern `reeve:crates/*/src/repo.rs` does not match, so a change to the planning adapter is not bound to this record by `scope`. `scope` is locked on a classified record (the ADR validator refuses the change), so the pattern `reeve:crates/*/src/repo/**` waits for the record that next replaces or restates this one. It is still PostgreSQL through sqlx (R1). No trigger fired; the decision holds.

## See also
- kb/ADR-0006, kb/ADR-0010.
