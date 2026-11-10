<img src="assets/brand/reeve-seal-512.png" width="96" height="96" alt="Reeve">

# Reeve

Reeve is a planning tool for small software teams that work with coding agents.

Work lives in three levels, initiative → epic → story, each with a key. Any item
can carry required decisions, each with a responsible person and a link to a
decision record in git that Reeve checks for acceptance. A story is ready when
its ancestors are through discovery, nothing it requires is undecided, it has
acceptance criteria and nothing blocks it. Moving a story that is not ready out
of todo needs a written reason from a person. A CLI with JSON output lets agents
create, place and read that same structure, and a web board plus a strategic
planning view lets everyone else see it.

Reeve is the successor to the original Reeve, now
[reeve-runner](https://github.com/Mindful-Stack/reeve-runner), which orchestrated
Claude Code sessions per card and becomes one pluggable executor in milestone two.

## Running it

The project's tasks run through [mise](https://mise.jdx.dev): install it, run
`mise install` once (it fetches Node 22; Rust comes from `rust-toolchain.toml`),
then `mise tasks` lists everything.

Start PostgreSQL and the server with `mise run run`, then open the bind address
(`http://127.0.0.1:7474` by default) and sign in with Google: the server
serves the web client itself (planning view, board, decisions, activity,
tokens), once `mise run web` has built it and an `[oidc]` section is configured.
Signed-in users create their CLI token on the Tokens page (`reeve auth login`
walks through it). [deploy/README.md](deploy/README.md)
covers configuration, Google sign-in, tokens, the GitHub webhook (pull requests
linked to stories by branch name, shown with their status, never moving a story),
the cut-over-day runbook for `reeve import jira` (a rehearsal by default,
`--apply` behind the server's `[import] enabled` switch; its test is
`mise run import-test`), `reeve export` and the operator cron for the record check. The
whole flow (create a story and a required decision, see the gate refuse, link a
record, watch the story become ready) is the acceptance test, run with
`mise run acceptance`. The decision records behind the design are in
`lore/knowledge/adrs/`, this household's single ADR home, and the rules the
crates follow are in `lore/knowledge/general/architecture.md`.

## Web client

The browser client lives in `web/` (SvelteKit on Svelte 5, built as a single
page with the static adapter). For development run `mise run run` for the server
and `mise run web-dev` for the Vite dev server, which proxies `/api` and the
`/auth` endpoints to `http://127.0.0.1:7474` so the session cookie stays
first-party. `mise run web` builds `web/build`, which `reeve-server` embeds on its
next build; the container image builds the client in its own Node stage, so
production needs no Node. API types in `web/src/lib/api/schema.d.ts` are
generated from the OpenAPI snapshot (`cd web && npm run gen`) and never written
by hand; `mise run web-check` fails when they drift. `mise run web-smoke` runs the
Playwright smoke against a real server and a fresh database; `mise run skill-smoke`
runs the Lorekeeper planning skills' `reeve` commands against a real server (it
needs a Lorekeeper checkout next to this repository, or `LOREKEEPER_DIR`).

## Layout

```
crates/
  reeve-protocol/   API types; the OpenAPI document is generated from them
  reeve-planning/   initiatives, epics, stories, required decisions, comments
  reeve-identity/   users and tokens
  reeve-activity/   the append-only activity log
  reeve-server/     composition root; serves the API
  reeve/            CLI
web/                SvelteKit web client (single-page build, embedded by reeve-server)
deploy/             run instructions
scripts/            context-isolation lint and its test, the web smoke
lore/               knowledge base; lore/knowledge/adrs/ is the ADR home
```

The TUI is not built yet.

## Household

This repository is its own Witan household. `household.json` names reeve-runner
as a sibling, and the inline knowledge base holds every decision record.

## Support

Reeve is developed by [Mindful Stack](https://github.com/Mindful-Stack) and used
in its own projects. It is provided as is, without warranty and without a promise
of support, fixes or response times. Bug reports and pull requests are welcome as
GitHub issues and pull requests; they are read when time allows, and a change is
taken when it fits the project's direction and its decision records. Please
report a security problem through GitHub's private vulnerability reporting rather
than in a public issue.

## Licence

MIT; see [LICENSE](LICENSE).
