---
id: kb/ADR-0003
title: "ADR-0003: Readiness is computed from facts, never stored, and gates every entry into active work, with an interactive-only override"
description: "A story's readiness is computed on read from its ancestors' discovery marks and required decisions, its own required decisions, its acceptance criteria and its unfinished blockers, never stored, and it gates entering any active state from todo or done with a written override for interactive users only, because a stored flag rots and an unmeasured gate cannot be tightened honestly."
tags: [adr, reeve, readiness, gate, domain]
status: accepted
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve-planning/**", "lorekeeper:skills/**"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0002, kb/ADR-0004]
related: []
implements: []
rfc:
---

# ADR-0003: Readiness is computed from facts, never stored, and gates every entry into active work, with an interactive-only override

## Status
Proposed 2026-10-25.
Accepted 2026-10-26 by Daniel Thyselius.

## Context
The team's initiative template has a section for decisions required before building, and it stays empty, because nothing checks it. Published practice computes readiness from facts: Beads' ready query is "open with no open blockers", Shape Up's "solved" and Kubernetes' "implementable" encode decided-ness as a state. Nobody found phrases the gate as "every required decision accepted", which is the gap this record fills. Definition-of-ready practices fail when they harden into a barrier the team routes around, so a gate that is bypassed silently teaches nothing. Automation acting under a person's token must not be able to bypass the gate, or the bypass count measures nothing. Review found that a gate on entering in progress alone could be sidestepped by moving to review or testing, and that named maturity levels (decided, refined) were vocabulary nobody needed.

## Facts relied on
- F1 Readiness in Beads is a query, not a field — source: internal design research of 2026-10-25, §4 (not published).
- F2 Reeve's `needs_heavy` flag became orphaned metadata because its enforcement point was never built — source: reeve-runner's `docs/superpowers/specs/2026-05-27-review-and-pr-flow-design.md` §Future work.
- F3 Atlassian measured agent-authored-for tickets as agent-ready 83% of the time versus 6% for traditional ones — source: internal design research of 2026-10-25, §6 (not published).

## Considered options
- **Stored readiness label set by humans** — zero computation and familiar from Jira labels.
- **Computed readiness, strict gate: the whole epic refined before any story starts** — maximises upfront planning.
- **Computed readiness, loose gate on this story's facts and its ancestors, interactive override with a written reason, no automation or agent override** — enforces the gate where it is cheap and measures bypasses.

## Decision
We decided to compute a story's readiness on every read as: every initiative and epic above it has its discovery mark and no open unsatisfied required decision, a story of kind story has an epic, every open required decision on the story is satisfied, it has at least one acceptance criterion, and no story that blocks it is unfinished; the computation returns reasons on failure, gates entering any active state from todo or done, allows an override only from an interactive token with a written reason, and allows none for automation or agents, because a computed value cannot rot and a recorded override lets the gate be tightened on evidence.

- **R1** The planner MUST NOT persist a readiness value; every readiness answer MUST be derived from current facts and MUST include the failing reasons; no named maturity level other than ready MAY be exposed on stories.
- **R2** An interactive user MAY move a non-ready story from todo or done into an active state only by supplying a written reason, which MUST be written to the activity log; automation tokens and agent principals MUST NOT override.
- **R3** A required decision MUST be satisfied only when its last check found the referenced record accepted on the repository's default branch; no record, proposed with PR open, not found, and any other status MUST each yield a distinct reason; a closed required decision MUST be ignored, and a satisfied one MUST stay under check so a later supersession unsatisfies it.
- **R4** A story of kind story without an epic MUST be not ready with reason untriaged; bugs and chores MAY be ready without an epic. Unfinished MUST mean delivery not done. Cycles in blocks edges MUST NOT be prevented; they MUST leave every story in the cycle blocked with a visible reason.

## Consequences
- Immediately after Jira import most of the backlog is blocked, which is true; the importer grandfathers in-flight work and extracts criteria where Jira had them.
- Readiness queries touch ancestors, required decisions, dependencies and criteria on each read; indexes on those links are mandatory.
- Tightening to the strict gate later is a rule change plus a policy decision backed by the override filter on the activity feed, which the lead owns.

## Assumptions and invalidation triggers
- *Assumes the record check runs often enough that results are current.* Trigger: staleness between acceptance and check is routinely noticed ⇒ shorten the schedule and surface check age.
- *Assumes the loose gate is enough to change behaviour.* Trigger: override rate stays above half of starts after one quarter ⇒ supersede with the strict gate.

## Later observations
<!-- append-only, accepted records only; newest last -->
- 2026-11-04 (public copy, one-time edit): F1's and F3's sources named an internal research document, and F2's now names the public reeve-runner repository that holds the cited design. The cited material is not in this repository; the sources now describe it in words or point to the public reeve-runner repository that holds it. No fact, rule or decision changed; the decision holds.

## See also
- kb/ADR-0002, kb/ADR-0004.
