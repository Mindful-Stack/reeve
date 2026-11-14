# Reeve (planner)

This repository is a **witan-household meta-repo** for the Reeve planner: the manifest at
`household.json` names the runner (`reeve-runner`) as a sibling, and the inline `lore/`
knowledge base holds the household's single ADR home at `lore/knowledge/adrs/`.

Milestone one (the core planner) is implemented: six crates under `crates/` (plus `reeve-testdb`, test support), a Postgres-backed server
and the `reeve` CLI. Read first: the records under `lore/knowledge/adrs/` and `lore/knowledge/general/architecture.md`
(the rules the crates follow). Each further milestone gets its own spec and plan; they are working documents and are
not committed to this repository.

## Build and verify

Tasks are defined once in `mise.toml` and run the same way locally and in CI (`mise tasks` lists them all):

| Command | What it does |
|---|---|
| `mise run test` | `scripts/test.sh`: one PostgreSQL 16 for the run, then `cargo nextest run --workspace --features reeve-server/static-oidc`, the doc tests (`cargo test --doc`), then `cargo nextest run -p reeve-server --test config_no_static` WITHOUT the feature (proves a production build cannot parse the static provider); needs Docker for PostgreSQL; tests are never skipped |
| `mise run lint` | `cargo fmt --check`, clippy with `-D warnings` twice (with `--features reeve-server/static-oidc` and without it), the isolation lint, actionlint on `.github/workflows/` and the action-pin check (every third-party action pinned by SHA with a version comment) |
| `mise run audit` | `audit-rust` (`cargo deny check advisories`; policy and commented ignores in `deny.toml`) and `audit-web` (`npm audit` for `web/` through `scripts/npm-audit.mjs`; ignores in `scripts/npm-audit-ignore.json`); CI runs them in `audit.yml` on dependency changes and daily; not required checks |
| `mise run isolation`, `mise run isolation-test` | run the context-isolation lint; prove the lint catches every bypass shape |
| `mise run acceptance` | the CLI end-to-end acceptance test |
| `mise run import-test` | `reeve import jira` end to end against a real server and an in-test fake Jira (needs Docker) |
| `mise run script-test` | the backup and restore-rehearsal scripts, the skill smoke's logic against fakes, the release workflow's shell (`scripts/tests/workflows.sh`), its release notifications against a fake GitHub API (`scripts/tests/release-dispatch.sh`), the action-pin check's cases, ci.yml's Rust path classifier (`scripts/ci-needs-rust.sh`) and the npm audit gate's tests; no Docker needed; shellcheck is pinned in `mise.toml` |
| `mise run decision-check-test` | the scheduled decision check script (`deploy/decision-check.sh`) against fake binaries and a real git remote, plus the systemd units and compose file where those tools exist |
| `mise run tui-test` | the TUI against a scripted server, against a real server and PostgreSQL, and in a real pty (needs Docker); `mise run test` already runs these files |
| `mise run web` | `npm ci` and build the web client into `web/build` (embedded by the next `reeve-server` build) |
| `mise run web-check` | regenerate `schema.d.ts` from the snapshot and fail on drift, `svelte-check`, vitest |
| `mise run web-smoke` | the Playwright smoke against a real server and a fresh database (`scripts/web-smoke.sh`) |
| `mise run skill-smoke` | every command of Lorekeeper's `references/reeve-cli.md` against a real server and the fake CLI (`scripts/skill-smoke.sh`); needs a Lorekeeper checkout (`LOREKEEPER_DIR`, default `../lorekeeper`) and the compose Postgres or an allowed `DATABASE_URL`'s server, on which it creates and drops a database of its own |

Every push to `main` publishes the image `ghcr.io/mindful-stack/reeve:<sha>` (`.github/workflows/release.yml`); deploying it is the operator's, outside this repository (`deploy/README.md` covers the reference stack).

Server tests, clippy and the acceptance test build with `--features reeve-server/static-oidc` (the static sign-in
provider the test harness selects); `mise.toml` passes it through `REEVE_FEATURES`, and the API tests fail without it (the harness config
does not parse). The Dockerfile never enables it. nextest's profile is `.config/nextest.toml` (no retries).

Add an RPC: handler in the owning crate, route in `reeve-server/src/routes`, `UPDATE_SNAPSHOTS=1 cargo test -p reeve-server --test openapi_snapshot`
once (each handler also needs a `#[utoipa::path]` annotation registered in that route module's `router()`),
commit the snapshot.

**A wire change is one commit:** the type in `reeve-protocol`, the snapshot (`UPDATE_SNAPSHOTS=1 cargo test -p
reeve-server --test openapi_snapshot`) and the client types (`cd web && npm run gen`, which rewrites
`web/src/lib/api/schema.d.ts`), committed together. The client never defines a DTO by hand.
An MCP tool is wire as well: its `…Input` type in `reeve-protocol`, its annotations and the tool-list snapshot
(`UPDATE_SNAPSHOTS=1 scripts/test.sh -p reeve-server --features reeve-server/static-oidc --test mcp the_tool_list`,
which rewrites `crates/reeve-server/tests/snapshots/mcp_tools.json`) change in one commit.

Two rules the tests pin and no change may break: the GitHub webhook MUST NOT move a story (no path to `move_story`,
no write to delivery, resolution or version; pull-request status is informational), and the importer MUST NOT write
an `override` activity entry (grandfathered stories are created in their state and logged `imported`). Only
`Via::Interactive` may override the readiness gate. Moving a story never depends on goals (kb/ADR-0014 R4).

An API route and its MCP tool share one action (`crates/reeve-server/src/actions/`): validation and use-case logic live in the action; the route and the tool adapter only convert their input and present the answer (a tool adapter also bounds lists and cuts long text, `crates/reeve-server/src/mcp/tools/bound.rs`) (kb/ADR-0029 R1).

Conventions carried over from reeve-runner: `lore/knowledge/general/commit-conventions.md`,
`lore/knowledge/frameworks/rust/`, `lore/knowledge/languages/rust/`.
