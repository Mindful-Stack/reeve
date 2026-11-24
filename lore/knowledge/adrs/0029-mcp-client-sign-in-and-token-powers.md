---
id: kb/ADR-0029
title: "ADR-0029: MCP clients sign in through Reeve's own authorization server and act with interactive tokens bound to the MCP endpoint"
description: "We decided that the Reeve server is the OAuth authorization server for its own remote MCP endpoint, in front of the existing sign-in, and issues interactive tokens that work only at that endpoint, for a named set of tools that never includes tokens, access, import, export or overriding the gate, because a person is in the conversation and the chat clients reach only a remote server."
tags: [adr, reeve, identity, tokens, auth, security, cli]
status: accepted
date: 2026-11-14
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: one-way
blast_radius: shared
sensitivity: [security, privacy]
scope: ["reeve:crates/reeve-identity/**", "reeve:crates/reeve-activity/**", "reeve:crates/reeve-server/src/auth/**", "reeve:crates/reeve-server/src/routes/**", "reeve:crates/reeve-server/src/mcp/**", "reeve:crates/reeve-server/src/state.rs", "reeve:crates/reeve-protocol/**", "reeve:deploy/README.md"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0005, kb/ADR-0006, kb/ADR-0010, kb/ADR-0015, kb/ADR-0020]
related: [kb/ADR-0016, kb/ADR-0019, kb/ADR-0023, kb/ADR-0027]
implements: []
rfc:
---

# ADR-0029: MCP clients sign in through Reeve's own authorization server and act with interactive tokens bound to the MCP endpoint

## Status
Proposed 2026-11-14.
Accepted 2026-11-14 by Daniel Thyselius.

## Context
People reach Reeve through the `reeve` CLI, the TUI and the web client. Agents reach it by running the CLI (kb/ADR-0005), which chose the CLI over an MCP server and allowed one later if it wraps the same commands. Part of the team works only in claude.ai or ChatGPT. Those clients cannot run a local program: they call a remote server over HTTPS from their provider's cloud, and sign each person in with OAuth as the MCP authorization specification describes.

Reeve authenticates people through Google OpenID Connect and an allowlist (kb/ADR-0015). It issues two kinds of token, interactive and automation; only an interactive caller may override the readiness gate, issue tokens or change access. A web session is itself an interactive token. Tokens carry no client and no audience, so any live token is accepted on every route, and an activity entry records who acted and through which kind of token, not which token or client. Today the health check, the OpenAPI document, the sign-in routes, the webhook and the static client answer without a caller; kb/ADR-0020 adds the CLI downloads, accepted and not yet built, and allows no further one under itself.

A chat model reads story text and comments that other people wrote, so text in the planner can try to steer what the model does with the person's token. Every user of one chat product shares that product's callback address, so a person can start a connection in their own chat and send its approval link to someone else.

