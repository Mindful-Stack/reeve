---
id: kb/ADR-0016
title: "ADR-0016: The planner is self-hosted, one team per instance, with no organisation or household column"
description: "We decided to keep shipping Reeve as one self-hosted instance per team, with its key prefix, repositories, buckets and sprint settings in configuration and no tenant column, while sign-in access moves to kb/ADR-0015, because tenant scoping is effort the product does not need."
tags: [adr, reeve, deployment, tenancy, hosting]
status: accepted
date: 2026-11-09
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: [privacy]
scope: ["reeve:crates/**", "reeve:deploy/**"]
supersedes: [kb/ADR-0009]
superseded_by:
depends_on: [kb/ADR-0015]
related: [kb/ADR-0007]
implements: []
rfc:
---

# ADR-0016: The planner is self-hosted, one team per instance, with no organisation or household column

## Status
Proposed 2026-11-09.
Accepted 2026-11-09 by Daniel Thyselius.

## Context
kb/ADR-0009 made the planner a self-hosted instance serving one team and listed, in its R2, what is instance configuration: the key prefix, the repositories and their ADR directories, the horizon bucket names, the sign-in allowlist and the admin emails. A later observation on it added the sprint settings. kb/ADR-0015 takes the allowlist and the admins out of configuration alone: they become a configured floor plus database grants admins edit at runtime. An accepted record cannot be amended, so this record replaces kb/ADR-0009 with that clause moved; nothing else about tenancy or the reference deployment changes.

## Facts relied on
- F1 R2's configuration includes `[sprints] length_days` and `upkeep_line` — source: kb/ADR-0009 *Later observations* (2026-11-04); `crates/reeve-server/src/config.rs`.
- F2 No table carries an organisation, tenant or household column — source: the migrations under `crates/*/migrations/`.

## Considered options
- **Keep kb/ADR-0009 and let kb/ADR-0015 override its R2** — no new record; two accepted records would contradict each other on where the allowlist lives.
- **Fold tenancy and deployment into kb/ADR-0015** — one record; identity and hosting would be mixed, and the next hosting change would have to supersede an identity record.
- **Replace kb/ADR-0009 with the same decision, the access clause moved to kb/ADR-0015** — each concern keeps its own record, at the cost of one more record to read.

## Decision
We decided to keep shipping the planner only as a self-hosted instance serving one team, with the key prefix, repositories, horizon buckets and sprint settings in configuration, sign-in access as kb/ADR-0015 says, and no organisation or household concept in the schema, because the product is a planning tool for a team that runs it and any grouping above the initiative would be cost without a customer.

- **R1** No table MAY carry an organisation, tenant or household column; an instance MUST be assumed to belong to one team.
- **R2** The key prefix, the repositories and their ADR directories, the horizon bucket names and the sprint settings MUST be instance configuration; the sign-in allowlist and the admins MUST follow kb/ADR-0015.
- **R3** The reference deployment MUST be one server container and one PostgreSQL container behind the team's existing reverse proxy, provisioned by the existing host scripts.

## Consequences
- Anyone else wanting Reeve runs their own instance; a second team at the same company runs a second instance, with its own allowlist and admins.
- Access control is per instance and role; there is nothing to scope.
- A reader looking for what the instance is configured with now reads two records: this one, and kb/ADR-0015 for who may sign in.

## Assumptions and invalidation triggers
- *Assumes no hosted offering is wanted.* Trigger: a decision to offer Reeve as a service ⇒ supersede; the schema change is large and that is intended.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0009 (superseded by this record).
- kb/ADR-0015 (sign-in access), kb/ADR-0007.
