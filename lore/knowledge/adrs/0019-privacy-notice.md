---
id: kb/ADR-0019
title: "ADR-0019: Each instance serves a privacy notice from a statement kept with the code"
description: "We decided that the repository carries the statement of what personal data Reeve processes, served unauthenticated at /privacy and completed from reeve.toml by the operator who controls the instance, because people in Reeve are owed that information and only the code can say truthfully what it processes."
tags: [adr, reeve, identity, privacy, configuration]
status: proposed
date: 2026-11-10
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: two-way
blast_radius: shared
sensitivity: [privacy]
scope: ["reeve:crates/reeve-server/src/privacy.rs", "reeve:crates/reeve-server/src/privacy.html", "reeve:crates/reeve-server/src/config.rs", "reeve:crates/reeve-server/src/routes/activity.rs", "reeve:crates/reeve-server/src/routes/events.rs", "reeve:crates/*/migrations/**", "reeve:web/src/routes/auth/signin/**", "reeve:reeve.example.toml", "reeve:deploy/README.md", "reeve:deploy/erase-person.sql"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0015, kb/ADR-0016]
related: [kb/ADR-0010, kb/ADR-0006]
implements: []
rfc:
---

# ADR-0019: Each instance serves a privacy notice from a statement kept with the code

## Status
Proposed 2026-11-10.

