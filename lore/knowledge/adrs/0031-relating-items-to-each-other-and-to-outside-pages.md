---
id: kb/ADR-0031
title: "ADR-0031: An item link is a named, informational connection between two items; a web link is a titled URL on one item"
description: "We decided that any two items can be joined by an item link with one of three fixed names, and that an item can carry web links, both informational and kept apart from story blockers, because relating work must never change what the readiness gate reads."
tags: [adr, reeve, planning, domain, cli]
status: accepted
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
Accepted 2026-11-24 by Daniel Thyselius.

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
- 2026-11-24 (GRA-877, https://github.com/Mindful-Stack/reeve/pull/73): the answers of writes and R3. The answer of a write that returns one item (create, edit, move, mark) carries the item's item links, because it is built by the same reader as a read of one item. That reader reads item links after the write's commit, in a separate read-only transaction, so no item link is read while a story is moved and R3 holds as written. It is held by the isolation lint (`handler/moves.rs` may name none of the item-link functions), by the snapshot test (`item_links_change_no_readiness_delivery_progress_rank_or_version`) and by its statement-level check that readiness and a move send no statement naming `item_links`. No trigger fired; the decision holds.
- 2026-11-24 (GRA-878, https://github.com/Mindful-Stack/reeve/pull/74): The implementation also refuses a URL with a user name or a password (`canonical_url`), besides one whose scheme is not `http` or `https`; R5 does not require it and stands as written. The activity entries `web_link_added` and `web_link_removed` keep the URL after the web link is removed, so a credential in one could not be taken back. The snapshot test the observation above names is now `item_links_and_web_links_change_no_readiness_delivery_progress_rank_or_version` and covers web links too. No trigger fired; the decision holds.
- 2026-11-24 (GRA-879, https://github.com/Mindful-Stack/reeve/pull/76): plans and the export; how R2 and R7 read for `reeve apply`. `reeve apply` is a second way another tool's data becomes items; a plan written from such data is an import in R2's sense, so R2 applies to it as written. What holds it there: the name is a closed enum, so `blocks` or `depends_on` as a link's `type` fails the whole plan; the CLI points at `blocks`; the `apply` help and the planning node say that such a relation goes under `blocks` or is left out. Nothing checks that a `follows` in a plan was a blocker in the other tool. R7: plan apply writes each item link and each web link, with its entries, in a transaction of its own after the plan's items exist, and never all of a plan's links in one (why: `lore/knowledge/general/architecture.md`, a mutation records its activity entry last). "Logged on every item it names, in the same transaction" therefore holds per link; a plan whose apply stops at a link keeps the links before it, and applying the same plan id again finishes, once whatever made the link fail is corrected. The export lists both kinds, read whole from their tables. No trigger fired; the decision holds.
- 2026-11-25 (GRA-902, https://github.com/Mindful-Stack/reeve/pull/<n>): the MCP tools; how R2 reads for a chat. `add_item_link`, `remove_item_link`, `add_web_link` and `remove_web_link` (named by the observation for GRA-901 on kb/ADR-0029) are a third way to the writers the API and the CLI reach. We read a chat that is handed another tool's data as an import in R2's sense, as the observation for GRA-879 reads a plan. What holds R2 on this path: `type` is the closed enum on both item link tools, and `blocks`, `depends_on` and their other spellings are refused with a pointer to blockers ("Blocking is not an item link. Do not add it under another name: tell the person that a blocker is added by a person, in the CLI or the web client."; `a_blocking_word_as_the_type_is_answered_with_where_blocking_lives`); `add_item_link`'s description tells the model never to record another tool's "blocks" or "depends on" as an item link, under any name, and not to add one in place of a dependency the person names, but to tell the person that a blocker is added by a person (the sentences are pinned by `a_description_says_what_only_it_can_tell_the_model`); and the receipt's note after a `follows` between two stories says that it is informational and that a dependency is a blocker, which a person adds in the CLI or the web client. Here the importer is the model, so a description is an instruction and not a check. Nothing checks that a `follows` a chat adds was a blocker. The note is said only for a `follows` between two stories, so a dependency recorded as `relates_to` or `duplicates`, or a `follows` with an epic or an initiative at either end, gets none. R3 is now also held through the router by `the_link_tools_gate_nothing_and_no_tool_touches_a_blocker`: every successful add and removal by the four tools, on a blocked story and on its blocker, leaves the blocker rows, readiness, delivery and versions as they were. R7 holds unchanged: the tools call the same actions, which write an entry on every item a change names, with the client in it. A chat can now write web link titles and URLs; the observation for GRA-901 on kb/ADR-0029 says what that means for planted text and for the operator's privacy statement. No trigger fired; the decision holds.

## See also
- kb/ADR-0027 (the blocker and the gate)
- kb/ADR-0002 (hierarchy and bound vocabulary)
- kb/ADR-0014 (goal links, an earlier use of the word)
- kb/ADR-0029 (the read tools show both lists; write tools are added there by observation)