## Facts relied on
- F1 Part of the team works only in claude.ai or ChatGPT — source: Daniel Thyselius, 2026-11-14.
- F2 Both clients connect only to a remote MCP server over HTTPS, from the provider's cloud, and support OAuth with Client ID Metadata Documents and dynamic client registration — source: https://claude.com/docs/connectors/building/authentication.md, https://developers.openai.com/api/docs/guides/custom-mcp-server.md (read 2026-11-14).
- F3 The MCP authorization specification, revision 2026-07-28, requires protected resource metadata, a `401` that points at it, a `resource` parameter naming the server, and that a server accepts only tokens issued for itself — source: https://modelcontextprotocol.io/specification/latest/basic/authorization.
- F4 Claude always sends S256 PKCE, redirects to `https://claude.ai/api/mcp/auth_callback`, expects refresh tokens for public clients to rotate, waits 10 seconds for the OAuth endpoints and calls from `160.79.104.0/21` — source: the Claude page in F2 (*PKCE and requested scopes*, *Callback URLs*, *Token refresh*, *Endpoint latency*, *Network reference*).
- F5 A token is `reeve_` followed by 24 random bytes in hex, stored as its SHA-256, with `kind` constrained to `interactive` or `automation` and no client or audience column — source: `new_secret` and `hash_secret` in `crates/reeve-identity/src/domain.rs`; `crates/reeve-identity/migrations/2001_users_tokens.sql`.
- F6 Signing in on the web issues an interactive token labelled "web session" — source: `SESSION_LABEL` and `sign_in` in `crates/reeve-identity/src/handler.rs`.
- F7 A request's context holds the actor, the kind of token and the time, and nothing about the client — source: `Caller::ctx` in `crates/reeve-server/src/auth/mod.rs`.
- F8 The request that moves a story carries an optional override reason — source: `MoveStory.override_reason` in `crates/reeve-protocol/src/requests.rs`.
- F9 A cookie-authenticated write needs the header `X-Reeve-Client: web`, which an HTML form cannot send — source: `crates/reeve-server/src/auth/mod.rs`.
- F10 The only rate limit in the deployment guide is on `/auth/login`, per client IP at the proxy, and the proxy forwards every other path — source: `deploy/README.md` (`limit_req_zone … zone=reeve_login`, `location /`).
- F11 Pending sign-ins are held in the server process, with a cap and a ten-minute life — source: `logins` in `crates/reeve-server/src/state.rs`.

## Considered options
- **An MCP server over standard input in the CLI** — no new route and it reuses `reeve auth login`; claude.ai and ChatGPT cannot reach it (F2).
- **A remote endpoint where each person pastes a token** — no authorization server to build; neither client offers a per-person pasted token to every team, and a pasted long-lived secret is easy to leak.
- **Google as the authorization server** — no token code in Reeve; Google does not issue tokens bound to Reeve's endpoint or accept these clients' registration (F3).
- **A separate authorization server product in front of Reeve** — proven code; it is a second container or a vendor (kb/ADR-0016 R3), and the allowlist would be judged in two places.
- **Reeve as authorization server, issuing automation tokens** — the gate cannot be overridden by construction; every write would be logged as automation although a person made it.
- **Reeve as authorization server, issuing a third kind of token** — exactly the powers wanted; it supersedes kb/ADR-0015 R7 and touches every place that reads the kind.
- **Reeve as authorization server, issuing interactive tokens bound to the MCP endpoint** — the kind says what is true, a person is in the conversation, and the limit is in where the token works and which tools exist.

## Decision
We decided that the Reeve server is the OAuth authorization server for its own remote MCP endpoint, in front of the existing sign-in, and issues interactive tokens that work only at that endpoint, for a named set of tools that never includes tokens, access, import, export or overriding the gate, because a person is in the conversation and the chat clients that person uses reach only a remote server.

