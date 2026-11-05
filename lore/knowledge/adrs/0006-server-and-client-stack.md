---
id: kb/ADR-0006
title: "ADR-0006: The planner is a Rust server on axum and sqlx serving a statically built SvelteKit client from the same binary"
description: "The planner server is Rust on axum and sqlx and serves a SvelteKit client built as static assets from the same binary, because Rust's compiler gives agents the fastest wrong-shape feedback, the TUI and runner are already Rust, and one container is the simplest thing to host."
tags: [adr, reeve, stack, rust, sveltekit]
status: accepted
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/**", "reeve:web/**"]
supersedes: []
superseded_by:
depends_on: []
related: [kb/ADR-0007, kb/ADR-0010]
implements: []
rfc:
---

# ADR-0006: The planner is a Rust server on axum and sqlx serving a statically built SvelteKit client from the same binary

## Status
Proposed 2026-10-25.
Accepted 2026-10-26 by Daniel Thyselius.

## Context
The planner will be maintained by one person who writes Rust and will be worked on heavily by coding agents. The team's product backends are C# with SvelteKit clients and one Hono service, and a non-technical part of the team needs a browser interface. The existing Reeve code, including a ratatui TUI with embedded sessions, is Rust. The portal runs its client as a separate server-rendered container, which suits a public product more than an internal tool.

## Facts relied on
- F1 The team's product backends are C# and its main product's client is SvelteKit — source: those repositories' solution and package files (internal).
- F2 Reeve's CLI and TUI depend only on the protocol crate — source: `lore/knowledge/general/architecture.md`.
- F3 Reeve's 2026-04-29 design rejected a TypeScript-plus-Rust hybrid for a single-developer local tool — source: reeve-runner's `docs/superpowers/specs/2026-04-29-reeve-design.md` §Context.

## Considered options
- **C# server matching the portal backend** — the team's backend language and tooling.
- **TypeScript server sharing types with the Lorekeeper plugin** — one language for plugin, CLI and server.
- **Rust server on axum and sqlx with a static SvelteKit client served by the binary** — strictest compiler, reuse of Reeve's crates and TUI, one deployable.

## Decision
We decided to build the planner server in Rust on axum and sqlx and to serve a SvelteKit client compiled to static assets from the same binary, because compile-time strictness is the fastest feedback loop for agents working the codebase and one binary is the simplest thing to host and reason about.

- **R1** The server MUST be a single Rust binary that serves both the API and the client assets; production MUST NOT run a Node process.
- **R2** The client MUST be built with a static adapter and MUST consume the API only through types generated from the server's OpenAPI document.
- **R3** The API's wire types MUST live in the protocol crate and the OpenAPI document MUST be generated from them, so a wire change is a diff in one place.

## Consequences
- The web client is TypeScript, so the repo has two languages regardless; the boundary is the generated types.
- No server-side rendering; first paint waits for the API, which is acceptable for an authenticated internal tool.
- Team members who write C# cannot easily contribute to the server; this was accepted knowingly.

## Assumptions and invalidation triggers
- *Assumes one maintainer for the server is acceptable.* Trigger: the planner becomes something others must maintain and no Rust capacity exists ⇒ supersede.
- *Assumes a static client suffices.* Trigger: a need for server rendering or edge auth ⇒ supersede R1.

## Later observations
<!-- append-only, accepted records only; newest last -->
- 2026-11-04 (public copy, one-time edit): F1's source was a set of local paths naming the team's products, and F3's now names the public reeve-runner repository that holds the cited design. The cited material is not in this repository; the sources now describe it in words or point to the public reeve-runner repository that holds it. No fact, rule or decision changed; the decision holds.

## See also
- kb/ADR-0007, kb/ADR-0010.
