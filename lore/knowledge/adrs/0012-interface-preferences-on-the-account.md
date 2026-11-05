---
id: kb/ADR-0012
title: "ADR-0012: Interface preferences live on the user's account in the identity context as validated typed values, starting with a theme synced from Omarchy"
description: "A user's interface preferences are stored on their user row, owned by the identity context, as validated typed values that the clients interpret, never as CSS or free text; the first is a colour theme set by `reeve theme set` from Omarchy, because a browser tab cannot read the desktop's theme file and typed values keep user input out of the page."
tags: [adr, reeve, identity, sveltekit, cli]
status: accepted
date: 2026-10-31
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: two-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve-identity/migrations/2002_user_preferences.sql", "reeve:crates/reeve-identity/src/preferences*", "reeve:crates/reeve-protocol/src/identity.rs", "reeve:crates/reeve/src/commands/theme.rs", "reeve:web/src/lib/theme.ts", "reeve:contrib/omarchy/**"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0006, kb/ADR-0008, kb/ADR-0010]
related: [kb/ADR-0011]
implements: []
rfc:
---

# ADR-0012: Interface preferences live on the user's account in the identity context as validated typed values, starting with a theme synced from Omarchy

## Status
Proposed 2026-10-31.
Accepted 2026-11-03 by Daniel Thyselius.

## Context
The web client is being redesigned around a small set of colour tokens with Rosé Pine Dawn and Rosé Pine as the defaults. At least one member of the team runs Omarchy, whose theme switcher rewrites a `colors.toml` of named role colours on every switch and runs user hooks afterwards, and wants Reeve to follow that theme. A page served by the planner runs in a browser sandbox and cannot read that file; something on the user's machine has to carry the colours to where the page can read them. The planner has no home for user preferences today: the identity context holds credentials and user administration, the TUI keeps its own settings locally (kb/ADR-0011), and planning has no notion of a person beyond an email. Every identity write is recorded as an activity entry, and kb/ADR-0008 R3 requires automation writes to be logged as the owner. Theme files come from many authors; some have low text contrast or status colours that are hard to tell apart.

## Facts relied on
- F1 Omarchy writes the current theme's role colours (`mode`, `background`, `foreground`, `accent`, `selection`, `muted`, the ANSI colours) to `~/.local/state/omarchy/current/theme/colors.toml` — source: that file on an Omarchy v4 machine.
- F2 Omarchy runs executable hooks in `~/.config/omarchy/hooks/theme-set.d/` after every theme switch — source: `~/.config/omarchy/hooks/theme-set.sample`.
- F3 Every identity mutation records an activity entry in the same transaction — source: `crates/reeve-identity/src/handler.rs` (`activity.record`).
- F4 Views refetch on every activity entry and every reconnect — source: kb/ADR-0011 R10, `web/src/lib/events.svelte.ts` (`onLive`).

## Considered options
- **Preferences in the browser only** (a pasted theme in local storage) — no server change at all; manual after every switch, per browser and device.
- **The page re-reads the file through the browser's file-access API** — automatic on window focus with no server change; Chromium only, per device, and permission prompts return.
- **A hook that posts with `curl` and a pasted token** — no CLI dependency; a long-lived token sits in a script and TOML has to become JSON in shell.
- **Store CSS per user** — any look can be expressed; user input becomes page styles, which invites injection and breaks on every token rename.
- **Typed values on the account but unlogged, picked up by polling `/me` on window focus** — no activity noise; the only unlogged identity write, and a second refresh path beside the event stream.
- **Typed values on the account in the identity context, logged like every identity write, set from the CLI** — automatic through the hook, follows the user to every device, the server holds only validated values, and the existing refetch carries changes to open tabs.

## Decision
We decided to keep a user's interface preferences on their user row, owned by the identity context, as validated typed values that each client interprets, starting with a colour theme of raw role colours set by `reeve theme set` from Omarchy's `colors.toml`, because a browser tab cannot read the desktop's theme file and typed values keep user input out of the page.

- **R1** A preference MUST be a typed value validated by the server (the theme: `mode` light or dark and named role colours, each `#rrggbb`); the server MUST refuse unknown keys and MUST NOT store CSS, markup or free text.
- **R2** Preferences MUST belong to the identity context and be set and cleared only by the signed-in user for their own account; `reeve-planning` MUST NOT read them.
- **R3** Every preference write MUST record an activity entry like any identity write, so automation writes are logged as the owner (kb/ADR-0008 R3); clients MUST pick up changes through the existing refetch on activity (kb/ADR-0011 R10) and MUST NOT add an event kind or polling for them.
- **R4** The server MUST store only what the user sent; every derived colour and every contrast adjustment MUST be computed by the client at render time, and the web client MUST refuse a theme whose text does not reach 4.5:1 on its background.

## Consequences
- The theme follows a user to every device they sign in on; other users keep the default.
- Syncing needs the CLI installed and signed in on the Omarchy machine; without it a user keeps the default.
- Every open view refetches once when anyone changes their theme; theme changes are rare, and the Activity view hides these entries by default.
- Clearing the theme in the web is undone by the hook on the next theme switch until the hook is removed.
- Fonts and other preferences need their own typed fields and a change to this record's scope, not a new mechanism.
- Reverting means dropping one column, two routes, a port and a CLI command; an installed hook then fails silently until removed.

## Assumptions and invalidation triggers
- *Assumes Omarchy keeps a flat `colors.toml` with named roles.* Trigger: Omarchy changes the file's location or keys ⇒ amend the CLI's mapping; if role names stop existing, supersede.
- *Assumes preference changes are rare enough that a refetch per change is cheap.* Trigger: preference writes show up as a measurable share of refetches ⇒ supersede R3 together with kb/ADR-0011 R10.
- *Assumes preferences are personal.* Trigger: a request for a team-wide theme ⇒ supersede with an instance-level setting in configuration.

## Later observations
<!-- append-only, accepted records only; newest last -->
- 2026-11-04 (public copy, one-time edit): See also named an internal design document. The cited material is not in this repository; the sources now describe it in words or point to the public reeve-runner repository that holds it. No fact, rule or decision changed; the decision holds.

## See also
- kb/ADR-0006, kb/ADR-0008, kb/ADR-0010, kb/ADR-0011.
- The personal theme design of 2026-10-31 (internal, not published).
