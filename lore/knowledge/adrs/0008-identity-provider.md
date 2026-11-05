---
id: kb/ADR-0008
title: "ADR-0008: Users sign in with Google, gated by a configured allowlist, with admins named in configuration, two token kinds and no password store"
description: "The planner authenticates humans through Google OpenID Connect only, refuses emails outside configured domains and addresses, treats configured emails as admins and everyone else as members, issues interactive and automation tokens with different powers, and stores no passwords, because everyone on the team has a Google identity, an open sign-in would expose the roadmap, and automation must not pass for a person."
tags: [adr, reeve, identity, auth, oidc, tokens]
status: accepted
date: 2026-10-25
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: [security]
scope: ["reeve:crates/reeve-identity/**", "reeve:crates/reeve-server/src/auth/**", "reeve:web/src/routes/auth/**"]
supersedes: []
superseded_by:
depends_on: []
related: [kb/ADR-0009]
implements: []
rfc:
---

# ADR-0008: Users sign in with Google, gated by a configured allowlist, with admins named in configuration, two token kinds and no password store

## Status
Proposed 2026-10-25.
Accepted 2026-10-26 by Daniel Thyselius.

## Context
The team is about six people, including non-coders without GitHub accounts; everyone has a Google identity. The CLI needs a credential that is not a browser session, Lorekeeper skills act for a person but must not be able to use the human-only readiness override, and a token that outlives its owner is a hole. Earlier drafts added a GitHub OAuth adapter, approval queues, a viewer role and team-derived roles; review found all of it to be account management for six people.

## Facts relied on
- F1 Not every member of the team is on GitHub; everyone has a Google identity — source: team lead's statement in the design session; the team's household manifest (internal), core-non-coders team.

## Considered options
- **GitHub OAuth, roles from GitHub teams** — free roles for developers; excludes members without GitHub and adds a sync pipeline.
- **Local accounts with passwords** — works for everyone; hashing, reset flows and invitations.
- **Google OpenID Connect only, allowlist and admins in configuration, members otherwise, two token kinds** — covers everyone with no account management beyond a config file.

## Decision
We decided that the planner authenticates humans through Google OpenID Connect only, refuses any verified email outside the configured domains and addresses, treats configured emails as admins and everyone else as members, issues interactive and automation tokens with different powers, and stores no passwords, because that covers every team member with no account management beyond configuration and no open door.

- **R1** The planner MUST NOT store password hashes; sign-in MUST go through Google OpenID Connect; a verified email outside the configured allowlist MUST be refused.
- **R2** Roles MUST be admin or member, determined by configuration; an admin MAY disable a user, which MUST revoke that user's tokens; every authenticated request MUST check the current role and disabled flag.
- **R3** Tokens MUST be interactive or automation; automation tokens MUST NOT override readiness or issue tokens, and their writes MUST be logged as the owner via automation; the CLI MUST take an automation token from an environment variable and an interactive one from login.

## Consequences
- Adding GitHub sign-in later is an adapter; nothing in the model assumes one provider.
- There is no viewer role; a member can do anything a member can, and the activity log shows who did it.
- Changing who is an admin is a configuration change and a restart, which is acceptable at this size.

## Assumptions and invalidation triggers
- *Assumes every team member has a Google identity.* Trigger: a member without one ⇒ add a provider, never a password.
- *Assumes configuration-managed roles stay cheap.* Trigger: the instance serves more people than one config file can keep current ⇒ supersede with an admin page.

## Later observations
<!-- append-only, accepted records only; newest last -->
- 2026-11-04 (kb/ADR-0014 R5): the interactive-only actions grow: opening or closing a sprint, editing its dates, creating, editing or removing a goal, and recording confidence or a verdict require an interactive token; automation tokens and the system may only link and unlink work and suggest a confidence. No trigger fired; the decision holds.
- 2026-11-04 (public copy, one-time edit): F1's source was a local path; F2 and a sentence of the context, about a third-party prototype the design research studied, are left out of this copy (the numbering is kept). The cited material is not in this repository; the sources now describe it in words or point to the public reeve-runner repository that holds it. No rule or decision changed; the decision holds.

## See also
- kb/ADR-0009.
