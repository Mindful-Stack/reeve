---
id: kb/ADR-0020
title: "ADR-0020: The Reeve server distributes the CLI it was released with, and the CLI asks to be updated when they differ"
description: "Each Reeve server with a configured public URL serves, without sign-in, the `reeve` CLI binaries built for its release, verified by SHA-256 over HTTPS, and the CLI tells a person at a terminal to run `reeve update` when its CLI version differs from the server's, because one install line and an exact match with each instance beat a channel that every instance shares and that drifts from each of them."
tags: [adr, reeve, cli, deploy, security]
status: accepted
date: 2026-11-13
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
Proposed 2026-11-13.
Accepted 2026-11-13 by Daniel Thyselius.

## Context
People install the `reeve` CLI today by building it from a checkout, or with `cargo install` from the public repository. Both need a Rust toolchain and a full compile, and both leave each person on whatever commit they last built. Lorekeeper skills shell out to the same binary under an automation token and detect the planner by `reeve --version` (kb/ADR-0005), and the TUI is used against a live instance, so a client that drifts from its server meets API changes it was not built for.

Every push to `main` that changes code publishes one server image per commit and tells subscribed repositories about it; deploying is each operator's business, outside this repository. Each team runs its own instance (kb/ADR-0016) and chooses when to deploy which release, so instances run different releases at the same time, and an operator may go back to an earlier one. A push that changes only documentation publishes nothing. The release build already compiles the Linux x86_64 CLI next to the server. The team uses Linux (x86_64 and arm64), macOS (Apple Silicon and Intel) and Windows.

The instance sits behind sign-in. Its unauthenticated surfaces today are the health check, the OpenAPI document, the sign-in routes, the HMAC-checked webhook and the static web client; they serve the browser's code but no native program. The CLI can store a token only once it is installed, so a first-time installer has none. TLS is terminated at the operator's reverse proxy, and an operator may put a CDN in front of it, as the team's own instance does with Cloudflare.

In this record, the **CLI version** is the commit id of the last commit that changed one of the CLI's build inputs, as `scripts/cli-version.sh` lists them; it is not the semver `reeve --version` prints. A **stamped** CLI is one built with its CLI version; a **dev build** is one built without it.

## Facts relied on
- F1 `release.yml` runs on every push to `main` that changes a path outside the ones it ignores (Markdown, `docs/`, `lore/`), publishes `ghcr.io/mindful-stack/reeve:<sha>`, and after a successful publish tries, best effort, to start each subscriber's release workflow; it deploys nothing — source: `.github/workflows/release.yml` (`on.push.paths-ignore`, `publish`, `notify`), `deploy/README.md` ("Subscribe to releases").
- F2 The release job's image build compiles `-p reeve` with the server and copies that Linux x86_64 binary into the image — source: `Dockerfile` (the `build` stage, `COPY --from=build /out/reeve`), `.github/workflows/release.yml` (`runs-on: ubuntu-24.04`).
- F3 Every registered `/api/v1` operation requires a caller; the other routes are `/health`, `/api/openapi.json`, `/auth/*`, the HMAC-checked `/webhooks/github` and the static client — source: `crates/reeve-server/src/routes/mod.rs` (`app`), `crates/reeve-server/src/auth/mod.rs` (`Caller`), `crates/reeve-server/src/routes/webhooks.rs`.
- F4 The deployment guide documents a per-client-IP limit on `/auth/login` at the proxy — source: `deploy/README.md` (`limit_req_zone … zone=reeve_login`).
- F5 `reeve --version` prints `reeve-planner <semver>` from the workspace version, and skills detect the planner by that prefix — source: `crates/reeve/src/lib.rs` (`#[command(name = "reeve-planner", …)]`), kb/ADR-0005.R1.
- F6 Every `reeve` command a skill uses supports `--json`, and Lorekeeper's planner detector runs `reeve` with standard error captured, not on a terminal — source: kb/ADR-0005.R3; Lorekeeper 3.9.1 `scripts/reeve-detect.js` (`stdio: ['ignore', 'pipe', 'pipe']`).
- F7 `public_url()` returns the configured `public_url` (https, or http only to a loopback host), or, when none is configured, an http loopback address on the bind's port; `public_url` is required only with `[oidc]` — source: `crates/reeve-server/src/config.rs` (`public_url`, `public_url()`, `default_public_url`), `deploy/README.md` (the quick start's `public_url` note).
- F8 For remote access, the reference deployment publishes the server on loopback and terminates TLS at the operator's reverse proxy — source: `deploy/README.md` (step 3 of the quick start, and the proxy section).

