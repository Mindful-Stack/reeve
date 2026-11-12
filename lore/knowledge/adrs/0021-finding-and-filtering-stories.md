---
id: kb/ADR-0021
title: "ADR-0021: The story list is the one filtered list of stories, filtered on the server with an additive vocabulary"
description: "We decided that GET /stories and `reeve story ls` are the one filtered list of stories, filtered on the server with an additive parameter vocabulary that includes text, because people looked in two places for one job and Lorekeeper's skills already depend on that list."
tags: [adr, reeve, planning, search, api]
status: accepted
date: 2026-11-10
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: one-way
blast_radius: shared
sensitivity: [contract]
scope: ["reeve:crates/reeve-planning/src/handler/stories.rs", "reeve:crates/reeve-planning/src/repo/stories.rs", "reeve:crates/reeve-planning/src/ports/mod.rs", "reeve:crates/reeve-planning/src/ports/stories.rs", "reeve:crates/reeve-protocol/src/enums.rs", "reeve:crates/reeve-server/src/routes/stories.rs", "reeve:crates/reeve/src/commands/story.rs", "reeve:web/src/routes/backlog/**", "reeve:web/src/routes/+layout.svelte", "reeve:web/src/lib/backlog.ts", "reeve:web/src/lib/components/BacklogFilters.svelte"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0001, kb/ADR-0005, kb/ADR-0014, kb/ADR-0017, kb/ADR-0018]
related: [kb/ADR-0002, kb/ADR-0006, kb/ADR-0007, kb/ADR-0010, kb/ADR-0011]
implements: []
rfc:
---

# ADR-0021: The story list is the one filtered list of stories, filtered on the server with an additive vocabulary

## Status
Proposed 2026-11-10.
Accepted 2026-11-11 by Daniel Thyselius.

## Context
People find work in two places. The backlog page loads every todo story from `GET /stories?delivery=todo` and
filters it in the browser by mine, ready and kind only. The search page sends `GET /search?q=`, which matches text
across initiatives, epics and stories, returns at most fifty hits with no state, owner or readiness, and takes no
filter. Most searches look for an open story by words in its title or description, or jump to a known key or Jira
reference; epics, initiatives and done work are looked up rarely. So a person who wants "open bugs in this epic
that mention invoices" has no single place to ask.

The story list is already a public contract. Lorekeeper's skills call `reeve story ls --json` and depend on its
flags and field shapes (kb/ADR-0005 R3); executors may reach the same list through the public API (kb/ADR-0001 R2).
It already filters on the server by epic, owner, delivery, untriaged, label, ready, done window and goal. Readiness,
goal membership and urgency are computed on read in one snapshot. Urgency is computed in one place, `urgency_in`,
and the mark columns are read only through the urgency port (architecture rule 5). The server ignores query
parameters it does not know, so a client newer than the server gets an unfiltered list without an error.

## Facts relied on
- F1 The backlog page filters the todo list in the browser by mine, ready and kind — source: `web/src/routes/backlog/+page.svelte`.
- F2 Search takes only `q`, matches with ILIKE over a union of all three levels, matches a key only exactly, and caps at fifty hits — source: `crates/reeve-planning/src/handler/search.rs`, `crates/reeve-planning/src/repo/reads.rs` `search_items`.
- F3 The story list already filters by epic, owner, delivery, untriaged, label, ready, done (`recent`, `all`) and goal, and applies ready and goal after the read — source: `crates/reeve-server/src/routes/stories.rs` `StoryQuery`; `crates/reeve-protocol/src/enums.rs` `DoneFilter`; `crates/reeve-planning/src/handler/stories.rs` `list_stories_in`.
- F4 Every story the list returns already carries its computed urgency — source: `crates/reeve-planning/src/handler/dto.rs` `urgent_fields`; `crates/reeve-planning/src/handler/urgent.rs` `urgency_in`.
- F5 Lorekeeper's CLI reference lists `reeve story ls` and its flags, and the skill smoke checks them — source: Lorekeeper `references/reeve-cli.md`; `scripts/skill-smoke.sh`.
- F6 Unknown query parameters are ignored, not refused — source: `crates/reeve-server/src/routes/stories.rs` `StoryQuery` (no `deny_unknown_fields`); `crates/reeve-server/src/extract.rs` `ApiQuery`.
- F7 The skill smoke fails when a flag in `reeve <command> --help` is missing from Lorekeeper's reference, and runs against Lorekeeper `main` unless `LOREKEEPER_REF` names an open pull request's branch — source: `.github/workflows/skill-smoke.yml`; Lorekeeper `scripts/__tests__/reeve-reference.test.js`.

