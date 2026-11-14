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

## Trying the MCP endpoint locally

To connect an AI app such as Claude to a running instance, see
[docs/connecting-an-ai-app.md](docs/connecting-an-ai-app.md); this section is for developers.

The MCP endpoint (`/mcp`) is off unless `[mcp] enabled = true`. The quickest way to try it locally is a token a
developer mints (connecting a real app is in `deploy/README.md`). Both need a binary built with the test sign-in
feature:

1. In `reeve.toml`, keep `bind` and `public_url` on loopback and add:

   ```toml
   [oidc]
   provider = "static"
   static_email = "dev@example.com"

   [mcp]
   enabled = true
   ```

   (`dev@example.com` must be allowed by `allowed_domains` or `allowed_emails`.)

2. Start the server. `serve` runs until it is stopped, so leave it in a shell of its own:

   ```bash
   mise run db-up
   cargo run -p reeve-server --features reeve-server/static-oidc -- serve
   ```

   It logs `mcp endpoint enabled at http://127.0.0.1:7474/mcp (protocol 2026-07-28 and 2025-11-25, 9 tools)`.

   In a second shell, mint a token. It is valid for one hour and is printed once:

   ```bash
   TOKEN=$(cargo run -p reeve-server --features reeve-server/static-oidc -- mcp-token-issue --email dev@example.com)
   ```

3. One call with `curl`, with every header revision 2026-07-28 requires:

   ```bash
   curl -sS http://127.0.0.1:7474/mcp \
     -H "Authorization: Bearer $TOKEN" \
     -H 'Content-Type: application/json' \
     -H 'MCP-Protocol-Version: 2026-07-28' \
     -H 'Mcp-Method: tools/call' \
     -H 'Mcp-Name: get_story' \
     -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_story","arguments":{"key":"ABC-1"},"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}'
   ```

   A list tool takes its filters as arguments. "What is on my plate" is the caller's open stories:

   ```bash
   curl -sS http://127.0.0.1:7474/mcp \
     -H "Authorization: Bearer $TOKEN" \
     -H 'Content-Type: application/json' \
     -H 'MCP-Protocol-Version: 2026-07-28' \
     -H 'Mcp-Method: tools/call' \
     -H 'Mcp-Name: list_stories' \
     -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_stories","arguments":{"mine":true,"open":true},"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}'
   ```

   `tools/list` (the inspector line below) shows all nine tools with their arguments.

4. The MCP inspector, from the command line (it speaks revision 2025-11-25):

   ```bash
   npx -y @modelcontextprotocol/inspector --cli http://127.0.0.1:7474/mcp --transport http \
     --method tools/list --header "Authorization: Bearer $TOKEN"
   npx -y @modelcontextprotocol/inspector --cli http://127.0.0.1:7474/mcp --transport http \
     --method tools/call --tool-name get_story --tool-arg key=ABC-1 --header "Authorization: Bearer $TOKEN"
   ```

5. Claude Code (it speaks revision 2026-07-28):

   ```bash
   claude mcp add --transport http reeve-local http://127.0.0.1:7474/mcp --header "Authorization: Bearer $TOKEN"
   ```

What the refusals look like: `401` with
`WWW-Authenticate: Bearer resource_metadata="http://127.0.0.1:7474/.well-known/oauth-protected-resource/mcp"` (the
address of the document that tells a client where to sign in) for a missing, expired or non-MCP token, with
`error="invalid_token"` before `resource_metadata` when a bearer token was sent and refused (an API token from the
Tokens page is refused here, and an MCP token is refused by the API); `403` when the request carries
an `Origin` header that is not in `[mcp] allowed_origins` (by default only `public_url`'s own origin; add
`http://localhost:6274` for the inspector's browser page); `404` with the text
`MCP is not enabled on this instance ([mcp] enabled)` when the switch is off.

**Warning.** claude.ai cannot connect yet: a custom connector signs in with OAuth, which this endpoint does not
offer before the sign-in stories land. Do not put a local instance behind a tunnel to try it: the static sign-in
provider signs in anyone who reaches `/auth/callback`. If a tunnel is needed (to see in the server's log which
protocol revision a client sends), forward only the path `/mcp` through a local reverse proxy that answers 404 for
everything else.

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
