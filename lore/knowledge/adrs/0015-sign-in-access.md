---
id: kb/ADR-0015
title: "ADR-0015: Sign-in access is a locked configuration floor plus admin-edited database grants"
description: "We decided that who may sign in and who is admin is the union of reeve.toml, as a locked floor, and database grants interactive admins edit at runtime, still through Google only, because letting a guest in must not take a config edit, deploy and restart."
tags: [adr, reeve, identity, auth, oidc, tokens]
status: accepted
date: 2026-11-09
decided_by: [Daniel Thyselius]
consulted: []
confidence: high
reversibility: one-way
blast_radius: shared
sensitivity: [security, privacy]
scope: ["reeve:crates/reeve-identity/**", "reeve:crates/reeve-server/**", "reeve:crates/reeve-protocol/**", "reeve:web/src/routes/auth/**", "reeve:web/src/routes/(account)/**", "reeve:web/src/lib/me.svelte.ts"]
supersedes: [kb/ADR-0008]
superseded_by:
depends_on: []
related: [kb/ADR-0016, kb/ADR-0010, kb/ADR-0007, kb/ADR-0012]
implements: []
rfc:
---

# ADR-0015: Sign-in access is a locked configuration floor plus admin-edited database grants

## Status
Proposed 2026-11-09.
Accepted 2026-11-09 by Daniel Thyselius.

## Context
kb/ADR-0008 put the sign-in allowlist and the admin emails in configuration, so that an open sign-in would not expose the roadmap, and named its own remedy: when one config file can no longer keep up, supersede with an admin page. The production instance allows one company domain and one service address. People outside that domain now need access as guests: consultants, partners and customers who sign in with a Google identity on another address. Each guest today costs a change to the rendered `reeve.toml`, a deploy and a server restart, made by someone with access to the host. The team is still about six people; the pressure is the number and churn of guests, not the size of the team.

The server already re-checks, on every authenticated request, that the token's owner is allowed and what their role is, so the allowlist and roles can change without a restart if their source can. The configuration file is also the break-glass path: an operator regains admin by adding a fresh identity to it and restarting, and the scheduled decision check runs as a configured service address that has no Google account. Nothing today requires the configuration to name an admin, or that a configured admin be allowed by the configured lists.

## Facts relied on
- F1 The production instance allows one company domain and one service address, and names two admins, in its configuration — source: the operator's production `reeve.toml`, kept outside this repository (`deploy/README.md`).
- F2 Every authenticated request re-checks the owner's allowlist entry and role — source: `authenticate` in `crates/reeve-identity/src/handler.rs`.
- F3 Changing the allowlist or admins today needs a config change and a restart — source: kb/ADR-0008 *Consequences*; `AppState.identity` built once from config in `crates/reeve-server/src/state.rs`.
- F4 External guests outside the company domain need access — source: Daniel Thyselius, 2026-11-02.
- F5 Configuration validation checks neither that an admin is named nor that a named admin is allowed — source: `ServerConfig::validate` in `crates/reeve-server/src/config.rs`.

## Considered options
- **Keep everything in configuration** — no new tables or RPCs; every guest is a deploy and a restart by someone with host access.
- **Manage access in the identity provider**: a Google Group for members and one for admins, read at sign-in — membership lives where the company already manages people; it needs a Workspace admin and directory API credentials for every instance, guests' personal Google accounts must be added to a company group, and every instance of a public product would depend on one provider's group API.
- **Seed the database from configuration once, then the database only** — one source of truth; a mistaken edit can remove the last admin, and recovery needs SQL on the host.
- **Database only, first admin bootstrapped by a host command** — the cleanest model; changes the deploy, restore and decision-check runbooks, and the service address must be re-created by hand.
- **Configuration as a locked floor plus admin-edited database grants, read on every check** — guests are added in the UI in seconds; the file keeps the service address and a way back to an admin, at the cost of two sources an admin must read together.
- **The same, cached in memory and reloaded on write** — no extra query per request; goes stale after a restore, a manual SQL fix or a second process.
- **A role column on the user row plus an allowlist table** — fewer tables; admin can only be granted after a first sign-in, and the configured admins still need merging.

