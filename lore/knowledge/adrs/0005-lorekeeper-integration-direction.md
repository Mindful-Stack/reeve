---
id: kb/ADR-0005
title: "ADR-0005: Lorekeeper optionally drives Reeve through the reeve CLI when it is present; Reeve never depends on Lorekeeper"
description: "Lorekeeper's planning skills are an optional part of Lorekeeper that activates when a reeve on the path reports itself as the planner and the user is logged in, implemented as new planning-initiative and refining-epic skills plus one step each in brainstorming and recording-decisions, and Reeve neither links nor requires Lorekeeper, because the planner is the product and the plugin is one client of it; this inverts the earlier rule that Lorekeeper must not know Reeve exists."
tags: [adr, reeve, lorekeeper, integration, cli]
status: accepted
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["lorekeeper:skills/**", "reeve:crates/reeve/**"]
supersedes: []
superseded_by:
depends_on: []
related: [kb/ADR-0004, kb/ADR-0002]
implements: []
rfc:
---

# ADR-0005: Lorekeeper optionally drives Reeve through the reeve CLI when it is present; Reeve never depends on Lorekeeper

## Status
Proposed 2026-10-25.
Accepted 2026-10-26 by Daniel Thyselius.

## Context
Reeve's original design hard-required Lorekeeper and stated that Lorekeeper must not know Reeve exists, because Reeve was an execution loop meant to sit beside an existing planning tool. The successor is the planning tool, and using it should be an optional part of Lorekeeper that gets more value out of the knowledge base. Initiative-first work means Lorekeeper must create epics, stories and required decisions, link drafted records to them, and read context from the planner. Lorekeeper already drives external tools by shelling out to CLIs such as gh. The brainstorming skill has a hard gate and a checklist that ends in a spec and a hand-off to writing-plans, which planning at the initiative or epic level must not produce. The current Reeve installs a `reeve` binary with different commands. An earlier draft added a manifest section validated across three tools; review found an environment-free detection on the path and the user's login to be enough.

## Facts relied on
- F1 The 2026-04-29 design deferred Jira intake and stated Lorekeeper stays a pure plugin unaware of Reeve — source: reeve-runner's `docs/superpowers/specs/2026-04-29-reeve-design.md` §Deferred.
- F2 The brainstorming skill's hard gate and checklist force a spec and a writing-plans hand-off — source: Lorekeeper `skills/brainstorming/SKILL.md` on origin/main.
- F3 The current Reeve workspace builds a `reeve` CLI binary — source: `crates/reeve/Cargo.toml`.

## Considered options
- **Lorekeeper unaware of Reeve; humans copy keys between tools** — no cross-tool coupling.
- **Reeve exposes an MCP server that Lorekeeper connects to** — richer tool surface for agents.
- **Reeve exposes a CLI with JSON output; Lorekeeper's planning skills use it when it is present** — one surface for humans, agents and skills, degrading gracefully when absent.

## Decision
We decided to make Lorekeeper's planning skills an optional capability that uses the `reeve` CLI when a `reeve` on the path reports itself as the planner and the user is logged in, implemented as new planning-initiative and refining-epic skills, one link step in recording-decisions and one context-reading step in brainstorming, and to keep Reeve free of any Lorekeeper dependency, because the planner is the product every client talks to and a CLI is the surface the plugin already knows how to drive.

- **R1** Skills MUST detect the planner by `reeve --version` reporting `reeve-planner` and by a successful login; absent either, every skill MUST behave as before; no manifest change MAY be required.
- **R2** The runner's CLI MUST be renamed so it never answers to `reeve`.
- **R3** Every `reeve` command used by a skill MUST support `--json` and MUST exit with a distinct status for success, refusal and failure; create commands MUST accept a caller-supplied idempotency key and MUST NOT deduplicate on titles.
- **R4** Initiative and epic planning MUST be separate skills that produce no spec; the brainstorming skill's hard gate and checklist MUST NOT be weakened; recording-decisions MUST link a drafted record to its required decision when the planner is present.
- **R5** Reeve MUST NOT require Lorekeeper for any planning function; an MCP server, if ever added, MUST wrap the same commands.

## Consequences
- Two new skills and two new steps are more test surface for Lorekeeper, and nothing changes in witan-household.
- Skills run under an automation token, so every write is attributed to a person via automation and cannot override the readiness gate.

## Assumptions and invalidation triggers
- *Assumes a shell CLI is an adequate agent surface.* Trigger: a skill needs streaming or long-lived state the CLI cannot provide ⇒ add MCP under R5.

## Later observations
<!-- append-only, accepted records only; newest last -->
- 2026-11-04 (public copy, one-time edit): F1's source now names the public reeve-runner repository that holds the cited design. The cited material is not in this repository; the sources now describe it in words or point to the public reeve-runner repository that holds it. No fact, rule or decision changed; the decision holds.
- 2026-11-14 (kb/ADR-0029, proposed): an MCP server is being added under R5 because part of the team works only in claude.ai or ChatGPT, clients that cannot run the `reeve` CLI (source: Daniel Thyselius, 2026-11-14). This is not the trigger named under *Assumptions and invalidation triggers* ("a skill needs streaming or long-lived state the CLI cannot provide"): the CLI is still an adequate surface for skills. R5 holds, since the MCP tools wrap the same commands (kb/ADR-0029 R1). No trigger fired; the decision holds.

## See also
- kb/ADR-0004, kb/ADR-0002.
