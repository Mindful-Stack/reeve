---
id: kb/ADR-0030
title: "ADR-0030: The Reeve server distributes the CLI it was released with, and the CLI tells people and agents when to update"
description: "Each Reeve server with a configured public URL serves, without sign-in, the `reeve` CLI binaries built for its release, verified by SHA-256 over HTTPS, and the CLI says on standard error, to people and to agents that run it, to run `reeve update` when its build differs from the one the server offers, because one install line and an exact match with each instance beat a channel that every instance shares and that drifts from each of them."
tags: [adr, reeve, cli, deploy, security, lorekeeper]
status: proposed
date: 2026-11-23
decided_by: [Daniel Thyselius]
consulted: []
confidence: medium
reversibility: one-way
blast_radius: shared
sensitivity: [security]
scope: ["reeve:crates/reeve/**", "reeve:crates/reeve-server/src/cli_dist.rs", "reeve:crates/reeve-server/src/routes/**", "reeve:crates/reeve-server/src/state.rs", "reeve:Dockerfile", "reeve:deploy/cli/**", "reeve:deploy/README.md", "reeve:.github/workflows/cli.yml", "reeve:.github/workflows/release.yml", "reeve:scripts/cli-*.sh", "reeve:scripts/tests/**", "lorekeeper:skills/**"]
supersedes: [kb/ADR-0020]
superseded_by:
depends_on: [kb/ADR-0005, kb/ADR-0006, kb/ADR-0015, kb/ADR-0016]
related: [kb/ADR-0010, kb/ADR-0011, kb/ADR-0019, kb/ADR-0029]
implements: []
rfc:
---

# ADR-0030: The Reeve server distributes the CLI it was released with, and the CLI tells people and agents when to update

## Status
Proposed 2026-11-23.

## Context
This record replaces kb/ADR-0020, whose distribution design it keeps. What changed: kb/ADR-0020.R6 showed the update notice only on a standard error that is a terminal. Agents such as Claude Code run `reeve` without a terminal, so an agent would never learn that its CLI no longer matches the server and would keep running a stale one until a person noticed. Reviews of the implementation plan also tightened the install scripts' failure behaviour (R5), published the text agents match on (R6), and removed safeguards that a second check already covers (R1, R2, R5).

"Agent" in this record means one that runs the `reeve` CLI, such as Claude Code or a Lorekeeper skill. An MCP client (kb/ADR-0029) runs no Reeve code, so it cannot be out of date and gets no notice; the header on MCP responses is incidental.

People install the `reeve` CLI today by building it from a checkout, or with `cargo install` from the public repository. Both need a Rust toolchain and a full compile, and both leave each person on whatever commit they last built. Lorekeeper skills shell out to the same binary under an automation token and detect the planner by `reeve --version` (kb/ADR-0005), and the TUI is used against a live instance, so a client that drifts from its server meets API changes it was not built for.

Every push to `main` that changes code publishes one server image per commit and tells subscribed repositories about it; deploying is each operator's business, outside this repository. Each team runs its own instance (kb/ADR-0016) and chooses when to deploy which release, so instances run different releases at the same time, and an operator may go back to an earlier one. A push that changes only documentation publishes nothing. The release build already compiles the Linux x86_64 CLI next to the server. The team uses Linux (x86_64 and arm64), macOS (Apple Silicon and Intel) and Windows.

The instance sits behind sign-in. Its unauthenticated surfaces today are the health check, the OpenAPI document, the sign-in routes, the HMAC-checked webhook, the static web client, and the MCP endpoint's metadata, registration, authorize and token routes (kb/ADR-0029.R11); they serve the browser's code but no native program. The CLI can store a token only once it is installed, so a first-time installer has none. TLS is terminated at the operator's reverse proxy, and an operator may put a CDN in front of it, as the team's own instance does with Cloudflare.

