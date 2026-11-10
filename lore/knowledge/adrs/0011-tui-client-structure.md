---
id: kb/ADR-0011
title: "ADR-0011: The planner's TUI is an interactive ratatui client built as a reducer of a board read model and workflow slices"
description: "The planner's TUI is a ratatui reducer: a board read model plus workflow slices that end in requests, with pure updates returning effects as data, server replies as the only truth and the user's interactive login as its only identity, because pure updates are testable without a terminal or server, server replies replace an optimistic merge, and a person acting there is always themselves."
tags: [adr, reeve, tui, cli, architecture]
status: accepted
date: 2026-10-28
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: one-way
blast_radius: shared
sensitivity: []
scope: ["reeve:crates/reeve/src/tui/**"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0001, kb/ADR-0006, kb/ADR-0008]
related: [kb/ADR-0003, kb/ADR-0010]
implements: []
rfc:
---

# ADR-0011: The planner's TUI is an interactive ratatui client built as a reducer of a board read model and workflow slices

## Status
Proposed 2026-10-28.
Accepted 2026-10-29 by Daniel Thyselius.

## Context
The planner design calls for a thin terminal board client: the board, the drawer's readiness reasons, move with the override prompt and set epic, live over the event stream. It runs as `reeve tui` inside the `reeve` CLI crate, which depends only on the protocol crate. The TUI is a presentation adapter, a client of the planning context: it is not a bounded context, its modules are client modules, and the planning domain's boundaries and rules stay on the server, so kb/ADR-0010's port and isolation rules govern the crates, not the TUI's modules.

The runner's TUI, the code the design names as the seed, is a ratatui application whose board state is already owned by one loop: background tasks report over a channel and the loop applies what they send; its only locking is in the embedded-session terminal parsers, which the planner drops. Its shape differs in three ways that matter here. One action handler of about 1500 lines holds every feature's logic and both changes state and starts I/O. One `App` struct of about twenty-five fields holds every feature's state. And it moves cards optimistically, reconciling a 500 ms poll against maps of in-flight moves, edits and deletes, because a runner move started containers and could take seconds. A planner move is one database write guarded by a row version, a stale write is refused as a conflict, and changes arrive as server-sent activity entries rather than through a poll.

The CLI lets `REEVE_TOKEN`, an automation token, win over the stored login. Only an interactive token may override the readiness gate, and the server derives every write's provenance from the token's kind. The TUI is the surface where a person overrides, and is expected to become where a person steers automation from.

## Facts relied on
- F1 The `reeve` crate depends only on `reeve-protocol` among the workspace crates, the others being dev-dependencies — source: `crates/reeve/Cargo.toml`.
- F2 Story content edits and moves carry the caller's `version` and a stale one is refused with a conflict; readiness, PR links, blocker edges and neighbours' ranks change without a version increment — source: `crates/reeve-protocol/src/requests.rs` (`MoveStory`, `EditItem`, `RankStory`, `BlockEdge`), `lore/knowledge/general/architecture.md` §Transactions and time.
- F3 `GET /api/v1/events` streams activity entries with resumable ids, and clients refetch on any entry and on reconnect — source: `crates/reeve-server/src/routes/events.rs`, `lore/knowledge/general/architecture.md` §Browser and live updates.
- F4 The runner TUI keeps optimistic in-flight maps (`pending_mvs`, `pending_edits`, `pending_deletes`, `pending_mv_intents`) and merges every poll against them; its board state is applied on one loop from channel messages, and its only locks are in the session terminal code — source: `Mindful-Stack/reeve-runner:crates/reeve/src/tui/app.rs`, `actions.rs` (`merge_cards`), `sessions.rs`.
- F5 The CLI resolves `REEVE_TOKEN` before the stored token, and `GET /me` reports the token's kind — source: `crates/reeve/src/config.rs` (`resolve`), `crates/reeve-protocol/src/activity.rs` (`Me.token_kind`).
- F6 The server derives each request's `Via` from the token's kind — source: `crates/reeve-server/src/auth/mod.rs` (the `TokenKind::Automation => Via::Automation` mapping).
- F7 The web client refetches the whole view on any activity entry and on every (re)connect, 500 ms after the last change and at most 5 s after the first of a burst, without filtering by entity — source: `web/src/lib/events.svelte.ts` (`onLive`).

