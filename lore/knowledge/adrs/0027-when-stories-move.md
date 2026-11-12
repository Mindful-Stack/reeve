---
id: kb/ADR-0027
title: "ADR-0027: A story's owner moves it at the moments of work, and done means its criteria were verified on the merged code"
description: "We decided that the owner, or the agent working for them, moves each story when work starts, when the pull request that completes it opens, when that merges, and when its criteria are verified, because pull-request status may not move a story, one pull request often completes several, and a board that lags the work blocks everyone downstream."
tags: [adr, reeve, delivery, lorekeeper, process]
status: proposed
date: 2026-11-12
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: two-way
blast_radius: shared
sensitivity: []
scope: ["lorekeeper:skills/**", "lorekeeper:references/**"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0002, kb/ADR-0005]
related: [kb/ADR-0014, kb/ADR-0026]
implements: []
rfc:
---

# ADR-0027: A story's owner moves it at the moments of work, and done means its criteria were verified on the merged code

## Status
Proposed 2026-11-12.

## Context
kb/ADR-0002 R4 gives stories five delivery states and forbids pull-request status from moving a story; the GitHub webhook only shows a story's pull requests. Nothing says who moves a story or when. In one initiative, three stacked pull requests implemented thirteen stories over two days while every story stayed in todo: nobody moved them, progress read 0%, and every dependent read "blocked".

Agents do most implementation through Lorekeeper skills, which drive the planner through its CLI when it is present (kb/ADR-0005). One pull request commonly completes several stories, a story may take several pull requests (kb/ADR-0002 R1), and the webhook links a pull request only to the story whose key starts its branch's last path segment. Production releases are batched, and not every repository has a QA environment.

## Facts relied on
- F1 Pull-request status MUST NOT move a story; delivery has five states — source: kb/ADR-0002 R4.
- F2 The webhook links a pull request to the story whose key starts the branch's last path segment, and shows its status only — source: `deploy/README.md` (GitHub webhook).
- F3 Skills use the planner only when it is present and signed in — source: kb/ADR-0005 R1.
- F4 Moving to done needs a resolution, completed only from an active state — source: kb/ADR-0002 R4.

## Considered options
- **The webhook moves stories from pull-request events** — nobody has to remember.
- **The owner moves stories at the moments of work, and the planner flags a story whose pull request has moved ahead of it** — catches forgotten moves without moving anything.
- **The owner moves stories at the moments of work** — no planner change; works when one pull request completes several stories.
- **Done means released to production** — done then says users have it.

## Decision
We decided that a story's owner, or the agent working for them, moves it explicitly when work on it starts, when the pull request that completes it opens and merges, and when its acceptance criteria are verified on the merged code, and that a pull request names the stories it completes, because pull-request status may not move a story, one pull request often completes several, and a board that lags the work blocks everyone downstream.

- **R1** Starting work on a story MUST move it to in progress; a story owned by someone else MUST NOT be started without that person's agreement, and a refusal as not ready MUST stop an agent, which reports the reasons and does not work around them.
- **R2** A pull request MUST name, in a Stories section of its description, exactly the stories whose implementation it completes once merged; when it opens, each named story MUST move to in review.
- **R3** When that pull request merges, each named story MUST move to testing; a story already in testing or done MUST be left where it is.
- **R4** A story MUST move to done with resolution completed when its acceptance criteria have been verified on the merged code, by its owner unless someone else is named, and back to in progress when the verification fails.
- **R5** Moves MUST be made by people, or by agents in the work they were asked to do.

## Consequences
- The board follows the work as long as the moves are made; a forgotten move still leaves a story behind. Merges made in GitHub's web interface depend on the person moving the stories.
- Pull-request descriptions gain a Stories section, and branches put the main story's key at the start of their last path segment (`feat/<key>-…`) so the webhook shows the pull request on that story.
- Done can precede the production release by days: whether a change is live is answered by the release, not the story.
- A pull request that does part of a story's work does not name it, so the story stays in progress until the pull request that completes it.

## Assumptions and invalidation triggers
- *Assumes owners and agents make the moves.* Trigger: at a sprint's close, more than a quarter of the stories named by pull requests merged in that sprint are still in todo, in progress or in review ⇒ review the decision, starting with a planner flag for stories behind their pull requests.
- *Assumes verification follows the merge closely.* Trigger: at a sprint's close, more than a quarter of the stories in testing entered it before the sprint began ⇒ review whether done means merged.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0002 (delivery states and rules), kb/ADR-0005 (Lorekeeper drives the CLI), kb/ADR-0026 (who may build on unmerged work), kb/ADR-0014 (sprints and goals).
