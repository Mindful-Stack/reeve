---
id: kb/ADR-0031
title: "ADR-0031: An item link is a named, informational connection between two items; a web link is a titled URL on one item"
description: "We decided that any two items can be joined by an item link with one of three fixed names, and that an item can carry web links, both informational and kept apart from story blockers, because relating work must never change what the readiness gate reads."
tags: [adr, reeve, planning, domain, cli]
status: proposed
date: 2026-11-23
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: one-way
blast_radius: shared
sensitivity: [privacy]
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve-protocol/**", "reeve:crates/reeve-server/**", "reeve:crates/reeve/**", "reeve:web/**", "reeve:scripts/check-context-isolation.sh"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0002, kb/ADR-0010, kb/ADR-0027]
related: [kb/ADR-0004, kb/ADR-0005, kb/ADR-0014, kb/ADR-0019, kb/ADR-0029]
implements: []
rfc:
---

# ADR-0031: An item link is a named, informational connection between two items; a web link is a titled URL on one item

## Status
Proposed 2026-11-23.

## Context
The planner relates items in two ways: the hierarchy (kb/ADR-0002) and the story blocker, where one story holds
another back until the blocker is far enough along (kb/ADR-0027 R5). Nothing else can be said about two items. A team
that plans in documents writes "this is step 1, step 2 comes after it", and points at pages kept elsewhere; today both
go into a brief as prose, where the other item never sees them and no command can list them. Triage records a
duplicate as a comment on the duplicate, so the original shows nothing. Goals are kept away from the gate by rule and
by a lint (kb/ADR-0014 R4), and urgency and promises follow the same pattern. The word "link" is already used three
times: a goal's link to an item, a pull-request link, and a required decision's link to its record.

## Facts relied on
- F1 The only relation between items besides the hierarchy is `story_blocks`, story to story — source: `crates/reeve-planning/migrations/1001_planning.sql`.
- F2 The gate judges each blocking edge on its own, from the blocker's delivery and both owners — source: `crates/reeve-planning/src/domain/readiness.rs` (`blocks`).
- F3 Nothing deletes an initiative, epic or story — source: `grep -rnE "DELETE FROM (stories|epics|initiatives)\b" crates` finds nothing outside tests.
- F4 A goal link points at an item of any level by a plain key column, without a foreign key — source: `crates/reeve-planning/migrations/1008_sprints.sql` (`goal_links`).
- F5 The Jira importer reads issue links, counts them by type and uses one type to find an epic's parent; the rest are dropped — source: `crates/reeve/src/jira/plan.rs` (`count_links`).
- F6 Triage marks a duplicate by posting the comment "Duplicate of <key>." — source: `web/src/lib/triage.ts` (`duplicateText`).
- F7 The isolation lint restricts what `handler/moves.rs` may reach — source: `scripts/check-context-isolation.sh`.

## Considered options
- **Blocking as one of the link names**: one mechanism and one command family, as in Jira. But a blocker is defined by what it does, so "links never gate" would carry an exception, one name would be limited to stories and carry owner rules, and the table, plan files, `blocked_by` and the skills would all migrate.
- **Link names configured per instance**: each team keeps its own words. But agents and skills could rely on no name, and removing a name leaves stored links without one.
- **Free-text names**: nothing to configure. But the other item has no wording for the reverse direction, and near-duplicate names accumulate.
- **One list whose target is an item or a URL**: one table. But a web page has no reverse side and a name such as "follows" means little for it.
- **Web links as markdown links in the brief only**: nothing to build. But no command or agent can list an item's outside pages without parsing prose, and a story description is not the place people look for them.
- **A foreign key per level, as attachments have**: the database guarantees both ends exist. But it needs six nullable columns and two checks for a guarantee F3 already gives.
- **Item links with a fixed set of names, web links as their own list, blockers untouched** (chosen): two small concepts, no change to the gate.

## Decision
We decided that any two items can be joined by an item link carrying one of three fixed names, and that an item can
carry web links, both informational and separate from story blockers, because people need to say how work relates
without that ever changing whether a story may start.

- **R1** An item link MUST join two different items of any level and carry exactly one name from: `follows` (the item comes after the other in a sequence; read from the other item as "is followed by"), `duplicates` (the item covers the same work as the other; "is duplicated by"), `relates_to` (the same from both sides). A name MUST NOT be added without a later observation on this record that names it and its reverse wording.
- **R2** No item link name MAY mean holding work back ("blocks", "depends on" or the like), and an importer MUST NOT turn another tool's blocking relation into an item link.
- **R3** Item links and web links MUST NOT be read when readiness is computed or a story is moved, and MUST NOT change progress, rank, a goal or an item's version. Blocking (kb/ADR-0027) stays the only relation the gate reads; the isolation lint and a named test MUST hold this.
- **R4** An item link is an edge of its own, owned by neither item. It MUST be stored once: adding one that exists succeeds and changes nothing, and `relates_to` between two items is one row whichever end it was added from. `duplicates` MUST NOT merge, drop or move either item.
- **R5** A web link MUST be a title and a URL on one item, in a list separate from item links. The URL scheme MUST be `http` or `https`.
- **R6** "Item link" and "web link" MUST be the words used across API, database, CLI and UI (kb/ADR-0002 R6). Bare "link" MAY be used where no goal link, pull-request link or decision's record link is in view.
- **R7** Both MUST be kept in planning-owned tables that name items by key (kb/ADR-0010 R3) and be reachable through the API and the CLI. A caller who may edit an item may add or remove its item links and web links; each change MUST be logged on every item it names, in the same transaction.

## Consequences
- A reader sees how an item relates to others, from either end.
- Two command families relate items: `reeve story block` for what gates and `reeve link` for the rest. A person who records a real dependency as `follows` gets no gate; the clients say so when a `follows` is added between two stories, and nothing more enforces it.
- With plain keys the database does not check that an end exists; the handler does, and a future way to delete an item must clean these tables.
- Web link titles and URLs can hold personal data and are shown to connected chat apps, so what an operator's privacy statement must say grows (kb/ADR-0019 R1), and so does the text a chat model reads from the planner (kb/ADR-0029).
- A team migrating from Jira loses its issue links unless it adds them itself; a later change may import the informational ones.
- Lorekeeper's CLI reference changes with the commands.

## Assumptions and invalidation triggers
- *Assumes three names are enough.* Trigger: the same missing name is asked for by two teams, or `relates_to` is more than half of all item links on an instance ⇒ add the name by observation (R1).
- *Assumes dependencies above stories need no gate.* Trigger: a team asks for an epic or initiative to hold another back ⇒ a new record extending blocking; R2 and R3 stand.
- *Assumes people tell a blocker from an item link.* Trigger: stories found started ahead of work they `follow`, where a blocker was meant ⇒ revisit how the two are presented, or supersede.
- *Assumes items are never deleted (F3).* Trigger: a delete or merge of items is introduced ⇒ revisit R7's storage.
- *Assumes outside pages are worth listing apart from prose.* Trigger: web links stay unused while briefs keep carrying markdown links ⇒ supersede R5.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0027 (the blocker and the gate)
- kb/ADR-0002 (hierarchy and bound vocabulary)
- kb/ADR-0014 (goal links, an earlier use of the word)
- kb/ADR-0029 (the read tools show both lists; write tools are added there by observation)
