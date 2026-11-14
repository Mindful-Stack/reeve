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

## See also
- kb/ADR-0005 (the CLI is the agent surface; R5 allows an MCP server that wraps the same commands)
- kb/ADR-0015 (sign-in, the allowlist and the two token kinds; unchanged by this record)
- kb/ADR-0020 (the unauthenticated routes before this record; its CLI routes ignore any token)
- kb/ADR-0027 (only an interactive user overrides the gate; R10 keeps that out of the chat)
- kb/ADR-0023 (attachments pass the normal access check; no shared URL for a chat client)
- kb/ADR-0019 (proposed: the privacy statement)
