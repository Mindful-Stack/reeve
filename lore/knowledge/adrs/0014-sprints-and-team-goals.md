---
id: kb/ADR-0014
title: "ADR-0014: Sprints are time boxes holding team goals that link to work, never buckets of stories"
description: "A sprint is a time box holding team goals linked to any item; stories carry no sprint field and goals never touch delivery or readiness, because a bucket of stories brings carry-over and goals written to fit tickets, and a goal gate would be a third delivery rule."
tags: [adr, reeve, planning, sprints, goals]
status: accepted
date: 2026-11-03
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: one-way
blast_radius: shared
sensitivity: [security]
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve-protocol/**", "reeve:crates/reeve-server/**", "reeve:crates/reeve/**", "reeve:web/**", "lorekeeper:references/reeve-cli.md", "lorekeeper:skills/**"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0002, kb/ADR-0003]
related: [kb/ADR-0008, kb/ADR-0009, kb/ADR-0010, kb/ADR-0013]
implements: []
rfc:
---

# ADR-0014: Sprints are time boxes holding team goals that link to work, never buckets of stories

## Status
Proposed 2026-11-03.

The words sprint, goal, confidence, verdict and upkeep are the names under trial. The settled words are recorded as a
later observation here and on kb/ADR-0002 (whose R6 enumerates the bound vocabulary); the rules hold whatever the
words.

Accepted 2026-11-04 by Daniel Thyselius.

## Context
The parent design lists "Goals and metrics as entities" and "Sprints" among its non-goals and leaves open whether
sprints return. The team has since chosen two-week goal cycles: a planning Tuesday closes the previous goals with a
verdict and sets new ones, and a check Tuesday records confidence mid-cycle. For about ten cycles the team has
recorded met or missed by hand, outside the planner. Agents write most of the code and new stories appear throughout
a cycle; review and decision time, not story count, limits throughput (internal planning-model research of 2026-11-03, finding 6).

Work outside any goal (bugs, maintenance, support, operations) is a steady share of every cycle. Forcing it through
goals risks goals written to make room for tickets.

Stories carry a rank in one global todo column, a kind of story, bug or chore, and a readiness answer computed on
read. Delivery has exactly two rules (kb/ADR-0002 R4), and only interactive tokens may override readiness
(kb/ADR-0003 R2, kb/ADR-0008 R3). kb/ADR-0003's invalidation trigger counts readiness overrides from the activity
log.

## Facts relied on
- F1 The parent design lists goals and sprints as non-goals — source: the parent design of 2026-10-25 (internal, not published), §Non-goals, §Open questions.
- F2 Delivery has exactly two rules — source: kb/ADR-0002 R4; `crates/reeve-planning/src/domain/delivery.rs` `validate_move`.
- F3 Required decisions and comments take numeric ids, not keys from the item sequence — source: `crates/reeve-planning/migrations/1001_planning.sql`.
- F4 Readiness is never stored and an override is logged as `override` — source: kb/ADR-0003 R1, R2; `crates/reeve-planning/src/handler/stories.rs` `move_story`.

## Considered options
- **A sprint as a bucket of stories** (a sprint field on stories, carry-over at close): familiar from Jira, and it records what the team meant to do.
- **A standing goal for work outside the goals**: reuses one mechanism for everything, with no lane to build.
- **Goals with their own start and end dates and no sprint entity**: one table fewer; but no shared window for upkeep and no single close.
- **Sprints generated from a configured cadence**, with goals attached to whichever sprint covers a date: nothing to open by hand.
- **A start that requires a goal or a reason** (a gate): every started story is classified.
- **A frozen summary written at close**: past sprints never change when the hierarchy changes later.
- **A time box that holds goals linked to work, never touching delivery, with history computed on read** (chosen): goals stay outcomes, nothing carries over, work outside the goals is visible as upkeep, and one reader serves live and past sprints.

## Decision
We decided that the planner models a sprint as a time box holding team-level goals that link to work, with no sprint
field on stories and goals that never touch delivery or readiness, because a bucket of stories brings back carry-over
and invites goals written to describe tickets (A1), and a goal gate would be a third delivery rule (F2).

- **R1** A sprint MUST be a planning entity whose window runs from its opening to its closing; at most one sprint MAY be active at a time; stories MUST NOT carry a sprint field.
- **R2** A story's work MUST count in the one sprint whose window contains its completion; a story started in a sprint and not completed by its close MUST be shown there as in flight at close, never as done work; every sprint view MUST state this rule where it shows counts.
- **R3** A goal MUST belong to one sprint and MAY link to any initiative, epic or story by key; a goal MUST NOT be a parent of an item, and no item's progress or readiness MAY depend on goals; per-goal counts MAY be computed.
- **R4** Delivery validation and readiness MUST NOT depend on sprints, goals or goal links; goal actions MUST NOT change delivery or readiness inputs and MUST be logged under their own actions, never `override`.
- **R5** For sprint and goal actions, opening or closing a sprint, editing its dates, creating, editing or removing a goal, and recording confidence or a verdict MUST require an interactive token; automation tokens and the system MAY only link and unlink work and suggest a confidence, which a person records or discards.
- **R6** Planning MUST NOT store a sprint summary, a story's sprint membership or an upkeep classification; sprint and goal views, closed sprints included, MUST be derived on read; the counts at close MUST live only in the sprint's `closed` activity entry, and a closed sprint's view MUST show them, labelled as at close, beside the derived counts whenever the two differ; a closed sprint's goals, goal links, confidence and verdicts MUST NOT change.

## Consequences
- The parent design's non-goals "Goals" and "Sprints" are lifted (metrics stay a non-goal); the parent design is revised to point here.
- Goal verdicts, confidence and cycle history move into the planner, replacing the hand-kept record.
- Nothing carries over at close; a missed goal worth pursuing is written again in the next sprint, on purpose.
- Sprints and goals take numeric ids, as required decisions do (F3), so item keys and branch names stay items only (kb/ADR-0002 R5).
- Goal actions logged as `override` would inflate the readiness-override count that kb/ADR-0003's trigger reads, which R4 prevents.
- Accepted enumerations grow, each recorded there as a later observation: kb/ADR-0010 R3's planning tables (sprints, goals, goal links), kb/ADR-0002 R6's vocabulary (the settled words), kb/ADR-0009 R2's configuration (sprint length, upkeep reference line), kb/ADR-0008 R3's interactive-only actions (R5's).
- "What did we do this cycle?" has one answer: what was completed inside its window, with the rest shown as in flight at close. A story started in one sprint and completed in the next counts only in the next.
- The `closed` activity entry carries the counts computed at close, in the append-only log owned by the activity context; a closed sprint's view reads it through the activity API to show "at close" beside "now", so a retro view never changes silently.
- Agents can draft a confidence from linked progress ("3 of 7 done, 2 in review"); the decision stays with a person, so review time is spent accepting a draft rather than writing one.
- Negative: because counts are derived on read, moving an item to another parent or changing a story's timestamps later can change a past sprint's "now" counts; the "at close" counts, goals, links, confidence and verdicts are fixed.
- Negative: a story's sprint is never stored, so "which sprint did this story belong to" is answered by its timestamps and links, and cannot be reconstructed differently later.

## Assumptions and invalidation triggers
- *A1 Assumes a goal written after the work was chosen, or a compulsory classification, yields goals and reasons that describe tickets rather than outcomes* (the same research, findings 5 and 8). Trigger: two consecutive retros find goals written to fit tickets despite this model ⇒ revisit R3 and R4.
- *Assumes one team per instance runs goal cycles.* Trigger: a second team adopts goal cycles on this instance ⇒ revisit R1 (sprints and goals would need a team scope).
- *Assumes past counts rarely drift.* Trigger: at a close, either of the two previous closed sprints shows "at close" and "now" counts that differ ⇒ revisit R6 with a frozen summary.
- *Assumes the team keeps goal cycles.* Trigger: two consecutive cycles pass with no sprint opened ⇒ supersede.

## Later observations
<!-- append-only, accepted records only; newest last -->
- 2026-11-04 (public copy, one-time edit): F1's source, the context, A1 and See also named internal planning documents. The cited material is not in this repository; the sources now describe it in words or point to the public reeve-runner repository that holds it. No fact, rule or decision changed; the decision holds.
- 2026-11-09 (kb/ADR-0015, kb/ADR-0016): kb/ADR-0008 R3 is now kb/ADR-0015 R7; kb/ADR-0009's rules are now kb/ADR-0016 R1–R3, and the allowlist and admins kb/ADR-0015 R2. No trigger fired; the decision holds.
- 2026-11-10 (https://github.com/Mindful-Stack/reeve/pull/15): F2's source moved: the delivery rule is now `Story::move_to` in `crates/reeve-planning/src/domain/items.rs` (`domain/delivery.rs` and `validate_move` are gone), and F4's `move_story` is now in `crates/reeve-planning/src/handler/moves.rs`. Moving a story still reads no sprint, goal or link; the isolation lint now refuses `ports.sprints` and `active_goals_of_stories` in `handler/moves.rs`. No trigger fired; the decision holds.
- 2026-11-12 (kb/ADR-0025 R5, R8): a sprint's planned `ends_on` also classifies promises (this sprint, next sprint, later) and bounds a sprint's derived promise list. No sprint reference or classification is stored, so R1 holds, and moving a story still reads no promise. No trigger fired; the decision holds.
- 2026-11-12 (https://github.com/Mindful-Stack/reeve/pull/35, kb/ADR-0026): the planner keeps one draft sprint, the next sprint, with no window, created by the first goal added or by each opening; kb/ADR-0026 decides it and lets the opening leave chosen draft goals out, moving them with their links to the next draft. R3's "a goal MUST belong to one sprint" holds as one sprint at a time. "Nothing carries over at close" still holds: leaving a goal out happens at an opening, from a draft, and an active or closed sprint's goals never move. No trigger fired; the decision holds.

## See also
- kb/ADR-0002, kb/ADR-0003, kb/ADR-0008, kb/ADR-0009, kb/ADR-0010, kb/ADR-0013.
- The sprints and goals design and the planning-model research, both of 2026-11-03 (internal, not published).