## Considered options
- **GitHub Releases installed through mise's `github:` backend** — a widely known channel with `mise upgrade`, no server change, and no token, since the repository is public. But it has one newest release for every instance, while instances run different releases: a person whose server is behind or ahead must find and pin the matching release by hand, and nothing tells them when it stops matching.
- **GitHub Releases selected by the instance** — upstream publishes each CLI version as a tagged release; the server advertises only its CLI version, and the install scripts and `reeve update` download exactly that release from GitHub. No manual pinning, no GitHub account and no growth of the server image. But every install and update then depends on GitHub being reachable from where the person is and on upstream keeping every old release, and a fork or a self-built server advertises a version upstream never published.
- **`cargo install --git` from `main`** (directly or through mise's `cargo:` backend) — no CI work at all. Each person needs Rust, every update is a full compile, `main` is rarely what their instance runs, and a named branch never looks upgradable.
- **The server serves the CLI built for its release, and the CLI asks to be updated** — one line installs it from the instance the person already uses, needs no GitHub access, and the client is the one the running server was built with, wherever that server's image came from.

Within the chosen option: updating automatically was rejected to keep the binary unchanged between the calls of one skill run, which this record requires (R6); updating only forwards was rejected because it needs a version order and leaves a client ahead of a server that runs an older release. Signed binaries are deferred, not refused: the project could sign its own builds (keyless signing, for instance), so no operator would need a key, and a signature would separate trust in the publisher from trust in the instance that delivers. They are deferred because the installer is itself a script fetched from the instance, and a substituted script can skip any check; signing pays off only with a verifier installed and trusted independently, which this record does not build. Building every target on Linux was rejected in favour of building on each target's own operating system: the binaries must run on that system before they ship (R2), so the runners would be needed anyway.

## Decision
Daniel Thyselius has each Reeve server serve, without sign-in, the CLI binaries built for its release, and has the CLI tell a person at a terminal to run `reeve update` whenever its CLI version differs from the server's, because one install line from the instance the team already uses, and an exact match with the server it talks to, beat a channel that every instance shares and that drifts from each of them.

- **R1** CI MUST compute the CLI version only from the input list in `scripts/cli-version.sh`, and MUST never overwrite a published CLI version: a run MAY compile a version that is not yet published, but MUST look again just before publishing and reuse what another run published meanwhile. Every image the release workflow publishes MUST carry, by digest, the bundle of its commit's CLI version, and MUST check that bundle's version and checksums before the image is pushed. An image built any other way MAY carry no bundle, and then serves no CLI.
- **R2** CI MUST build each target on a runner of its own operating system (Linux x86_64 and arm64; macOS arm64, and macOS x86_64 on an arm64 runner under Rosetta; Windows x86_64), with the exact compiler release the server image uses, and MUST run each binary before publishing it.
- **R3** While the server has a valid bundle and a configured public URL (R4), it MUST answer `GET` and `HEAD` under `/cli/` for exactly nine fixed names (the CLI version, the checksum file, the five binaries and the two install scripts), looked up in a fixed list and never used as a file path, and MUST answer 404 to any other name under `/cli/`, and to every name while distribution is off. These routes MUST ignore any token or cookie a request carries, neither authenticating it nor setting one, and MUST send `Cache-Control: no-cache` and `X-Content-Type-Options: nosniff`. No other route MAY become unauthenticated under this record. The deployment guide MUST rate-limit `/cli/` per client IP at the proxy, answering requests over the limit with 429.
- **R4** The install scripts MUST be rendered at startup with the instance's configured `public_url`, quoted for each script's language, and no released file MAY contain an instance's URL. Without a configured `public_url`, distribution MUST be off, and the startup log MUST say why.
- **R5** The install scripts and `reeve update` MUST send no token or cookie, MUST download only over HTTPS to the configured server URL (plain HTTP only to a loopback address), MUST NOT follow redirects, and MUST install a binary only once it matches the single checksum line for its file name. `reeve update` MUST also require every response it reads to report the same CLI version, MUST download beside the running executable and replace it only with a complete, verified file, MUST leave the running executable unchanged on any failure, and MUST refuse to replace an executable under a cargo, mise or nix directory unless the user passes `--force`. The install scripts MUST refuse to overwrite a `reeve` whose `--version` does not start with `reeve-planner` unless the user sets `REEVE_FORCE=1` (kb/ADR-0005.R2).
- **R6** The CLI MUST NOT replace itself unless the user runs `reeve update`. A stamped CLI whose CLI version differs from the server's MUST say so at most once per process and only to a person: on a standard error that is a terminal, without `--json`, or as the TUI's header note, decided before the TUI starts. Dev builds and the TUI's own clients MUST NOT print the notice, and the TUI MUST print nothing to standard error while it runs. The notice MUST NOT change standard output, the `--json` envelope or exit statuses (kb/ADR-0005.R3); `reeve auth status` MAY add both CLI versions to its output, `--json` included. `reeve --version` MUST stay `reeve-planner <semver>` and MUST NOT touch the network (kb/ADR-0005.R1).
- **R7** For an executable `reeve update` may replace (R5), a successful update MUST install the server's CLI version, even when it is older than the running one; when the two are equal it MUST replace nothing.

## Consequences
- Installing is one `curl … | sh` (or `irm … | iex`) line, then `reeve auth login`: no Rust, no mise, no GitHub access.
- Anyone who can reach the server can download the CLI and read its CLI version, a commit id. Neither holds team data, but the instance is no longer closed to anonymous requests beyond sign-in and the static client.
- Authenticity rests on HTTPS to the instance, and therefore on every party that terminates TLS in front of it (the operator's proxy, and any CDN): any of them can replace a binary and its checksum together, and a replaced binary runs with the installing person's full local rights, not only their Reeve token. The checksum catches a damaged download, not a substituted one.
- Every release now depends on the registry lookup for the CLI bundle, and a push that changes the CLI also depends on all five builds. A failure in either blocks the image and the subscriber notification, a security fix to the server included.
- A push that changes the CLI adds the slowest of five native builds to the time before the image is published; a push that does not change it adds one short lookup.
- The image grows by the bundle (an estimated 60 to 90 MB, one layer shared by images with the same CLI version), and each instance serves anonymous downloads of that size. A per-IP limit caps neither total bandwidth nor what people behind one shared address can download together.
- An operator who deploys an older release also sends its users back to that release's CLI, known defects and vulnerabilities included; choosing the release is the operator's decision.
- A dependency bump in the shared lock file that only the server uses still produces a new CLI version and an update notice.
- A skill that calls with `--json`, or captures standard error as Lorekeeper's detector does, never sees the notice; one that needs the versions reads `reeve auth status --json`.
- A stamped CLI installed another way (cargo, mise, nix) shows the notice and is updated that way; `reeve update` refuses it unless forced. A local dev build shows no notice.
- An instance without a configured `public_url` serves no CLI.
- Windows becomes a supported platform for the CLI and TUI, which needs a Windows configuration path and browser opener it does not have today.

## Assumptions and invalidation triggers
- *Assumes each team trusts its instance, its proxy and every TLS terminator in front of it as much as the binary they serve; today that is the operator's proxy, and Cloudflare in front of the team's own instance.* Trigger: a binary must stay trustworthy when one of them is compromised, or an operator puts in front of the instance a party the team does not already trust with its traffic ⇒ supersede R5 with signed binaries and an independently installed verifier.
- *Assumes CLI changes are infrequent enough that native builds on every CLI change are affordable.* Trigger: in a calendar month, the CLI jobs (from the first CLI job's start to the bundle's publish, by their timestamps in the release runs) delay the image by more than fifteen minutes on three or more pushes, or a server security fix is held up even once by a failing CLI build or a registry outage ⇒ revisit R1 and R2.
- *Assumes the proxy limit fits real use.* Trigger: real installs or updates are refused with 429, or CLI downloads become a noticeable share of an instance's traffic ⇒ revisit R3's limit.
- *Assumes people update when told.* Trigger: outdated clients cause a reported failure against the server more than once ⇒ revisit R6 (an update the TUI starts itself, or the server refusing clients older than a minimum).
- *Assumes one installed CLI needs to match only one instance release at a time; one team per instance (kb/ADR-0016) does not ensure that, since a guest or a member of two teams uses two instances.* Trigger: anyone needs one installation to work against two instances on different releases ⇒ supersede R7.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0005 (Lorekeeper detects the planner by `reeve --version` and reads it through `--json` with distinct exit statuses; R6 preserves all three)
- kb/ADR-0006 (one server binary serves the API and the client, and now the CLI; no API type changes)
- kb/ADR-0010 (CLI distribution is file serving in the composition root, not a context)
- kb/ADR-0011 (the TUI shows the notice as a header note decided before it starts; its login rules are unchanged)
- kb/ADR-0015 (sign-in and token kinds; R3 adds `/cli/` to the unauthenticated surfaces and reads no token there)
- kb/ADR-0016 (self-hosted, one team per instance; R4 keeps the instance's URL out of released files, and no new configuration or container)
- kb/ADR-0019 (proposed: `/privacy` would be another unauthenticated route, under its own record)
