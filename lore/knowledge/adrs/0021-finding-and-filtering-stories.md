---
id: kb/ADR-0021
title: "ADR-0021: The story list is the one filtered list of stories, filtered on the server with an additive vocabulary"
description: "We decided that GET /stories and `reeve story ls` are the one filtered list of stories, filtered on the server with an additive parameter vocabulary that includes text, because people looked in two places for one job and agents and executors already depend on that list."
tags: [adr, reeve, planning, search, api]
status: proposed
date: 2026-11-10
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: one-way
blast_radius: shared
sensitivity: [contract]
scope: ["reeve:crates/reeve-planning/src/handler/stories.rs", "reeve:crates/reeve-planning/src/repo/stories.rs", "reeve:crates/reeve-planning/src/ports/**", "reeve:crates/reeve-server/src/routes/stories.rs", "reeve:crates/reeve/src/commands/story.rs", "reeve:web/src/routes/backlog/**", "reeve:web/src/lib/backlog.ts"]
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

## Context
People find work in two places. The backlog page loads every todo story from `GET /stories?delivery=todo` and
filters it in the browser by mine, ready and kind only. The search page sends `GET /search?q=`, which matches text
across initiatives, epics and stories, returns at most fifty hits with no state, owner or readiness, and takes no
filter. Most searches look for an open story by words in its title or description, or jump to a known key or Jira
reference; epics, initiatives and done work are looked up rarely. So a person who wants "open bugs in this epic
that mention invoices" has no single place to ask.

The story list is already a public contract. Lorekeeper's skills call `reeve story ls --json` and depend on its
flags and field shapes (kb/ADR-0005 R3), and executors find work through the same public API (kb/ADR-0001 R2). It
already filters on the server by epic, owner, delivery, untriaged, label, ready, done window and goal. Readiness,
goal membership and urgency are computed on read in one snapshot and are never stored (kb/ADR-0018 R1, kb/ADR-0014
R6, kb/ADR-0017 R1). Urgency is computed in one place, `urgency_in`, and the mark columns are read only through the
urgency port (architecture rule 5).

## Facts relied on
- F1 The backlog page filters the todo list in the browser by mine, ready and kind — source: `web/src/routes/backlog/+page.svelte`.
- F2 Search takes only `q`, matches with ILIKE over a union of all three levels and caps at fifty hits — source: `crates/reeve-planning/src/handler/search.rs`, `crates/reeve-planning/src/repo/reads.rs` `search_items`.
- F3 The story list already filters by epic, owner, delivery, untriaged, label, ready, done and goal, and applies ready and goal after the read — source: `crates/reeve-server/src/routes/stories.rs` `StoryQuery`; `crates/reeve-planning/src/handler/stories.rs` `list_stories_in`.
- F4 Every story the list returns already carries its computed urgency — source: `crates/reeve-planning/src/handler/urgent.rs` `urgency_in`.
- F5 Lorekeeper's CLI reference lists `reeve story ls` and its flags, and the skill smoke checks them — source: Lorekeeper `references/reeve-cli.md`; `scripts/skill-smoke.sh`.

## Considered options
- **A new list over all three levels** (`/items`) that replaces both the backlog's list and search: one filter type for everything, but a mixed row has no rank, readiness or state for epics and initiatives, grouping and ranking become awkward, and the endpoints that Lorekeeper and executors use would be replaced in a three-repository migration.
- **Two views, each with more filters**: the smallest change, but people still choose between two places for one job and the two filter sets drift.
- **The story list as the one filtered list, extended with text, kind, initiative, open, urgent and sort** (chosen): it reuses the list that agents already depend on, keeps rank and readiness on every row, and search remains for the rare look across all levels.
- **Urgency as a SQL predicate in the story query**: the filter would run in the database and suit pagination, but the story port would read mark columns, and a second definition of R3 of kb/ADR-0017 could drift from `urgency_in`.

## Decision
We decided that `GET /stories` and `reeve story ls` are the one filtered list of stories, filtered on the server
with an additive vocabulary that includes free text, because people looked in two places for one job (F1, F2) and
agents and executors already depend on this list (F5).

- **R1** The backlog view MUST get its rows by sending filter parameters to the story list; it MUST NOT filter a full story list in the client.
- **R2** The story list MUST accept, besides its existing parameters, `q` (1 to 200 characters, matched case-insensitively against key, title, description and external reference; a key-shaped value also matches that key), `kind`, `initiative` (a key or reference, matched through the story's epic; one that names nothing yields an empty list), `open` (excludes done; combines with `delivery`), `urgent` and `sort` (`rank` by default, or `relevance`, which requires `q`, ranks a key match, then an exact external reference, then a title match, then the rest, and breaks ties by rank); `reeve story ls` MUST expose each as a flag.
- **R3** An existing parameter's meaning MUST NOT change and no parameter MAY be removed or renamed without a record that replaces this one.
- **R4** `ready`, `goal` and `urgent` MUST be evaluated on values computed in the same snapshot as the rows they filter and MUST NOT be stored; `urgent` MUST select exactly the rows the response shows as urgent, as computed by `urgency_in`.
- **R5** `untriaged` MUST keep the meaning of kb/ADR-0018 R4.

## Consequences
- The header search box jumps to an item when it is given a key and otherwise opens the backlog filtered by the text; `/search` stays as the view across all levels and done work.
- Text matching is ILIKE; moving to full-text ranking is a change behind the port and needs no new record.
- Every new flag must be listed in Lorekeeper's CLI reference, with a version and listing bump, in the same release.
- Negative: the list has no pagination; adding it would need `ready`, `goal` and `urgent` moved into one read on `PlanningReads`, or pages would come back short.
- Negative: R3 freezes the vocabulary's names, so a poorly named parameter stays until a replacing record.
- Negative: the board still filters in the client and does not yet share the backlog's filter bar.

## Assumptions and invalidation triggers
- *Assumes the open backlog stays small enough to return whole (a few hundred stories).* Trigger: a typical backlog response exceeds 1,000 stories or 500 ms ⇒ revisit with pagination and supersede R4's placement.
- *Assumes people search mostly for open stories and keys.* Trigger: the "search all items" link is used for more than a quarter of searches over a month ⇒ revisit whether the list should span epics and initiatives.
- *Assumes ILIKE is precise enough.* Trigger: people repeatedly fail to find stories they know exist because of wording ⇒ revisit text matching (still within kb/ADR-0007).

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0001, kb/ADR-0005, kb/ADR-0017, kb/ADR-0018.
