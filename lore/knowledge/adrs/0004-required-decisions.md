---
id: kb/ADR-0004
title: "ADR-0004: Decision records stay in git; the planner tracks required decisions, links each to a record by path, and checks the record's status on the default branch"
description: "The planner never owns a decision: it tracks required decisions as its own objects with a decider, links each to a record by repository, path and id without writing anything into the record, and a scheduled check reads the file's front-matter status on the default branch, because the versioned file must stay the single truth and accepted records are locked."
tags: [adr, reeve, decisions, lorekeeper, rfc]
status: accepted
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve-planning/**", "reeve:crates/reeve/src/commands/decision*", "lorekeeper:skills/recording-decisions/**"]
supersedes: []
superseded_by:
depends_on: []
related: [kb/ADR-0003, kb/ADR-0005]
implements: []
rfc:
---

# ADR-0004: Decision records stay in git; the planner tracks required decisions, links each to a record by path, and checks the record's status on the default branch

## Status
Proposed 2026-10-25.
Accepted 2026-10-26 by Daniel Thyselius.

## Context
The team keeps architecture decision records beside the code they govern and shared ones in the knowledge base, with front matter carrying status and relations. Lorekeeper locks an accepted record to five permitted edits, and a proposal's PR merges only once the record is accepted, so proposed records exist only on open branches. RFCs were written separately, often with security, UX or product people, and covered both broad product questions and non-architectural hard-to-reverse choices. Practitioners keep records in the repo and the tracker holds a tracking item that links out; Rust's RFC process is the canonical example. An earlier draft of this design mirrored every record, including proposals on branches, and review found that this produced status-mapping, branch-scanning and snapshot-ordering machinery out of proportion to the problem; a later draft cached record bodies and commit stamps, which review found unnecessary since the team reads records on GitHub.

## Facts relied on
- F1 Shared ADRs use front matter with id, status, supersedes and superseded_by — source: an accepted record in the team's knowledge base (internal).
- F2 Accepted records are locked to five edits and a proposal merges only once accepted — source: Lorekeeper `skills/recording-decisions/reference/lifecycle.md` on origin/main.
- F3 Non-coders on the team have read access to the knowledge-base repository — source: the team's household manifest (internal), team access.
- F4 Rust's RFC merge procedure creates the tracking issue first and links it from the RFC — source: internal design research of 2026-10-25, §6 (not published).

## Considered options
- **Decisions authored and stored in the planner** — one place for non-technical readers; two sources of truth.
- **Mirror every record from every home, including proposals on branches** — a complete catalogue; heavy sync machinery and collisions between canonical and proposed versions.
- **Required decisions as planner objects, each linked to one record by path, checked for status on the default branch** — readiness from one lookup, the file untouched, nothing to scan.

## Decision
We decided to keep every decision record in its repository or knowledge-base home, to track the decisions an item requires as planner-owned objects with a decider and an optional due date, to link each to a record by repository, path and id written only on the planner side, and to run a check that reads the file's front-matter status on the default branch, because readiness needs one fact per required decision and the versioned file must remain the only truth and must not be edited to add links.

- **R1** The planner MUST NOT edit, accept, reject or cache a record; the only record data it stores is the check result and the reference.
- **R2** A required decision MUST be a planner-owned object with title and decider, an optional due date, attached to an initiative, epic or story, optionally referencing one record by repository, path and id and one PR url; closed-by-a-person and satisfied-by-the-record MUST be two separate facts, and closing MUST be a logged action with a reason; a required decision MUST represent a consequential choice that blocks work, not a clarification.
- **R3** Setting or changing a reference MUST reset the check result and trigger an immediate check of that required decision; no item keys MAY be written into records.
- **R4** The check MUST cover every required decision that has a reference and is not closed, including satisfied ones, MUST read each referenced file on the repository's default branch and parse `status` and `superseded_by` from front matter, MUST report not found when the file cannot be read and "proposed, PR open" when a PR url is set, and MUST NOT walk ADR directories or depend on Lorekeeper.
- **R5** Broad product reasoning MUST live in the initiative brief; a hard-to-reverse non-architectural choice MUST be a decision record, and Lorekeeper's record triage MUST draft process, tooling and product records rather than decline them; no separate RFC artifact MAY be introduced.

## Consequences
- Readers follow the link to GitHub for a record's text; the planner shows only what is open, who owns it and whether it is accepted.
- A superseded or rejected record shows as a reason on the required decision, and a person re-points or closes it; nothing is unready forever by accident.
- The Lorekeeper changes are the record triage and one link step at draft time.

## Assumptions and invalidation triggers
- *Assumes a check every ten minutes is frequent enough.* Trigger: staleness between acceptance and check is routinely noticed ⇒ run the check from a merge webhook.
- *Assumes reading records on GitHub is acceptable to non-coders.* Trigger: a demonstrated need to read or author records outside git ⇒ supersede.

## Later observations
<!-- append-only, accepted records only; newest last -->
- 2026-11-03: kb/ADR-0013 extends this record: a required decision carries a reversibility (R2), every close carries a kind, decided or withdrawn (R2), and the check also reads the record's `reversibility` front-matter key and reports it inside the check result (R4); R1 still holds.
- 2026-11-04 (public copy, one-time edit): F1's and F3's sources were local paths, and F4's named an internal research document. The cited material is not in this repository; the sources now describe it in words or point to the public reeve-runner repository that holds it. No fact, rule or decision changed; the decision holds.

## See also
- kb/ADR-0003, kb/ADR-0005.