- **R1** The server binary MUST serve the MCP endpoint at one path over streaming HTTP. Each tool MUST run the same handler and the same validation as an API route and MUST correspond to a `reeve` CLI command (kb/ADR-0005 R5); a tool MUST NOT do what no CLI command does.
- **R2** The server MUST publish protected resource metadata and authorization server metadata for the endpoint and MUST answer a call without a valid token with `401` naming that metadata. On an instance without sign-in configured, the endpoint and its metadata MUST answer 404.
- **R3** A client MUST identify itself by a Client ID Metadata Document or by dynamic registration. The server MUST fetch a metadata document only over HTTPS from a public address, without following redirects and within a size and time limit, and MUST require the document's client id to equal its URL. A redirect address MUST match exactly both an entry in the instance's configuration and one the client registered. Stored registrations MUST be capped, and unused ones MUST expire.
- **R4** The authorize step MUST require S256 PKCE and the `resource` of the endpoint, MUST send the person through the sign-in of kb/ADR-0015, and MUST hold the request in server state, not in a URL. The approval page MUST show the signed-in email, the client, the host it redirects to and whether it may write, and MUST NOT be framed. Approving MUST be a POST with a one-time value bound to the session and the pending request; a GET MUST NOT approve. A code MUST be single-use, live at most one minute, and be bound to the user, client, redirect address, challenge and resource.
- **R5** A token issued this way MUST be an interactive token (kb/ADR-0015 R7) recorded with its client, its audience and its scopes. An access token MUST live at most one hour. A refresh token MUST be stored as a hash, MUST rotate on use, MUST stop working 30 days after the approval, and MUST re-check the person's access when used; using a rotated refresh token again MUST revoke the whole connection.
- **R6** The MCP endpoint MUST accept only tokens issued for it, as a bearer header, and MUST ignore cookies. Every other authenticated surface (the API, the event stream, attachments, the session cookie) MUST refuse such a token, and a refresh token MUST authenticate nowhere.
- **R7** There MUST be two scopes, `read` and `write`; a tool that changes anything MUST require `write`.
- **R8** Every MCP call, and every batch of a stream held open, MUST pass the access check of kb/ADR-0015 R5.
- **R9** A connection MUST be listed once among its owner's tokens with the client's name, and its owner MUST be able to revoke it; disabling the user MUST revoke it. Approving and revoking MUST each write an activity entry, and every write made through the endpoint MUST record the client in its activity entry.
- **R10** The tools MUST be limited to: search; list and read stories, epics, initiatives, required decisions and the sprint; create and edit a story; add a comment; move a story. A tool outside this set MUST NOT be added without a later observation on this record that names it. A tool MUST NOT issue a token, change sign-in access or admin grants, import, export, or override the readiness gate; the move tool's input MUST NOT carry an override reason, and a call that sends one MUST be refused.
- **R11** The metadata, registration, authorize and token routes are the only routes that become unauthenticated under this record; approving MUST require a session. The deployment guide MUST limit authorize per client IP and the other routes at a rate sized for a provider's shared addresses, and the server MUST cap pending requests and codes.
- **R12** Tool inputs MUST be types in `reeve-protocol`, a type of their own where a field is withheld, with schemas generated from them and snapshot-tested (kb/ADR-0006 R3).

## Consequences
- People who work only in a chat client read and change their work in Reeve as themselves; their writes are logged as theirs and name the client.
- Reeve now contains an authorization server: codes, client registration, refresh rotation and an approval page are security-critical code that a small team maintains, and a flaw in it is a way into every instance.
- Text another person wrote into a story or comment can steer the model into any write a tool allows. The client's own confirmation prompt is the only check on that; R6 and R10 keep the worst actions out of reach.
- A person can be sent an approval link for someone else's chat; the approval page is the only place that can be noticed.
- What a person reads through a connected client is sent to that client's provider under the person's own account with it. The repository serves no privacy statement of its own (kb/ADR-0019, proposed), so an operator's statement must say so before the endpoint goes live on their instance.
- Anyone who can reach the instance can create client registrations and start sign-ins; the limits bound the rate and the stored rows, and a flood can still lock real users out for a while.
- A connection ends after 30 days and the person approves again.
- Recording the client is a change to the activity entry and to the token list on the wire.

## Assumptions and invalidation triggers
- *Assumes a person reads and approves what the chat client does with Reeve.* Trigger: a client runs Reeve tools with nobody in the conversation (a scheduled or unattended agent) ⇒ supersede R5: those tokens are automation.
- *Assumes confirmation prompts are enough against planted instructions.* Trigger: one case, found in the activity log's client entries, of a write a person did not intend, caused by text in the planner ⇒ supersede R7 or R10 with narrower powers.
- *Assumes the clients are claude.ai and ChatGPT, with a short fixed list of redirect addresses.* Trigger: a client needs a loopback or per-install redirect address ⇒ revisit R3.
- *Assumes one server process per instance (kb/ADR-0016), so pending requests and codes may live in it (F11).* Trigger: an instance runs two processes ⇒ revisit R4.
- *Assumes dynamic registration is still needed.* Trigger: both clients identify themselves by metadata document only ⇒ record it as an observation and drop registration.

