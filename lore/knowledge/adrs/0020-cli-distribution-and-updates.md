---
id: kb/ADR-0020
title: "ADR-0020: The Reeve server distributes the CLI it was released with, and the CLI asks to be updated when they differ"
description: "Each Reeve server serves, without sign-in, the `reeve` CLI binaries built for its release, verified by SHA-256 over HTTPS, and the CLI tells a person at a terminal to run `reeve update` when its CLI version differs from the server's, because one install line and an exact match with each instance beat a single package channel that every instance shares and that drifts from each of them."
tags: [adr, reeve, cli, deploy, security]
status: proposed
date: 2026-11-10
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: one-way
blast_radius: shared
sensitivity: [security]
scope: ["reeve:crates/reeve/**", "reeve:crates/reeve-server/src/cli.rs", "reeve:crates/reeve-server/src/routes/**", "reeve:crates/reeve-server/src/state.rs", "reeve:Dockerfile", "reeve:deploy/cli/**", "reeve:deploy/README.md", "reeve:.github/workflows/cli.yml", "reeve:.github/workflows/release.yml", "reeve:scripts/cli-*.sh", "reeve:scripts/tests/**"]
supersedes: []
superseded_by:
depends_on: [kb/ADR-0005, kb/ADR-0006, kb/ADR-0015, kb/ADR-0016]
related: [kb/ADR-0010, kb/ADR-0011, kb/ADR-0019]
implements: []
rfc:
---

# ADR-0020: The Reeve server distributes the CLI it was released with, and the CLI asks to be updated when they differ

## Status
Proposed 2026-11-10.

## Context
People install the `reeve` CLI today by building it from a checkout, or with `cargo install` from the public repository. Both need a Rust toolchain and a full compile, and both leave each person on whatever commit they last built. Lorekeeper skills shell out to the same binary under an automation token and detect the planner by `reeve --version` (kb/ADR-0005), and the TUI is used against a live instance, so a client that drifts from its server meets API changes it was not built for.

Every push to `main` that changes code publishes one server image per commit and tells subscribed repositories about it; deploying is each operator's business, outside this repository. Each team runs its own instance (kb/ADR-0016) and chooses when to deploy which release, so instances run different releases at the same time, and an operator may go back to an earlier one. A push that changes only documentation publishes nothing. The image build already compiles the Linux x86_64 CLI next to the server. The team uses Linux (x86_64 and arm64), macOS (Apple Silicon and Intel) and Windows.

The instance sits behind sign-in. Its unauthenticated surfaces today are the health check, the OpenAPI document, the sign-in routes, the HMAC-checked webhook and the static client; none of them serves software. A first-time installer has no login yet, because logging in is done with the CLI. TLS is terminated at the operator's reverse proxy, and an operator may put a CDN in front of it, as the team's own instance does with Cloudflare.

In this record, the **CLI version** is the commit id of the last commit that changed one of the CLI's build inputs; it is not the semver `reeve --version` prints.

## Facts relied on
- F1 `release.yml` publishes `ghcr.io/mindful-stack/reeve:<sha>` on every push to `main` except documentation-only pushes, then starts each subscriber's release workflow; it deploys nothing — source: `.github/workflows/release.yml` (`on.push.paths-ignore`, `publish`, `notify`), `deploy/README.md` ("Subscribe to releases").
- F2 The image build compiles `-p reeve` with the server and copies the Linux x86_64 binary into the image — source: `Dockerfile` (the `build` stage, `COPY --from=build /out/reeve`).
- F3 The unauthenticated routes are `/health`, `/api/openapi.json`, `/auth/*`, the HMAC-checked `/webhooks/github` and the static client; everything under `/api/v1` requires a caller — source: `crates/reeve-server/src/routes/mod.rs` (`app`), `crates/reeve-server/src/auth/mod.rs`.
- F4 The deployment guide rate-limits `/auth/login` per client IP at the proxy — source: `deploy/README.md` (`limit_req_zone … zone=reeve_login`).
- F5 `reeve --version` prints `reeve-planner <semver>` from the workspace version, and skills detect the planner by that prefix — source: `crates/reeve/src/lib.rs` (`#[command(name = "reeve-planner", version)]`), kb/ADR-0005.R1.
- F6 Skills run `reeve` with standard error captured, not on a terminal — source: Lorekeeper `scripts/reeve-detect.js` (`stdio: ['ignore', 'pipe', 'pipe']`).
- F7 The server always resolves the URL users reach it at — source: `crates/reeve-server/src/config.rs` (`public_url()`).
- F8 The reference deployment publishes the server on loopback and terminates TLS at the operator's reverse proxy — source: `deploy/README.md` (step 3 of the quick start, and the proxy section).