Terms used here and in the design. They are deployment vocabulary, not the planner's domain language, and stay out of the planning domain nodes.
- **CLI version**: the commit id of the last commit that changed one of the CLI's build inputs, as `scripts/cli-version.sh` lists them. People see it as a **build**, shortened to seven characters. "Version" alone always means the semver that `reeve --version` prints, which this record never changes.
- **Stamped** CLI: one built with its CLI version. **Unstamped**: one built without it, wherever it was built (a local `cargo build`, the server image's own copy); people see it labelled `dev build`.
- **Bundle**: the binaries for every supported target, their `SHA256SUMS` and a `VERSION` file holding the CLI version, published once per CLI version.
- **The build the server offers**: the CLI version of the bundle a server serves. It is not the server's own commit, which differs whenever a push left the CLI unchanged.
- **Distribution is on** while a server has a valid bundle and a configured `public_url` (R3, R4), and off otherwise.

## Facts relied on
- F1 `release.yml` runs on every push to `main` that changes a path outside the ones it ignores (Markdown, `docs/`, `lore/`), publishes `ghcr.io/mindful-stack/reeve:<sha>`, and after a successful publish tries, best effort, to start each subscriber's release workflow; it deploys nothing — source: `.github/workflows/release.yml` (`on.push.paths-ignore`, `publish`, `notify`), `deploy/README.md` ("Subscribe to releases").
- F2 The release job's image build compiles `-p reeve` with the server and copies that Linux x86_64 binary into the image — source: `Dockerfile` (the `build` stage, `COPY --from=build /out/reeve`), `.github/workflows/release.yml` (`runs-on: ubuntu-24.04`).
- F3 Every registered `/api/v1` operation requires a caller; the other routes are `/health`, `/api/openapi.json`, `/auth/*`, the HMAC-checked `/webhooks/github`, the static client, and the MCP endpoint's metadata, registration, authorize and token routes, while the MCP endpoint itself requires a token issued for it — source: `crates/reeve-server/src/routes/mod.rs` (`app`), `crates/reeve-server/src/auth/mod.rs` (`Caller`), `crates/reeve-server/src/routes/webhooks.rs`, `crates/reeve-server/src/mcp/`, kb/ADR-0029.R2, R6, R11.
- F4 The deployment guide documents a per-client-IP limit on `/auth/login` at the proxy — source: `deploy/README.md` (`limit_req_zone … zone=reeve_login`).
- F5 `reeve --version` prints `reeve-planner <semver>` from the workspace version, and skills detect the planner by that prefix — source: `crates/reeve/src/lib.rs` (`#[command(name = "reeve-planner", …)]`), kb/ADR-0005.R1.
- F6 Every `reeve` command a skill uses supports `--json`; Lorekeeper's planner detector runs `reeve` with standard error captured, not on a terminal; Lorekeeper's skills already treat informational standard-error lines from `reeve` as expected; and Claude Code's Bash tool returns a command's standard error to the agent — source: kb/ADR-0005.R3; Lorekeeper 3.14.0 `scripts/reeve-detect.js` (`stdio: ['ignore', 'pipe', 'pipe']`) and `skills/planning-initiative/SKILL.md` (the expected `no decide-by date` stderr line).
- F7 `public_url()` returns the configured `public_url` (https, or http only to a loopback host), or, when none is configured, an http loopback address on the bind's port; `public_url` is required only with `[oidc]` — source: `crates/reeve-server/src/config.rs` (`public_url`, `public_url()`, `default_public_url`), `deploy/README.md` (the quick start's `public_url` note).
- F8 For remote access, the reference deployment publishes the server on loopback and terminates TLS at the operator's reverse proxy — source: `deploy/README.md` (step 3 of the quick start, and the proxy section).