## Context
kb/ADR-0015 lets admins add guests outside the company domain, who sign in with any Google account. For Google to let such an account in, the OAuth consent screen must be External and published to production, and Google refuses that until the Branding page names a home page and a privacy policy URL on an authorized domain. People who sign in are owed the information in GDPR Art. 13; people who appear in Reeve without signing in (an invited guest's entry, an imported or assigned author, someone mentioned) are owed the information in Art. 14.

Reeve has no privacy notice. The production operator's public policy describes its customer product, promises deletion within 30 days and lists other cookies, so it would mislead Reeve's users. Reeve is self-hosted, one team per instance (kb/ADR-0016), and its repository is public: the repository cannot know an operator's legal identity, purposes and basis, retention, hosting or transfers, and an operator cannot easily know what each release processes.

What Reeve processes, read from the code on 2026-11-10: email and display name; whether access is enabled; guest entries and admin grants with who added them; SHA-256 hashes of sessions and CLI tokens with label, creation, expiry and revocation, never deleted; items, comments, decisions, goals and sprints with authors, owners and other people fields; an append-only activity log keyed by email (kb/ADR-0010 R5) whose payloads keep earlier values and which records every sign-in and token issue; refused authentications logged with the owner's email; pull-request status from GitHub with no GitHub user; two cookies (session, and a 10-minute sign-in binding) and a `sessionStorage` report draft holding the email. No passwords, no Google tokens after sign-in, no IP addresses in Reeve's logs, no analytics. Every signed-in member can list every activity entry, sign-ins included. Removing or disabling a user keeps the user row and authored work (kb/ADR-0015 R4, R6). The backup script never prunes. Nothing in the product erases a person.

Two independent legal-design reviews of an earlier draft agreed that a notice promising lifetime retention and an activity log that is "never edited" reads as refusing erasure in advance, and that the data inventory above must be stated accurately.

## Facts relied on
- F1 An External OAuth app can be published to production only once the Branding page has an app name, support email, home page URL and privacy policy URL — source: the Google Cloud console's "Make external" dialog, observed 2026-11-10.
- F2 Guests outside the company domain need access — source: kb/ADR-0015 F4.
- F3 The production operator's public privacy policy covers its customer product and promises deletion within 30 days — source: https://grantigo.com/integritetspolicy, read 2026-11-10.
- F4 The personal data listed in Context is what Reeve persists, logs or stores in the browser — source: `crates/*/migrations/`, `crates/reeve-identity/src/handler.rs` (`signed_in`, `token_issued`, refused-authentication logs), `crates/reeve-server/src/auth/routes.rs` (cookies), `web/src/lib/report.svelte.ts` (`sessionStorage`).
- F5 The activity log is append-only through the product and keyed by the actor's email — source: kb/ADR-0010 R5; `crates/reeve-activity/migrations/3001_activity.sql`.
- F6 Any signed-in member can list and stream every activity entry — source: `crates/reeve-server/src/routes/activity.rs`, `crates/reeve-server/src/routes/events.rs`.
- F7 Backups are never pruned by the repository's script — source: `scripts/backup.sh`.

## Considered options
- **Point at the operator's existing privacy policy** — no work; it describes another product and misstates retention and cookies.
- **The operator writes the whole notice and Reeve serves the file** — any language and wording; the description of what Reeve processes is copied out of the repository and drifts.
- **A generic statement in the repository only, linked from GitHub** — kept with the code; it names no controller, purposes or retention, and is not on the operator's authorized domain.
- **A generic statement in the repository, served by each instance and completed from configuration** — the description changes in the same diff as the code, and the operator supplies only what only they know; the text is English and fixed per release.
- **Build an erasure tool first** — erasure becomes a product operation; it amends kb/ADR-0010 R5 and delays the notice that unblocks guests. A tested manual runbook covers requests meanwhile.

## Decision
We decided that the repository carries the statement of what personal data Reeve processes, served unauthenticated at `/privacy` by every instance that configures it and completed from `reeve.toml` by the operator who controls the instance, because the people in Reeve are owed that information and only the code can say truthfully what it processes.

- **R1** The statement MUST describe every kind of personal data Reeve persists, logs, receives or stores in the browser, its sources, who can see it, and the product's own retention behaviour; a change to any of these MUST update the statement and its revision date in the same pull request.
- **R2** The statement MUST name the controller configured for the instance; it MUST NOT name the Reeve project or its authors as controller or processor merely for supplying the software, and the operator's values MUST disclose any role they actually perform.
- **R3** When `[privacy]` is present, the server MUST refuse to start unless the controller, a contact email, the purposes and legal basis, the retention, the hosting and the transfers are configured; when it is absent, `/privacy` MUST answer 404, the sign-in page MUST NOT link to it, and a server with browser sign-in MUST log a warning at start-up.
- **R4** `/privacy` MUST be readable without signing in, MUST NOT read or set a cookie, and MUST disclose nothing beyond the statement and the operator's configured values; every configured value MUST be escaped as text.
- **R5** Until the product can erase a person, the statement MUST say that removing or disabling access keeps the account, authored work and activity log, that the product has no erasure function, and that the operator carries out erasure in the database, the activity log included; it MUST NOT say that any record is never edited. The repository MUST carry a manual erasure procedure whose coverage of every column holding an email is tested.
- **R6** Activity entries about an account (sign-ins, token issues and revocations, disabling and enabling) MUST be listed and streamed only to admins and to the account they concern.

## Consequences
- An operator gets an accurate notice by setting six required keys, and the consent screen can point at `<public_url>/privacy`.
- The required keys force the operator to decide retention, transfers and a legal basis before guests sign in; an instance that cannot answer them cannot serve a notice.
- Every pull request that processes personal data differently carries a documentation duty that review must catch, and every migration that adds an email column must extend the erasure procedure or fail its coverage test.
- Members can no longer see when colleagues signed in; anything that relied on reading those entries as a member stops seeing them.
- The wording is the repository's and English only; an operator needing another language or wording must propose it upstream or serve their own page.
- Erasure stays a manual database operation by the operator until the erasure tool exists, and a placeholder is pseudonymisation, not necessarily anonymisation: colleagues may still recognise the person from context.
- Reeve gains its first public page outside sign-in, a new place where an unescaped value would be an injection.
- Whether the operator's values meet the law is the operator's responsibility.

## Assumptions and invalidation triggers
- *Assumes personal data cannot be erased through the product.* Trigger: an erasure or pseudonymisation operation ships ⇒ supersede R5.
- *Assumes the software's authors have no access to any instance's data.* Trigger: project maintainers operate an instance or hold operational access for someone else, or a hosted offering starts ⇒ supersede R2.
- *Assumes an English notice is understood by every instance's users.* Trigger: an operator's users need another language ⇒ supersede.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0015 (guests outside the domain; access removal keeps records; the policy re-read that R6 filters with).
- kb/ADR-0016 (one team per instance; instance settings in configuration).
- kb/ADR-0010 (the append-only activity log; amended when the erasure tool is designed).
- kb/ADR-0006 (one binary; the page is rendered by the server).