## Considered options
- **GitHub Releases installed through mise's `github:` backend** — a widely known channel with `mise upgrade`, no server change, and no token, since the repository is public. But it has one newest release for every instance, while instances run different releases: a person whose server is behind or ahead must find and pin the matching release by hand, and nothing tells them when it stops matching.
- **`cargo install --git` from `main`** (directly or through mise's `cargo:` backend) — no CI work at all. Each person needs Rust, every update is a full compile, `main` is rarely what their instance runs, and a named branch never looks upgradable.
- **The server serves the CLI built for its release, and the CLI asks to be updated** — one line installs it from the instance the person already uses, needs no GitHub access, and the client is the one the running server was built with.

Within the chosen option, updating automatically was rejected because a binary swapped between two calls of one skill run breaks kb/ADR-0005.R3's contract; updating only forwards was rejected because it needs a version order and leaves a client ahead of a server that runs an older release; signed binaries were rejected because every self-hosting team would need its own key to guard and rotate, while the CLI already trusts the same server with its token over HTTPS; cross-compiling every target on Linux was rejected because the macOS and Windows binaries would ship without ever having run.

## Decision
Daniel Thyselius has each Reeve server serve, without sign-in, the CLI binaries built for its release, and has the CLI tell a person at a terminal to run `reeve update` whenever its CLI version differs from the server's, because one install line from the instance the team already uses, and an exact match with the server it talks to, beat a channel that needs repository access and drifts.

- **R1** CI MUST build and publish binaries only for a CLI version it has not published before, MUST never overwrite a published CLI version, and a server image MUST carry the binaries of its own commit's CLI version, checked before the image is pushed.
- **R2** CI MUST build each target natively on its own operating system (Linux x86_64 and arm64, macOS arm64 and x86_64, Windows x86_64) with one pinned compiler release, and MUST run each binary before publishing it.
- **R3** The server MUST serve the CLI version, the binaries, a SHA-256 checksum file and the install scripts under `/cli/` without authentication, and MUST NOT accept or require a token there; no other route MAY become unauthenticated under this record. The deployment guide MUST rate-limit `/cli/` per client IP at the proxy, as it does `/auth/login`.
- **R4** The install scripts MUST take the server URL from the instance's own public URL, never from a value compiled into the CLI or the scripts.
- **R5** The install scripts and `reeve update` MUST download only over HTTPS to the configured server URL (plain HTTP only to a loopback address), MUST NOT follow redirects, and MUST verify a binary against the server's SHA-256 checksum before installing it.
- **R6** The CLI MUST NOT replace itself without the user running `reeve update`. When the server's CLI version differs from its own, it MUST say so at most once per process, and only to a person: on a standard error that is a terminal, without `--json`, or in the TUI's header. It MUST NOT change standard output, `--json` output or exit statuses (kb/ADR-0005.R3). `reeve --version` MUST stay `reeve-planner <semver>` and MUST NOT touch the network (kb/ADR-0005.R1).
- **R7** "Up to date" MUST mean equal to the server's CLI version: `reeve update` installs the server's binary even when it is older than the running one.

## Consequences
- Installing is one `curl … | sh` (or `irm … | iex`) line, then `reeve auth login`: no Rust, no mise, no GitHub access.
- Anyone who can reach the server can download the CLI and read its CLI version, a commit id. Neither holds team data, but the instance is no longer closed to anonymous requests beyond sign-in and the static client.
- Authenticity rests on HTTPS to the team's server, and therefore on every party that terminates TLS in front of it (the proxy, and any CDN in front of it): any of them can replace a binary and its checksum together.
- A push that changes the CLI adds five native builds, and the slowest of them to the time before the image is published and subscribers are told; a push that does not change it adds one short lookup.
- A dependency bump in the shared lock file that only the server uses still produces a new CLI version and an update notice.
- Skills and scripts never see the notice; they compare versions through `reeve auth status --json` if they need to.
- People who install the CLI some other way (a local build, cargo, mise) see the notice and must update that way.
- Windows becomes a supported platform for the CLI and TUI, which needs a Windows configuration path and browser opener it does not have today.

## Assumptions and invalidation triggers
- *Assumes the team trusts its server, its proxy and its TLS terminator as much as the binary they serve.* Trigger: a binary must stay trustworthy when one of them is compromised, or binaries are served through any other party the team does not control ⇒ supersede R5 with signed binaries.
- *Assumes CLI changes are infrequent enough that native builds on every CLI change are affordable.* Trigger: in a calendar month, CLI builds delay publishing by more than fifteen minutes on three or more pushes, as the release workflow's run durations show ⇒ revisit R2.
- *Assumes people update when told.* Trigger: outdated clients cause a reported failure against the server more than once ⇒ revisit R6 (an update the TUI starts itself, or the server refusing clients older than a minimum).
- *Assumes a person uses one instance (kb/ADR-0016).* Trigger: one person's CLI must work against several instances running different releases ⇒ supersede R7.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0005 (Lorekeeper detects the planner by `reeve --version` and relies on distinct exit statuses; R6 preserves both)
- kb/ADR-0006 (one server binary serves the API and the client, and now the CLI; no API type changes)
- kb/ADR-0015 (sign-in and token kinds; R3 adds `/cli/` to the unauthenticated surfaces and reads no token there)
- kb/ADR-0016 (self-hosted, one team per instance; R4 keeps the instance's URL out of the binaries, and no new configuration or container)
- kb/ADR-0010 (CLI distribution is file serving in the composition root, not a context)
- kb/ADR-0011 (the TUI shows the notice as a header note decided before it starts; its login rules are unchanged)
- kb/ADR-0019 (proposed: `/privacy` would be another unauthenticated route, under its own record)