## Considered options
- **Port the runner TUI's shape** (one `App`, one action handler that changes state and starts I/O, optimistic moves with pending-merge bookkeeping) — familiar, working code that already keeps its board state on one loop, and the fastest start.
- **A reducer organised by layer** (`state.rs`, `msg.rs`, `update.rs`, `view/`) — the whole state is visible in one place.
- **An inline-await loop** (each action awaits its request before the next key is read) — the least code.
- **A reducer of a board read model and workflow slices** — each workflow is read, changed and tested in one place.

## Decision
We decided to build the planner's TUI on ratatui and crossterm as a reducer of a board read model and workflow slices, whose update functions are pure and return effects as data, acting only under the user's interactive login, because pure updates make every behaviour testable without a terminal or a server, server replies ordered by request replace the runner's optimistic merge, slices keep each workflow in one place instead of one action handler, and a person acting in the TUI must always act as themselves.

- **R1** The TUI MUST be built on ratatui with the crossterm backend and MUST live under `crates/reeve/src/tui/`, reached as `reeve tui`.
- **R2** The board is the read model: it MUST own the server data on display, the selection, the filter and search query, and each story's pending mark (its write lock), which it receives as messages. Every other slice is a workflow that ends in a request: moves (move windows, the state picker, and the resolution and override prompts), set_epic (the epic picker) and live (the event-stream connection, its resume id, backoff and refetch debounce, and the ticker). The core owns identity, input mode, the drawer, help and the status line, and the request sequence; each slice owns the generation counters of its own timers and replies. A feature MUST be a slice when it holds state across more than one message; anything else is a one-shot action in the core (take, which is `PATCH /stories/{key}` setting the owner to the caller; copy; open). A slice's state fields MUST be private to its module: other modules read them through accessor methods and change them only through the slice's `update(&mut State, Msg, &Ctx)` or its note handler, so the compiler enforces single ownership. Value types a slice hands out only by shared reference or by copy (a board entry's story and pending flag, a filter, an intent or operation, a picker, the ticker) hold no invariant and MAY have public fields; fields that carry a slice's own bookkeeping (such as a board entry's request sequence) MUST stay private. Each owner MUST receive only the narrow read-only inputs it needs, and views MAY borrow state read-only. The core MUST be limited to composition, input mode and key routing, running effects, transport and envelope classification of replies (a sign-out on the server's own 401, a refetch on a conflict, the wording of transport errors) and the one-shot actions; move refusals and the override workflow MUST stay in the moves slice.
- **R3** Only the main loop MAY own or change TUI state; background work (HTTP requests, timers, the event stream) MUST report its result as a message and MUST NOT hold a reference to state.
- **R4** Update functions MUST be free of I/O and MUST return effects as data: an enum the runtime interprets (a request, a timer, connecting or disconnecting the stream, opening a URL, copying text), never a future; a request's continuation is a pure function from its completion to a message. This is what makes every behaviour testable without a terminal or a server.
- **R5** Writes MUST NOT be applied optimistically: the board shows a pending mark until the server replies, and the reply, a refetch or a conflict decides what is shown. Whether data is current MUST be decided by request order, not by story version alone (F2): the core numbers every request when it is sent; a board read sent before a write completed or before an activity entry arrived MUST NOT replace the displayed snapshot and MUST be followed by a fresh read; a write reply MUST NOT overwrite data from a read sent after that write; a lower story version MUST NOT replace a higher one; and a timer or reply whose generation is no longer current for its key MUST be dropped.
- **R6** The TUI MUST reach the planner only through the public HTTP API and the event stream, with wire types from `reeve-protocol`, and MUST NOT keep planning data beyond the running process; it MAY persist the user's own interface preferences.
- **R7** `reeve tui` MUST authenticate only with the stored login and MUST NOT use `REEVE_TOKEN` as a credential (it MAY note the variable's presence); with no stored login it MUST exit before touching the terminal and say to run `reeve auth login`, even when `REEVE_TOKEN` is set. It MUST verify the token's kind with `GET /me` and run read-only when the token is not interactive. This narrows kb/ADR-0008.R3 for `reeve tui` alone: every other command keeps taking an automation token from the environment. The server derives provenance from the token's kind (F6), so this rule is not the security boundary: it keeps the surface where a person overrides from appearing able to override when its token cannot.
- **R8** Every write MUST end in exactly one completion delivered to the board, whatever its outcome and whether or not the workflow that started it is still open: a reply the server committed, or an unknown outcome (a timeout, a lost reply, a server error), MUST release the story's pending mark, mark its readiness pending and trigger a fresh read; a refusal or any other failure MUST release the pending mark and change nothing else.
- **R9** Domain decisions MUST stay with the server: the TUI MUST NOT compute readiness, decide whether a transition or an override is allowed, or authorize anything; workflow slices collect intent and present the server's answer. The TUI MAY hide a choice the server is known to refuse (completed from a state that is not active, an override under an automation token) as presentation, but the server's answer always decides.
- **R10** On every activity entry and every (re)connect the TUI MUST refetch the whole board, 500 ms after the last change and at most 5 s after the first of a burst, without filtering by entity, as the web client does (F7); a change to either client's policy MUST be made in both or recorded as a later observation here.

An example, with the owner of each step: `L` on a story that is not ready makes moves record an intent (the key, origin and captured version) and a 400 ms timer. When the timer fires, moves sends the move with the captured version and tells the board, which sets the story's pending mark. The server refuses with readiness reasons: the completion clears the pending mark on the board, and moves opens the override prompt bound to that operation. The reason the user types goes back with the same version, and the board sets the pending mark again. The server commits: the completion clears the mark, applies the returned story fields, marks its readiness pending and asks for a fresh read, before which every earlier read is discarded; the activity entry then arrives over the stream (live), discards any read still in flight and triggers the debounced refetch; the ticker shows the move with `(override)`. Had the user closed the prompt or moved on, the completion would still have reached the board.

## Consequences
- The runner's TUI code is reused for its key-binding table, help and widget code, not its `App` or action handler; the optimistic-merge layer is not carried over.
- A write's result is visible only after a round trip, so on a slow link a move feels slower than an optimistic one would.
- Message enums, generations and routing add boilerplate a direct-mutation design does not have, and a feature that spans slices needs a message between them rather than a function call. Private slice state needs accessors for the views and tests of other modules.
- `reeve tui` resolves its token differently from every other command; a user who relies on `REEVE_TOKEN` alone must run `reeve auth login` before the TUI can write.
- Some refusals the server would make are anticipated in the interface (choices hidden), so a rule change on the server must be followed in the TUI's presentation; until it is, the server's refusal shows and nothing breaks.
- Swapping ratatui or the slice structure later means rewriting every view and update function.

## Assumptions and invalidation triggers
- *Assumes a write round trip is short enough that a pending mark reads as instant.* Trigger: moves routinely take more than about half a second for users of the TUI ⇒ supersede R5.
- *Assumes the TUI stays a human surface.* Trigger: a need to run the TUI unattended or as an agent ⇒ supersede R7.
- *Assumes the TUI stays a thin board client.* Trigger: the TUI takes on planning, decisions or briefs, or embeds sessions (where the runner needed shared, locked state) ⇒ revisit the slice set, R3 and R6.

## Later observations
<!-- append-only, accepted records only; newest last -->
- 2026-11-09 (kb/ADR-0015): kb/ADR-0008 is superseded by kb/ADR-0015; the token-kind rule R7 narrows is now kb/ADR-0015 R7, unchanged in substance. No trigger fired; the decision holds.
- 2026-11-09 (kb/ADR-0017): R2's slices grow by `urgent` (the optional-reason prompt that marks a story urgent and owns that write) and its one-shots by clearing a story's urgent mark; both writes take the write path (R5, R8). The TUI, like the web, does not offer marking a done story; the server accepts such a mark, so this is presentation (R9). No trigger fired; the decision holds.

## See also
- kb/ADR-0001 (public API only; R6 restates its R2 for the TUI)
- kb/ADR-0006 (Rust stack; F2, the CLI and TUI depend only on the protocol crate)
- kb/ADR-0008 (token kinds; R3, which R7 narrows for `reeve tui`)
- kb/ADR-0010 (crate architecture rules; they govern crates, not the TUI's modules)
- kb/ADR-0003 (readiness and the override the TUI prompts for)
