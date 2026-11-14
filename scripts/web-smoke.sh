#!/usr/bin/env bash
# Testing layer five: the Playwright smoke against a real reeve-server (static OIDC provider) and a seeded database.
#
#   scripts/web-smoke.sh                     # local: a fresh `reeve_smoke` database in the compose Postgres
#   CI=true DATABASE_URL=postgres://… scripts/web-smoke.sh   # CI: an EMPTY database (the seed is not repeatable on top of itself)
#
# An exported DATABASE_URL is used only when CI=true or REEVE_SMOKE_ALLOW_DATABASE_URL=1: developers often export it for
# sqlx tooling, and the smoke migrates, seeds and moves stories in whatever database it is given.
#
# Builds the client and both binaries, starts the server on 127.0.0.1:7475, seeds the fixture through the CLI, runs
# `npx playwright test` in web/ and stops the server on exit. Extra arguments go to Playwright.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

url="http://127.0.0.1:7475"
email="smoke@example.com"
work="$root/target/smoke"
mkdir -p "$work"

log() { printf '\n== %s\n' "$*" >&2; }

log "building the client"
(cd web && npm run build)

log "building reeve-server (static-oidc) and reeve"
cargo build -p reeve-server -p reeve --features reeve-server/static-oidc
server="$root/target/debug/reeve-server"
reeve="$root/target/debug/reeve"

if [[ -n "${DATABASE_URL:-}" && ( "${CI:-}" == "true" || "${REEVE_SMOKE_ALLOW_DATABASE_URL:-}" == "1" ) ]]; then
	db="$DATABASE_URL"
else
	if [[ -n "${DATABASE_URL:-}" ]]; then
		echo "warning: ignoring the exported DATABASE_URL (set REEVE_SMOKE_ALLOW_DATABASE_URL=1 to seed it); using reeve_smoke" >&2
	fi
	log "fresh reeve_smoke database in the compose Postgres"
	mise run db-up
	for _ in $(seq 60); do
		docker compose exec -T postgres pg_isready -U reeve -d reeve >/dev/null 2>&1 && break
		sleep 1
	done
	docker compose exec -T postgres psql -q -v ON_ERROR_STOP=1 -U reeve -d reeve \
		-c 'DROP DATABASE IF EXISTS reeve_smoke WITH (FORCE)' -c 'CREATE DATABASE reeve_smoke'
	db="postgres://reeve:reeve@localhost:5433/reeve_smoke"
fi

config="$work/reeve.toml"
# Escape sed's replacement specials (`\`, `&` and the `|` delimiter) so a URL such as `?sslmode=require&...` survives.
db_sed="$(printf '%s' "$db" | sed -e 's/[\\&|]/\\&/g')"
sed "s|@DATABASE_URL@|$db_sed|" reeve.smoke.toml >"$config"
export REEVE_CONFIG="$config"

# A leftover server on the port would answer the health check while the new one fails to bind.
# 7475 is the server's; 7477 is the stand-in for an MCP client that one test starts (reeve.smoke.toml lists its
# callback).
for port in 7475 7477; do
	if (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
		echo "port $port is already in use; stop whatever listens there (a previous smoke server?) and retry" >&2
		exit 1
	fi
done

log "starting reeve-server on $url"
# Auth refusals name their reason at warn/debug (reeve_server's Caller, reeve_identity's authenticate_api); CI uploads this
# log with the traces when the smoke fails.
RUST_LOG="${RUST_LOG:-info,reeve_server=debug,reeve_identity=debug}" "$server" serve >"$work/server.log" 2>&1 &
pid=$!
cli_dir="$(mktemp -d)"
cleanup() {
	kill "$pid" 2>/dev/null || true
	wait "$pid" 2>/dev/null || true
	rm -rf "$cli_dir"
}
trap cleanup EXIT

for _ in $(seq 60); do
	curl -fsS --max-time 2 "$url/health" >/dev/null 2>&1 && break
	if ! kill -0 "$pid" 2>/dev/null; then
		cat "$work/server.log" >&2
		echo "reeve-server exited before it was healthy" >&2
		exit 1
	fi
	sleep 1
done
curl -fsS --max-time 2 "$url/health" >/dev/null || { cat "$work/server.log" >&2; echo "reeve-server never became healthy" >&2; exit 1; }

log "seeding through the CLI"
# The CLI gets its own config directory and an interactive token, so nothing touches the developer's own config.
export REEVE_CONFIG_DIR="$cli_dir"
unset REEVE_TOKEN REEVE_URL
"$server" token-issue --email "$email" --kind interactive --label smoke |
	"$reeve" auth set-token --url "$url" --token-stdin >/dev/null
SMOKE_MEMBER_TOKEN="$("$server" token-issue --email smoke-member@example.com --kind interactive --label smoke-member)"

# `reeve --json …` prints the envelope; field PATH reads value.PATH from it and fails on anything but outcome ok.
cli() { "$reeve" --json "$@"; }
field() {
	node -e '
		let s = ""; process.stdin.on("data", (d) => (s += d)).on("end", () => {
			const env = JSON.parse(s);
			if (env.outcome !== "ok") { console.error(s); process.exit(1); }
			const v = process.argv[1].split(".").reduce((o, k) => o[k], env.value);
			console.log(v);
		});' "$1"
}

ini_json="$(cli initiative create --idempotency-key smoke-ini --title 'Smoke initiative' --horizon Now)"
ini="$(field key <<<"$ini_json")"
cli initiative discovery "$ini" --version "$(field version <<<"$ini_json")" --complete >/dev/null

epic_a_json="$(cli epic create --idempotency-key smoke-epic-a --title 'Smoke epic A' --initiative "$ini")"
epic_a="$(field key <<<"$epic_a_json")"
cli epic discovery "$epic_a" --version "$(field version <<<"$epic_a_json")" --complete >/dev/null

epic_b_json="$(cli epic create --idempotency-key smoke-epic-b --title 'Smoke epic B' --initiative "$ini")"
epic_b="$(field key <<<"$epic_b_json")"
cli decision require --idempotency-key smoke-decision --on "$epic_b" --title 'Smoke decision' --decider "$email" >/dev/null
epic_b_version="$(cli epic show "$epic_b" | field version)"
cli epic discovery "$epic_b" --version "$epic_b_version" --complete >/dev/null

SMOKE_STORY="$(cli story create --idempotency-key smoke-story --epic "$epic_a" --title 'Smoke ready story' \
	--criterion 'it works' | field key)"
SMOKE_BLOCKED_STORY="$(cli story create --idempotency-key smoke-blocked --epic "$epic_a" --title 'Smoke story without criteria' |
	field key)"
SMOKE_DECISION_STORY="$(cli story create --idempotency-key smoke-decision-story --epic "$epic_b" \
	--title 'Smoke story behind a decision' --criterion 'it is decided' | field key)"
# The discovery-complete epic the smoke's chores sit under: an item without an epic is untriaged, not ready.
SMOKE_EPIC="$epic_a"
export REEVE_URL="$url" SMOKE_MEMBER_TOKEN SMOKE_STORY SMOKE_BLOCKED_STORY SMOKE_DECISION_STORY SMOKE_EPIC
echo "seeded $ini: $SMOKE_STORY (ready), $SMOKE_BLOCKED_STORY (no criteria), $SMOKE_DECISION_STORY (decision)" >&2

log "running the Playwright smoke against $url"
# No session is exported: the tests sign in through the browser.
unset REEVE_CONFIG_DIR REEVE_CONFIG
(cd web && npx playwright test "$@")
