---
id: kb/ADR-0002
title: "ADR-0002: Work is initiative, epic and story under one key sequence, with a discovery mark above and five delivery states under two rules below"
description: "Reeve models work as initiative → epic → story with kinds story, bug and chore, one key sequence per instance, a discovery-complete mark on initiatives and epics, and five freely movable delivery states constrained only by a readiness rule on entering active work and a resolution rule on entering done, because three levels with one artifact each is the simplest model that forces product planning upfront while keeping design just-in-time."
tags: [adr, reeve, domain, hierarchy, vocabulary, keys, delivery]
status: accepted
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve-protocol/**", "lorekeeper:skills/**", "reeve-runner:**"]
supersedes: []
superseded_by:
depends_on: []
related: [kb/ADR-0003, kb/ADR-0004]
implements: []
rfc:
---

# ADR-0002: Work is initiative, epic and story under one key sequence, with a discovery mark above and five delivery states under two rules below

## Status
Proposed 2026-10-25.
Accepted 2026-10-26 by Daniel Thyselius.

## Context
The team plans in Jira Product Discovery ideas and delivers in a Scrum project with epics and stories across five columns. Developers plan implementation with Lorekeeper, where one brainstorming session yields one spec sized for one plan. Specs have grown fat because they carry product decisions that nothing above them captured. The team is small, several developers are juniors, and under-planning is the observed failure. Reeve's single column enum was honest only while the agent was the sole actor; an explicit edge list for delivery kept missing cases in review; Jira keys tell a reader the project but not the level; and keys outlive tools, appearing in branches, commits and records.

## Facts relied on
- F1 The brainstorming skill decomposes anything too large for one plan into sub-projects with one spec each — source: Lorekeeper `skills/brainstorming/SKILL.md`.
- F2 Linear, Jira and Plane each stop at three hierarchy levels plus sub-items — source: internal design research of 2026-10-25, §4 (not published).
- F3 The Jira delivery board on 2026-10-25 used To Do, In Progress, Ready for review, In testing, Done under product epics — source: the team's Jira delivery board (internal).

## Considered options
- **Four levels, initiative, epic, feature as spec, story as PR slice** — no re-cut of existing epics; four levels to teach and route.
- **Three levels, epic is the spec, stories are plan slices** — the crispest gate; demands full upfront design the team does not sustain.
- **Three levels, epic holds decisions and breakdown, story is the unit one person or one agent session finishes; discovery as a mark; delivery as free movement under two rules** — simplest rule set and no change of grain for Reeve or Lorekeeper.

## Decision
We decided to model work as initiative, epic and story with kinds story, bug and chore, keyed from one sequence per instance, with a discovery-complete mark on initiatives and epics and five freely movable delivery states under two rules, because that forces product decisions to the epic while leaving implementation design just-in-time, and because every explicit edge list reviewed was found incomplete.

- **R1** A story MUST be sized so that one person or one agent session can finish it; it MAY link to any number of pull requests; the planner MUST NOT introduce a fourth hierarchy level.
- **R2** Stories MUST carry a kind of story, bug or chore; a story of kind story without an epic MUST be treated as untriaged.
- **R3** Discovery on initiatives and epics MUST be a mark holding timestamp and person; clearing it MUST be logged; no other refinement state MAY be stored.
- **R4** Delivery MUST be todo, in progress, in review, testing, done, freely movable, with exactly two rules: entering an active state from todo or from done MUST satisfy readiness or an interactive override with a written reason, and entering done MUST carry a resolution, completed only from an active state and dropped from any state; pull-request status MUST NOT move a story.
- **R5** Keys MUST be `<PREFIX>-<n>` from one monotonic sequence per instance, MUST NOT change when an item moves level or parent, and MUST be used in artifact paths and branch names; imported Jira keys MUST be kept as external references.
- **R6** The vocabulary initiative, epic, story and decision MUST be used across API, database, CLI and UI.

## Consequences
- Stories can be larger than Jira user stories were; a small story needs criteria but no spec.
- A story can be dragged backwards or forwards without ceremony; the two rules are the whole policy.
- A key alone does not reveal the level; the UI and CLI show the kind beside it.
- Existing fat specs are not migrated; their product content belongs to epics going forward.

## Assumptions and invalidation triggers
- *Assumes one agent session per story is the right execution grain.* Trigger: a sustained need to run several sessions on one story ⇒ supersede with an epic-level spec that generates stories.
- *Assumes five delivery states and two resolutions cover the team's flow.* Trigger: the team needs a sixth state or a third resolution for a quarter ⇒ supersede R4.

## Later observations
<!-- append-only, accepted records only; newest last -->
- 2026-11-04 (kb/ADR-0014): R6's bound vocabulary gains sprint, goal, confidence, verdict and upkeep as the trial names; the settled words are recorded here when the team's naming trial ends. Sprints and goals take numeric ids, not keys from R5's sequence, and a goal is never a parent of an item, so R1 holds. No trigger fired; the decision holds.
- 2026-11-04 (public copy, one-time edit): F2's source named an internal research document, and F3's an internal Jira board and a product name; F4, about a third-party prototype the research studied, is left out of this copy (the numbering is kept). The cited material is not in this repository; the sources now describe it in words or point to the public reeve-runner repository that holds it. No rule or decision changed; the decision holds.
- 2026-11-09 (kb/ADR-0017): R6's bound vocabulary grows by "urgent" (a mark on any item, kb/ADR-0017); "war room" names the planning page's list of urgent marks (active and expired) and is not bound vocabulary. No trigger fired; the decision holds.
- 2026-11-09 (kb/ADR-0017): "war room" also names the API resource `/war-room` and the CLI's `reeve war-room`; it stays unbound vocabulary. No trigger fired; the decision holds.
- 2026-11-10 (kb/ADR-0018): R2 is the story-kind case of kb/ADR-0018 R4, which governs: every item without an epic is untriaged, so the placement tree's "bug under no epic" end state is gone. No trigger fired; the decision holds.
- 2026-11-10 (kb/ADR-0018): R3's discovery mark is set on standing loose-work epics without product decisions behind it; for their items the ancestor discovery check is a formality (kb/ADR-0018, Consequences). No trigger fired; the decision holds.
- 2026-11-10 (kb/ADR-0022, kb/ADR-0023): R6's bound vocabulary grows by "attachment" (an image owned by one initiative, epic or story, or uploaded and not yet saved with any); it is not an item and adds no hierarchy level, so R1 holds. No trigger fired; the decision holds.
- 2026-11-12 (kb/ADR-0025): R6's bound vocabulary grows by "promise", "overdue" and "late" (a dated commitment on a story or epic and two derived states of it); "effective date", "holder" and "due" name its parts and are not bound vocabulary. No trigger fired; the decision holds.

## See also
- kb/ADR-0003, kb/ADR-0004.