## Considered options
- **GitHub Releases installed through mise's `github:` backend** — a widely known channel with `mise upgrade`, no server change, and no token, since the repository is public. But it has one newest release for every instance, while instances run different releases: a person whose server is behind or ahead must find and pin the matching release by hand, and nothing tells them when it stops matching.
- **GitHub Releases selected by the instance** — upstream publishes each CLI version as a tagged release; the server advertises only its CLI version, and the install scripts and `reeve update` download exactly that release from GitHub. No manual pinning, no GitHub account and no growth of the server image. But every install and update then depends on GitHub being reachable from where the person is and on upstream keeping every old release, and a fork or a self-built server advertises a version upstream never published.
- **`cargo install --git` from `main`** (directly or through mise's `cargo:` backend) — no CI work at all. Each person needs Rust, every update is a full compile, `main` is rarely what their instance runs, and a named branch never looks upgradable.
- **The server serves the CLI built for its release, and the CLI asks to be updated** — one line installs it from the instance the person already uses, needs no GitHub access, and the client is the one the running server was built with, wherever that server's image came from.

Within the chosen option: updating automatically was rejected to keep the binary unchanged behind a skill's back, which this record requires (R6); an agent that runs `reeve update` itself, between two commands, is a deliberate step in order and is allowed (R8); showing the notice only on a terminal (kb/ADR-0020.R6) was rejected because agents run without one; updating only forwards was rejected because it needs a version order and leaves a client ahead of a server that runs an older release. Signed binaries are deferred, not refused: the project could sign its own builds (keyless signing, for instance), so no operator would need a key, and a signature would separate trust in the publisher from trust in the instance that delivers. They are deferred because the installer is itself a script fetched from the instance, and a substituted script can skip any check; signing pays off only with a verifier installed and trusted independently, which this record does not build. Building every target on Linux was rejected in favour of building on each target's own operating system: the binaries must run on that system before they ship (R2), so the runners would be needed anyway.

## Decision
Daniel Thyselius has each Reeve server serve, without sign-in, the CLI binaries built for its release, and has the CLI tell people and agents alike, on standard error, to run `reeve update` whenever its CLI version differs from the server's, with agents updating between commands, because one install line from the instance the team already uses, and an exact match with the server it talks to, beat a channel that every instance shares and that drifts from each of them, and an agent that never hears of an update keeps working against a server its CLI was not built for.

- **R1** CI MUST compute the CLI version only from the input list in `scripts/cli-version.sh`, and MUST build and publish a bundle only for a CLI version it has not published, one run per version at a time. Every image the release workflow publishes MUST carry, by digest, the bundle of its commit's CLI version, and MUST check that the bundle's `VERSION` names that CLI version before the image is pushed. An image built any other way MAY carry no bundle, and then serves no CLI.
- **R2** CI MUST build each supported target on a runner of its own operating system (macOS x86_64 MAY run under Rosetta on an arm64 runner), and MUST run each binary before publishing it. The design lists the supported targets.
- **R3** While distribution is on, the server MUST answer `GET` and `HEAD` under `/cli/` for exactly the bundle's file names and the two install scripts, looked up in a fixed list and never used as a file path, and MUST answer 404 to any other name under `/cli/`, and to every name while distribution is off. These routes MUST ignore any token or cookie a request carries, neither authenticating it nor setting one, and MUST send `Cache-Control: no-cache` and `X-Content-Type-Options: nosniff`. No other route MAY become unauthenticated under this record. The deployment guide MUST rate-limit `/cli/` per client IP at the proxy, answering requests over the limit with 429.
- **R4** The install scripts MUST be rendered at startup with the instance's configured `public_url`, quoted for each script's language, and no released file MAY contain an instance's URL. Without a configured `public_url`, distribution MUST be off, and the startup log MUST say why.
- **R5** The install scripts and `reeve update` MUST send no token or cookie, MUST download only over HTTPS to the configured server URL (plain HTTP only to a loopback address), MUST NOT follow redirects, and MUST install a binary only once it matches the single checksum line for its file name. `reeve update` MUST download beside the running executable, replace it only with a complete, verified file, and leave it unchanged on any failure. The install scripts MUST refuse to overwrite a `reeve` whose `--version` does not start with `reeve-planner` unless the user sets `REEVE_FORCE=1` (kb/ADR-0005.R2), and MUST leave an existing installation working on any failure.
- **R6** The CLI MUST NOT replace itself unless someone runs `reeve update`. A stamped CLI whose build differs from the one the server offers MUST say so on standard error at most once per process, after the command's own output and whatever its outcome, with or without a terminal or `--json`, in a line that starts with `note: this reeve is build`. The TUI MUST instead show it as a header note decided before it starts, and MUST print nothing to standard error while it runs. Setting `REEVE_UPDATE_NOTICE=0` MUST silence both. Unstamped builds MUST NOT show either. The notice MUST NOT change standard output, the `--json` envelope or exit statuses (kb/ADR-0005.R3). `reeve auth status --json` MUST report both builds as `cli.current` and `cli.server`. The notice's prefix, that `cli` object and the `reason` values `reeve update` reports on failure are the contract agents match on, and MUST change only through a record. `reeve --version` MUST stay `reeve-planner <semver>` and MUST NOT touch the network (kb/ADR-0005.R1).
- **R7** A successful `reeve update` MUST install the build the server offers, even when it is older than the running one; when the two are equal it MUST replace nothing.
- **R8** Lorekeeper's skills, and any agent guidance this project ships, MUST instruct an agent that sees the notice, or unequal `cli` builds in `reeve auth status --json`, to run `reeve update` only between two planner commands, never while one is running, at most once per session, and to report that it updated in its output (to the user, when one is present) and continue with the updated CLI. If the notice appears again after a successful update, or `reeve update` fails, the agent MUST stop updating, report how to update, and continue with the CLI it has.

## Consequences
- Installing is one `curl … | sh` (or `irm … | iex`) line, then `reeve auth login`: no Rust, no mise, no GitHub access.
- Anyone who can reach the server can download the CLI and read its CLI version, a commit id. Neither holds team data, but the instance is no longer closed to anonymous requests beyond sign-in and the static client.
- Authenticity rests on HTTPS to the instance, and therefore on every party that terminates TLS in front of it (the operator's proxy, and any CDN): any of them can replace a binary and its checksum together, and a replaced binary runs with the installing person's full local rights, not only their Reeve token. The checksum catches a damaged download, not a substituted one.
- Every release now depends on the registry lookup for the CLI bundle, and a push that changes the CLI also depends on every target's build. A failure in either blocks the image and the subscriber notification, a security fix to the server included.
- A push that changes the CLI adds the slowest native build to the time before the image is published; a push that does not change it adds one short lookup.
- The image grows by the bundle (an estimated 60 to 90 MB, one layer shared by images with the same CLI version), and each instance serves anonymous downloads of that size. A per-IP limit caps neither total bandwidth nor what people behind one shared address can download together.
- An operator who deploys an older release also sends its users back to that release's CLI, known defects and vulnerabilities included; choosing the release is the operator's decision.
- A dependency bump in the shared lock file that only the server uses still produces a new CLI version and an update notice.
- Agents that run the CLI see the notice on standard error and update between commands (R8), so an agent session catches up with the server without a person stepping in. MCP clients are unaffected. Scripts that must keep standard error quiet set `REEVE_UPDATE_NOTICE=0`; a script that needs the builds reads `reeve auth status --json`.
- The notice's text is now an interface: Lorekeeper conforms to it, as it does to `--json` and the `reeve-planner` prefix (kb/ADR-0005), so rewording it needs a record.
- An agent may replace the binary a person is also using; the next command either of them runs uses the new CLI, and a TUI already running keeps the old one until it restarts.
- A stamped CLI installed by cargo or mise shows the notice like any other; `reeve update` replaces it in place, and the package manager's next upgrade replaces it again. One in the nix store is read-only, so the update fails and the agent reports how to update. `REEVE_UPDATE_NOTICE=0` silences a notice nobody wants. An unstamped build shows no notice.
- `reeve --version` prints the same semver before and after an update; the build shows in the notice, in `reeve auth status` and in the install scripts' last line.
- An instance without a configured `public_url` serves no CLI.
- Windows becomes a supported platform for the CLI and TUI, which needs a Windows configuration path and browser opener it does not have today.

## Assumptions and invalidation triggers
- *Assumes each team trusts its instance, its proxy and every TLS terminator in front of it as much as the binary they serve; today that is the operator's proxy, and Cloudflare in front of the team's own instance.* Trigger: a binary must stay trustworthy when one of them is compromised, or an operator puts in front of the instance a party the team does not already trust with its traffic ⇒ supersede R5 with signed binaries and an independently installed verifier.
- *Assumes CLI changes are infrequent enough that native builds on every CLI change are affordable.* Trigger: in a calendar month, the CLI jobs (from the first CLI job's start to the bundle's publish, by their timestamps in the release runs) delay the image by more than fifteen minutes on three or more pushes, or a server security fix is held up even once by a failing CLI build or a registry outage ⇒ revisit R1 and R2.
- *Assumes the proxy limit fits real use.* Trigger: real installs or updates are refused with 429, or CLI downloads become a noticeable share of an instance's traffic ⇒ revisit R3's limit.
- *Assumes people and agents update when told.* Trigger: outdated clients cause a reported failure against the server more than once ⇒ revisit R6 (an update the TUI starts itself, or the server refusing clients older than a minimum).
- *Assumes one installed CLI needs to match only one instance release at a time; one team per instance (kb/ADR-0016) does not ensure that, since a guest or a member of two teams uses two instances.* R8's once-per-session limit stops an agent swapping the binary back and forth between them. Trigger: anyone needs one installation to work against two instances on different releases ⇒ supersede R7.

## Later observations
<!-- append-only, accepted records only; newest last -->

## See also
- kb/ADR-0020 (superseded by this record: same distribution design, with the notice shown only on a terminal)
- kb/ADR-0005 (Lorekeeper detects the planner by `reeve --version` and reads it through `--json` with distinct exit statuses; R6 preserves all three, and R8 adds how its skills update)
- kb/ADR-0006 (one server binary serves the API and the client, and now the CLI; no API type changes)
- kb/ADR-0010 (CLI distribution is file serving in the composition root, not a context)
- kb/ADR-0011 (the TUI shows the notice as a header note decided before it starts; its login rules are unchanged)
- kb/ADR-0015 (sign-in and token kinds; R3 adds `/cli/` to the unauthenticated surfaces and reads no token there)
- kb/ADR-0016 (self-hosted, one team per instance; R4 keeps the instance's URL out of released files, and no new configuration or container)
- kb/ADR-0019 (proposed: `/privacy` would be another unauthenticated route, under its own record)
- kb/ADR-0029 (MCP sign-in adds its own unauthenticated routes under its own record, and depends on kb/ADR-0020's account of them; this record keeps the CLI routes token-free, as kb/ADR-0029 relies on; MCP clients get no notice)