## Considered options
- **A new list over all three levels** (`/items`) that replaces both the backlog's list and search: one filter type for everything, but a mixed row has no rank, readiness or state for epics and initiatives, grouping and ranking become awkward, and the list Lorekeeper uses would be replaced in a three-repository migration.
- **Two views, each with more filters**: the smallest change, but people still choose between two places for one job and the two filter sets drift.
- **The story list as the one filtered list, extended with text, kind, initiative, urgent, sort and a third done value** (chosen): it reuses the list that skills already depend on, keeps rank and readiness on every row, and search remains for the rare look across all levels.
- **A separate `open` flag for "not done"**: reads well, but it is a second parameter on the axis `done` already owns, so `open=true&done=recent` and `open=true&delivery=done` become inert combinations.
- **A multi-valued `delivery`** (`delivery=todo,in_progress`): general, but every client must spell out "not done" as four states, and a new state would silently change what those lists mean.

Where `urgent` is evaluated is a narrower choice inside the chosen option. A SQL predicate in the story query would suit pagination, but the story port would read mark columns, and a second definition of kb/ADR-0017 R3 could drift from `urgency_in`; it is evaluated after the read instead (R4).

## Decision
We decided that `GET /stories` and `reeve story ls` are the one filtered list of stories, filtered on the server
with an additive vocabulary that includes free text, because people looked in two places for one job (F1, F2) and
Lorekeeper's skills already depend on this list (F5).

- **R1** The backlog view MUST get its rows by sending filter parameters to the story list; it MUST NOT filter a full story list in the client.
- **R2** Besides its existing parameters, the story list MUST accept `q` (text, 1 to 200 characters), `kind` (`story`, `bug`, `chore`), `initiative` (a key or external reference; one that names nothing yields an empty list), `urgent` (boolean), `sort` (`rank`, the default, or `relevance`) and the `done` value `none` (no done stories); `sort=relevance` MUST be refused without `q`; under `relevance` a story whose key or external reference equals `q` MUST sort before the others. `reeve story ls` MUST expose them as `--text`, `--kind`, `--initiative`, `--urgent`, `--sort` and `--open` (for `done=none`).
- **R3** An existing parameter's name, value set or meaning MUST NOT change, and no parameter or flag named in R2 MAY be removed or renamed, without a record that replaces this one.
- **R4** `ready`, `goal` and `urgent` MUST be evaluated on values computed in the same snapshot as the rows they filter (kb/ADR-0018 R1, kb/ADR-0014 R6, kb/ADR-0017 R1); `urgent` MUST select exactly the rows the response shows as urgent, as computed by `urgency_in`.
- **R5** `untriaged` MUST keep the meaning of kb/ADR-0018 R4.
- **R6** A server that understands a parameter MUST be deployed before a Lorekeeper release (the marketplace listing installs pull) lists the flag that sends it (F6); Lorekeeper's pull request may merge earlier, since this repository's skill smoke needs the reference to list every flag the CLI has (F7).

## Consequences
- The header search box jumps to an item when it is given a key and otherwise opens the backlog filtered by the text; `/search` stays as the view across all levels and done work.
- Which fields `q` matches, how it matches them (ILIKE today) and the order below the exact-match tier are design, not contract; full-text ranking can replace them without a new record.
- "Open" is not wire vocabulary: on the wire it is `done=none`, in the CLI `--open`, in the web the "Open" state.
- Every new flag must be listed in Lorekeeper's CLI reference in a pull request that merges before this repository's change (F7); its release, the version tag and the listing bump, waits for the deployment (R6).
- Negative: the list has no pagination; adding it would need `ready`, `goal` and `urgent` moved into one read on `PlanningReads`, or pages would come back short.
- Negative: R3 freezes the vocabulary's names, so a poorly named parameter stays until a replacing record.
- Negative: an older server silently ignores the new parameters (F6); R6 orders the rollout but does not make skew fail loudly.
- Negative: the board still filters in the client and does not share the backlog's filter bar.

## Assumptions and invalidation triggers
- *Assumes the open backlog stays small enough to return whole (a few hundred stories).* Trigger: a typical backlog response exceeds 1,000 stories or 500 ms ⇒ revisit with pagination; `ready`, `goal` and `urgent` move into one `PlanningReads` read.
- *Assumes people search mostly for open stories and keys.* Trigger: people report, twice in a cycle, that they could not find an epic, an initiative or a done story from the header box ⇒ revisit whether the list should span epics and initiatives.
- *Assumes ILIKE is precise enough.* Trigger: people repeatedly fail to find stories they know exist because of wording ⇒ revisit text matching (still within kb/ADR-0007).

## Later observations
<!-- append-only, accepted records only; newest last -->
- 2026-11-12 (kb/ADR-0025): R2's story-list parameters grow by `promised` (`reeve story ls --promised`; additive, R3). R4 applies to it: `promised` is evaluated in the same snapshot as the rows, on the promise computed for each row, and `any` is frozen as "open or overdue". No trigger fired; the decision holds.

## See also
- kb/ADR-0001, kb/ADR-0005, kb/ADR-0017, kb/ADR-0018.
