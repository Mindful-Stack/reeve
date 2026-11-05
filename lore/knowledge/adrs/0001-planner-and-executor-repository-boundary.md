---
id: kb/ADR-0001
title: "ADR-0001: The planner and any executor live in separate repositories, coupled only by the planner's HTTP API and CLI"
description: "Reeve the planner is its own repository and every executor, including the existing Reeve runner, is a separate repository that talks to it only through the public HTTP API and CLI, because a pluggable executor contract must not privilege one implementation."
tags: [adr, reeve, repository, boundary, executor]
status: accepted
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:**", "reeve-runner:**"]
supersedes: []
superseded_by:
depends_on: []
related: []
implements: []
rfc:
---

# ADR-0001: The planner and any executor live in separate repositories, coupled only by the planner's HTTP API and CLI

## Status
Proposed 2026-10-25.
Accepted 2026-10-26 by Daniel Thyselius.

## Context
Reeve today is one Rust workspace in one repository: a daemon that owns a kanban board and also provisions per-card clones, devcontainers and agent sessions. The successor splits those roles. A planning server with a web client and a CLI replaces Jira for the team; execution of stories by agents is a later milestone, and the executor behind it may be the existing runner code, Claude Code's own background agents, or a hosted sandbox service. A single Cargo workspace would build separate binaries for server and runner, so deployment artefacts are not the issue. The issue is whether the planner's API is designed as a public contract or as an internal one that one co-located executor can lean on.

## Facts relied on
- F1 The current repository's six crates mix board logic (reeve-card) with provisioning (reeve-workspace) under one composition root — source: `Cargo.toml`, `lore/knowledge/general/architecture.md`.
- F2 Published orchestrators converge on a tracker-as-state-machine plus a separate supervisor that polls and claims work — source: internal design research of 2026-10-25, §5 (not published).
- F3 Lorekeeper's stack skill already handles coordinated changes across two repositories — source: Lorekeeper `skills/stack`.

## Considered options
- **One Cargo workspace, two binaries** — atomic changes across the server-runner boundary with one CI run.
- **Separate repositories in a Witan household** — the API must be good enough for a third-party executor because no executor can link a planner crate.
- **Planner only, executors always external services** — least code, but gives up the self-hosted sandbox model the runner already provides.

## Decision
We decided to keep the planner in a fresh repository named `reeve` and to rename the existing repository to `reeve-runner`, because an executor contract that is pluggable in theory must be pluggable in practice, and co-locating one executor with the planner would let it bypass the contract.

- **R1** The runner repository MUST NOT depend on any planner crate; it MAY depend on the planner's published OpenAPI document and CLI.
- **R2** Every capability an executor needs MUST be reachable through the public HTTP API; the server MUST NOT expose executor-only private endpoints.
- **R3** Both repositories MUST be members of one Witan household so cross-repo changes use the household's stack convention.

## Consequences
- A change that touches both the API and the runner needs two PRs and a stacked merge, which is slower than one workspace.
- The planner's API is forced to be complete enough for an executor the team did not write, which is the point.
- The existing repository keeps its history under a new name; GitHub redirects the old name.

## Assumptions and invalidation triggers
- *Assumes more than one executor implementation is plausible.* Trigger: two years pass with the Reeve runner as the only executor and the cross-repo tax dominates ⇒ supersede with a merge.
- *Assumes GitHub repository renames redirect.* Trigger: external links break after the rename ⇒ note and republish links.

## Later observations
<!-- append-only, accepted records only; newest last -->
- 2026-11-04 (public copy, one-time edit): F2's source named an internal research document, and F3's a local path. The cited material is not in this repository; the sources now describe it in words or point to the public reeve-runner repository that holds it. No fact, rule or decision changed; the decision holds.

## See also
- The milestone-two interface sketch in the planner design for the executor contract.
