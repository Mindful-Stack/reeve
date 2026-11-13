---
id: kb/ADR-0018
title: "ADR-0018: Readiness is computed from facts, gates every entry into active work with an interactive-only override, and an item of any kind without an epic needs placement"
description: "We decided that readiness stays computed from facts and gates every entry into active work with an interactive-only override, and that an item of any kind without an epic is untriaged and not ready, because work reported from anywhere must reach triage and the team already places every bug and chore under an epic."
tags: [adr, reeve, readiness, gate, domain, triage]
status: superseded
date: 2026-11-09
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve/**", "reeve:web/src/**", "lorekeeper:skills/**", "lorekeeper:references/**"]
supersedes: [kb/ADR-0003]
superseded_by: kb/ADR-0027
depends_on: [kb/ADR-0002, kb/ADR-0004]
related: [kb/ADR-0014]
implements: []
rfc:
---

# ADR-0018: Readiness is computed from facts, gates every entry into active work with an interactive-only override, and an item of any kind without an epic needs placement

## Status
Proposed 2026-11-09.
Accepted 2026-11-10 by Daniel Thyselius.
Superseded 2026-11-12 by kb/ADR-0027.

## Context
This record replaces kb/ADR-0003 and changes one clause of it. What changed: the web client gains a Report entry point reachable from every page, through which anyone files a bug, a story or a chore without knowing which epic owns it, and a triager places it. kb/ADR-0003 made only a story of kind story without an epic untriaged; a bug or chore without an epic was legal and could be ready, so a bug filed through that entry point would never appear in the triage filter and could be started unplaced. The placement tree the triage skill applies still allowed "a bug under the owning epic or none", but in practice the team places every item: of 443 stories in the team's own instance, one, a chore, had no epic.

Everything else kb/ADR-0003 decided is carried over: the facts readiness is computed from, derivation on read, the gate on entering any active state from todo or done, the interactive-only override with a written reason, and the record check that satisfies a required decision. Placing an item under an epic also places it under that epic's discovery mark and required decisions, so a standing epic for loose work gates every item in it.

## Facts relied on
- F1 On 2026-11-09 the team's own instance listed 443 stories, of which exactly one, of kind chore, had no epic — source: `reeve story ls --json` against that instance, counted by kind and `epic == null` (the instance is not public).
- F2 Today the untriaged reason and filter test `kind == story` and `epic is none` — source: `crates/reeve-planning/src/domain/readiness.rs` `compute_readiness`, `crates/reeve-planning/src/repo.rs` `list_stories`.
- F3 The placement tree the triage skill applies allows a bug "under the owning epic or none" — source: Lorekeeper's `skills/refining-epic/SKILL.md`, step 7.
- F4 Readiness in Beads is a query, not a field — source: internal design research of 2026-10-25, §4 (not published).
- F5 Reeve's `needs_heavy` flag became orphaned metadata because its enforcement point was never built — source: reeve-runner's `docs/superpowers/specs/2026-05-27-review-and-pr-flow-design.md` §Future work.
- F6 An item's readiness includes its epic's and initiative's discovery marks and open required decisions — source: `crates/reeve-planning/src/domain/readiness.rs` `compute_readiness`, ancestors loaded in `crates/reeve-planning/src/repo.rs`.

## Considered options
- **Keep kb/ADR-0003; the Report entry point files every item as kind story** — no rule change; the reporter's choice of broken, idea or task survives only as a label until triage sets the real kind.
- **An explicit triaged mark on every item, set when a person places or accepts it** — keeps epic-less bugs and chores legal once seen, and gives a home to later accept and snooze actions and to triage-time metrics.
- **Untriaged means no epic, for every kind** — one fact decides placement, no new stored state, and it matches how the team already files work.

## Decision
We decided to compute a story's readiness on every read as: it has an epic, whatever its kind; every initiative and epic above it has its discovery mark and no open unsatisfied required decision; every open required decision on the story is satisfied; it has at least one acceptance criterion; and no story that blocks it is unfinished; the computation returns reasons on failure, gates entering any active state from todo or done, allows an override only from an interactive token with a written reason, and allows none for automation or agents, because a computed value cannot rot, a recorded override lets the gate be tightened on evidence, and work filed from anywhere must reach triage instead of being started unplaced while the team already places every bug and chore under an epic.

- **R1** The planner MUST NOT persist a readiness value; every readiness answer MUST be derived from current facts and MUST include the failing reasons; no named maturity level other than ready MAY be exposed on stories.
- **R2** An interactive user MAY move a non-ready story from todo or done into an active state only by supplying a written reason, which MUST be written to the activity log; automation tokens and agent principals MUST NOT override.
- **R3** A required decision MUST be satisfied only when its last check found the referenced record accepted on the repository's default branch; no record, proposed with PR open, not found, and any other status MUST each yield a distinct reason; a closed required decision MUST be ignored, and a satisfied one MUST stay under check so a later supersession unsatisfies it.
- **R4** An item of kind story, bug or chore without an epic MUST be untriaged and MUST be not ready with reason untriaged, first among its reasons; the untriaged filter MUST list exactly the items without an epic that are not done; untriaged MUST mean "needs placement" and MUST NOT be read as a record that a person triaged the item; the planner MUST NOT store a separate triaged flag.
- **R5** Unfinished MUST mean delivery not done. Cycles in blocks edges MUST NOT be prevented; they MUST leave every story in the cycle blocked with a visible reason.

## Consequences
- A bug or chore with no natural home needs one: each area keeps a standing epic for loose work. Such an epic is marked discovery-complete without product decisions behind it, so for its items the ancestor discovery check is a formality, and a required decision put on it gates every unrelated item under it. kb/ADR-0002's "epic holds decisions and breakdown" does not describe these epics.
- The placement tree's "bug under no epic" end state is gone; Lorekeeper's triage guidance and CLI reference change in the same release as the planner, and declining a report becomes a move to done with resolution dropped.
- Placing is the only way out of the untriaged filter short of dropping the item; triage cannot accept an item as is.
- Epic-less chores that served a goal directly stop being ready; the sprint page's ready counts and ready-only lists change accordingly.
- Readiness queries still touch ancestors, required decisions, dependencies and criteria on each read; the indexes on those links stay mandatory.

## Assumptions and invalidation triggers
- *Assumes the team keeps placing every bug and chore under an epic.* Trigger: the team asks for an epic-less bug or chore to be ready, or any standing loose-work epic holds more than 30 open items or gets a required decision ⇒ split that epic, and if it recurs, supersede R4 with an explicit triaged mark.
- *Assumes triage happens often enough that reports do not age unplaced.* Trigger: in any two-week sprint, more than five untriaged items are older than seven days at the sprint's close ⇒ name a triage owner or schedule, and revisit whether the reporter should place.
- *Assumes the record check runs often enough that results are current.* Trigger: staleness between acceptance and check is routinely noticed ⇒ shorten the schedule and surface check age.
- *Assumes the loose gate is enough to change behaviour.* Trigger: override rate stays above half of starts after one quarter ⇒ supersede with the strict gate.

## Later observations
<!-- append-only, accepted records only; newest last -->
- 2026-11-10: `list_stories` (F2's source, now `crates/reeve-planning/src/repo/stories.rs`) carries kb/ADR-0021's text, kind, epic-set and not-done predicates, and the story list gains `q`, `kind`, `initiative`, `urgent`, `sort` and `done=none`. The untriaged predicate and its test are unchanged (`untriaged_filter_lists_every_kind_without_an_epic_and_agrees_with_readiness`); R4 holds and no trigger fired.

## See also
- kb/ADR-0003 (replaced), kb/ADR-0002 (R2 is the story-kind case of R4 here; R3's discovery mark on standing epics), kb/ADR-0004, kb/ADR-0014.