## Later observations
<!-- append-only, accepted records only; newest last -->
- 2026-11-14 (GRA-653 sign-in design): two things the design adds or leaves out, none against a rule.
  We deferred dynamic registration; we did not drop it. Both target clients document support for Client ID
  Metadata Documents (source: https://claude.com/docs/connectors/building/authentication.md,
  https://developers.openai.com/api/docs/guides/custom-mcp-server.md, read 2026-11-14), so the first build
  identifies a client by its metadata document only, and R3's "or by dynamic registration" and its cap and expiry
  have nothing to apply to yet. The registration trigger has not fired: how ChatGPT identifies itself is
  unverified until its trial, and the parked registration story is decided then. We fetch client documents only
  from hosts the instance lists, which narrows R3. The decision holds.
- 2026-11-14 (GRA-669, connections): an admin may list and revoke any person's connections, beside the owner R9
  names. An admin may do either with either kind of token. The two permissions differ. Revoking only reduces access,
  as disabling a user does (kb/ADR-0015 R6). Listing gives an admin sight of other people's connection metadata:
  which app each person connected, by the client's id, name and host, with what scope, when it was approved and last
  refreshed, and, for an ended connection, when and why it ended. That is new
  visibility, accepted because an admin who must cut a connection has to find it first, and because the
  `connection_approved` and `connection_revoked` activity entries already show every signed-in person which app
  someone connected. No trigger fired; the decision holds.
- 2026-11-15 (GRA-668, dynamic registration): we built the registration that R3 names and the first observation
  deferred. It is off by default, an instance turns it on in its configuration, and a client that registered
  itself is refused while it is off.
  We built it for the story's aim: connecting hosted apps that publish no metadata document and register
  themselves when a person adds a connector. That such apps exist and behave so is an expectation taken from
  their documentation and from third-party guides; we did not verify it against any such app.
  R3's cap and expiry are met so: at most 1,000 registrations are stored; one that no person has approved a
  connection for expires 24 hours after it was made; one that was approved expires 90 days after its last
  approval, which is after every connection made with it has left the lists; when the store is full the oldest
  never-approved registration is dropped to make room (one being approved at that moment is passed over), and a
  registration is refused only when every stored one has been approved. An app whose registration was dropped or
  has expired holds a client id that no longer works, and its person adds the connector again. R3's exact match
  on both lists holds as for a document: a registered client can register only addresses in the instance's
  configuration, and at every authorize its address must still be there and be one it registered, compared as
  text. No secret is issued or stored.
  Everything a registered client says about itself is its own claim, its name included, so people are shown it by
  the host of its redirect address: the approval page (R4) names it by that host and shows its name as what the
  app calls itself; the host is kept with the connection, which is listed by it with the name (R9); the
  approval's entry holds the client id, the host and the name; and every write made through it records that host
  as the client (R9). With registration on, the list of client hosts no longer limits who may start a connect to
  a listed redirect address; the deployment guide tells operators to list only a provider's own callback.
  The record's trigger about registration ("both clients identify themselves by metadata document only ⇒ record
  it as an observation and drop registration") no longer matches what is built: registration now serves apps
  other than the two clients. If that trigger fires, or if no app we operate an instance for has needed
  registration by 2027-05-15, the record should be replaced rather than observed again. No trigger has fired.
- 2026-11-23 (reeve#61): kb/ADR-0020, which this record depends on and whose CLI downloads the Context counts
  among the routes that answer without a caller, is superseded by kb/ADR-0030. The successor keeps what this record
  relies on: the CLI downloads are still served without sign-in, ignore any token or cookie, and allow no further
  unauthenticated route; read kb/ADR-0020 in the Context and See also as kb/ADR-0030. No trigger fired; the
  decision holds.
- 2026-11-24 (kb/ADR-0031, GRA-877, https://github.com/Mindful-Stack/reeve/pull/73): the read tools `get_story`,
  `get_epic` and `get_initiative` now show the item's item links, bounded by the adapter, with omitted counts
  (`links_omitted`). No tool was added and no input changed, so R10's set stands; item links are written through
  the API and the CLI only until an observation here names a tool for it. Linked items' titles are more text a chat
  model reads from the planner. No trigger fired; the decision holds.
- 2026-11-24 (kb/ADR-0031, GRA-878, https://github.com/Mindful-Stack/reeve/pull/74): the read tools `get_story`,
  `get_epic` and `get_initiative` now also show the item's web links, bounded by the adapter, with omitted counts
  (`web_links_omitted`); a web link is sent whole or not at all, and a URL is never cut. Their titles and URLs are
  free text written by people that a chat model now reads, which the tool descriptions say and the operator's
  privacy statement must cover (`deploy/README.md`). No tool was added, so R10's set stands. No trigger fired; the
  decision holds.
- 2026-11-24 (GRA-901, GRA-902, https://github.com/Mindful-Stack/reeve/pull/79): R10's set grows by eight tools, named here as R10
  requires. Four come with this pull request: `create_epic`, `edit_epic`, `create_initiative` and `edit_initiative`.
  Four come with the pull request after it (GRA-902) and do not exist until it merges: `add_item_link`,
  `remove_item_link`, `add_web_link` and `remove_web_link`. Each mirrors one CLI command and runs the action its API
  route runs (R1): `reeve epic create`, `reeve epic edit`, `reeve initiative create`, `reeve initiative edit`,
  `reeve link add` and `reeve link rm` for an item link, and `reeve link add --url` and `reeve link rm --url` for a
  web link, which the tool removes by its id as the route does, where the CLI finds that id by the URL. Each needs a
  connection with the `write` scope (R7), counts against the person's limit on changes, and records the client in
  every activity entry it writes (R9). Each input is a type of its own (R12); the epic and initiative inputs
  withhold the external reference and attachments, as the story tools do.
  The conditions these eight were named on, each held by a test: no tool marks or reopens discovery
  (`no_epic_or_initiative_tool_can_mark_or_reopen_discovery`), adds or removes a blocker
  (`the_link_tools_gate_nothing_and_no_tool_touches_a_blocker`, which comes with the four link tools) or carries an
  override reason (`no_tool_takes_an_override_reason_or_another_withheld_field`, which from this pull request on also
  refuses an input that names a discovery mark or a blocker; `move_story_never_overrides_the_gate`). Not in the set,
  and needing an observation of their own under R10: promises, urgent marks, and writing a required decision, a
  sprint or a goal.
  We decided that `edit_epic` keeps what `reeve epic edit` can do: move an epic to another initiative or out of one.
  That changes whose discovery mark and required decisions its stories wait on, as `edit_story` already can by
  putting a story in another epic. It is an edit, not an override: readiness is derived (kb/ADR-0027 R1), no tool
  carries an override reason, and the epic's `edited` entry names the client and the initiative it left and joined.
  A story's `edited` entry does not yet name the epic it left and joined (GRA-905).
  We accepted that a connection approved for "read and change" before either pull request is deployed gains that
  pull request's tools without a new approval: `write` is R7's scope for everything that changes, the approval page
  named fewer things when it was approved (until this pull request "create, edit, comment on and move stories"),
  and R5 ends every such connection within 30 days of its approval.
  Text in the planner can now steer a model into creating an epic or an initiative, replacing a title or a brief (an
  `edited` entry keeps no earlier text), moving an epic, or adding or removing an item link or a web link, a URL of
  that text's choosing included, under the person's name; the client's confirmation and the limit on changes remain
  the only checks. Briefs, and web link titles and URLs, are text a chat model can now write as well as read, which
  the operator's privacy statement must cover (`deploy/README.md`). No trigger fired; the decision holds.

## See also
- kb/ADR-0005 (the CLI is the agent surface; R5 allows an MCP server that wraps the same commands)
- kb/ADR-0015 (sign-in, the allowlist and the two token kinds; unchanged by this record)
- kb/ADR-0020 (the unauthenticated routes before this record; its CLI routes ignore any token)
- kb/ADR-0027 (only an interactive user overrides the gate; R10 keeps that out of the chat)
- kb/ADR-0023 (attachments pass the normal access check; no shared URL for a chat client)
- kb/ADR-0019 (proposed: the privacy statement)