## Decision
We decided that sign-in access is the union of the instance configuration, as a floor that cannot be changed at runtime, and identity-owned database grants that admins edit through the API, still through Google only, because letting an external guest in must take an admin a minute, not a config edit, deploy and restart, while the file keeps a way back in.

- **R1** The planner MUST NOT store password hashes; sign-in MUST go through Google OpenID Connect; an unverified email, or a verified email allowed by neither the configured floor nor a database grant, MUST be refused.
- **R2** The allowed domains, allowed emails and admins MUST each be the union of the configuration's list and an identity-owned table; entries from configuration MUST NOT be removable or editable through the API; a database row that duplicates a configuration entry MUST be listed as its own row and MAY be removed; the server MUST refuse to start when the configuration names no admin, or names an admin its own domains and emails do not allow.
- **R3** Only an admin calling with an interactive token MAY add or remove an allowlist entry or an admin grant; access writes MUST be serialised and decided on the policy read inside their transaction; each change MUST write an activity entry in the same transaction as the change.
- **R4** An admin MUST NOT be able to revoke their own admin grant or remove an entry after which they are no longer allowed or no longer admin; a database admin grant MUST confer the admin role only while the policy allows its email; removing an entry through the API MUST delete, each with its own activity entry, the database grants it leaves unallowed; a grant left unallowed any other way (a configuration change, a restore, a hand edit) MUST be kept, shown as not allowed, and deleted only by an admin's explicit revocation.
- **R5** Roles MUST be admin or member; every authenticated request, and every batch of an open event stream, MUST check the owner's current allowlist entries, role and disabled flag against the current configuration and database, without a cache that a write elsewhere can leave stale.
- **R6** Removing an allowlist entry or admin grant MUST recompute the owner's access and role from their next request and MUST NOT revoke tokens; an admin MAY disable, with either token kind, any existing user, allowed or not, except themselves and an admin named in configuration; disabling a user MUST revoke that user's tokens.
- **R7** Tokens MUST be interactive or automation; automation tokens MUST NOT override readiness, issue tokens, or add or remove allowlist entries or admin grants, and their writes MUST be logged as the owner via automation; the CLI MUST take an automation token from an environment variable and an interactive one from login.

## Consequences
- Adding a guest is an admin action in the web client and is visible in the activity log; no deploy or restart is involved.
- A guest is a full member: they see and edit every initiative, epic, story and decision a member can. Nothing narrows what a customer or partner sees; that needs a later record (see the triggers).
- Who may sign in is now read from two places: the Access page shows both and marks the configuration's entries as locked, but an operator reading only `reeve.toml` no longer sees the whole list.
- The allowlist and grants are in the database dump; a restore brings them back, while the floor comes from whatever `reeve.toml` the restored server runs with. The readable export derives each user's role from the same policy, grants included.
- A configured admin cannot be disabled through the API, so a compromised configured-admin account is stopped only by the operator on the host (`reeve-server user-disable`) or by a config edit and restart. Before this record any other admin could disable them in the web client.
- A grant orphaned by a configuration change or restore lingers, inert and flagged, until an admin removes it; correcting the configuration revives it, visibly on the Access page.
- Each authenticated request reads the access tables; at this size that is negligible, and a busier instance would need a cache that honours R5.
- Removing a guest blocks them from their next request, unless another entry still allows them, but leaves their tokens in place; re-adding the entry restores them, and an admin who wants the tokens gone must also disable the user.
- Removing an entry through the API can silently demote a database admin it covered; the Access page says so before the removal, and the activity log records it.
- An instance whose configuration names no admin, or an admin the configuration does not allow, no longer starts; such an admin could not sign in before either.
- Adding GitHub or another provider later is still an adapter; nothing in the model assumes one provider.

## Assumptions and invalidation triggers
- *Assumes every person who needs access has a Google identity.* Trigger: a guest or member without one ⇒ add a provider, never a password; supersede.
- *Assumes a flat list of emails and domains, with every guest a full member, is enough.* Trigger: guests need expiry, per-initiative visibility or a role between member and admin ⇒ supersede.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0008 (superseded by this record).
- kb/ADR-0016 (one team per instance; takes the allowlist and admins out of its configuration rule in favour of this record).
- kb/ADR-0010 (the activity port and table ownership these grants follow), kb/ADR-0007.
- kb/ADR-0012 (preferences on the account, which relied on kb/ADR-0008).
