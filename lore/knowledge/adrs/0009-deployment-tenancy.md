---
id: kb/ADR-0009
title: "ADR-0009: The planner is self-hosted, one team per instance, with no organisation or household column"
description: "Reeve is deployed as one self-hosted instance per team, with the key prefix, repositories and sign-in rules in configuration and no organisation or household column in the schema, because a hosted multi-organisation service and its tenant scoping are effort the product does not need and a recurring source of bugs."
tags: [adr, reeve, deployment, tenancy, hosting]
status: superseded
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: [privacy]
scope: ["reeve:crates/**", "reeve:deploy/**"]
supersedes: []
superseded_by: kb/ADR-0016
depends_on: []
related: [kb/ADR-0008, kb/ADR-0007]
implements: []
rfc:
---

# ADR-0009: The planner is self-hosted, one team per instance, with no organisation or household column

## Status
Proposed 2026-10-25.
Accepted 2026-10-26 by Daniel Thyselius.
Superseded 2026-11-09 by kb/ADR-0016.

## Context
The planner replaces Jira for one company and is open source as the successor to Reeve. The team's services run as Docker containers on hosts in Stockholm so that data stays in Sweden. An earlier draft kept a household as a grouping inside an instance; review found that one team has one household and that the concept was a second tenant boundary in disguise.

## Facts relied on
- F2 The team's infrastructure provisions Docker hosts in a Stockholm zone so data stays in Sweden — source: the team's infrastructure repository (internal).

## Considered options
- **Hosted multi-organisation service** — one deployment serves everyone; an organisation boundary in every table and query.
- **Self-hosted, several households per instance** — one deployment for a person with several teams; a grouping to scope every view and key.
- **Self-hosted, one team per instance, configuration for prefix and repositories** — no scoping anywhere.

## Decision
We decided to ship the planner only as a self-hosted instance serving one team, with the key prefix, repositories, horizon buckets, allowlist and admins in configuration and no organisation or household concept in the schema, because the product is a planning tool for a team that runs it and any grouping above the initiative would be cost without a customer.

- **R1** No table MAY carry an organisation, tenant or household column; an instance MUST be assumed to belong to one team.
- **R2** The key prefix, the repositories and their ADR directories, the horizon bucket names, the sign-in allowlist and the admin emails MUST be instance configuration.
- **R3** The reference deployment MUST be one server container and one PostgreSQL container behind the team's existing reverse proxy, provisioned by the existing host scripts.

## Consequences
- Anyone else wanting Reeve runs their own instance; a second team at the same company runs a second instance.
- Access control is per instance and role; there is nothing to scope.

## Assumptions and invalidation triggers
- *Assumes no hosted offering is wanted.* Trigger: a decision to offer Reeve as a service ⇒ supersede; the schema change is large and that is intended.

## Later observations
<!-- append-only, accepted records only; newest last -->
- 2026-11-04 (kb/ADR-0014): R2's instance configuration gains `[sprints] length_days` (default 14) and `upkeep_line` (default 20, a reference line). Sprints and goals carry no team column, so R1 holds. No trigger fired; the decision holds.
- 2026-11-04 (public copy, one-time edit): F2's source was a local path and named the hosting zone; F1 and a sentence of the context, about a third-party prototype the design research studied, are left out of this copy (the numbering is kept). The cited material is not in this repository; the sources now describe it in words or point to the public reeve-runner repository that holds it. No rule or decision changed; the decision holds.

## See also
- kb/ADR-0007, kb/ADR-0008.
