#!/usr/bin/env bash
# The real-planner smoke for the Lorekeeper skills: every command in Lorekeeper's references/reeve-cli.md *Commands*
# table runs against a real reeve-server, and the fake CLI the skills' scenario tests replay must answer in the same shape.
#
#   scripts/skill-smoke.sh                                   # local: a database of its own on the compose Postgres
#   CI=true DATABASE_URL=postgres://… scripts/skill-smoke.sh # CI (or REEVE_SMOKE_ALLOW_DATABASE_URL=1 locally)
#
# The smoke writes only in a database it created. It creates `reeve_skill_smoke_<pid>_<random>` (with no DROP before it,
# so it can never remove a database it did not make) on the compose Postgres or on the server an allowed DATABASE_URL
# names, and drops exactly that database on exit, after a failure too. DATABASE_URL is only the connection used to
# create and drop it: the database the URL names is never migrated or written. An exported DATABASE_URL is used only
# when CI=true or REEVE_SMOKE_ALLOW_DATABASE_URL=1 (the web smoke's guard): creating a database still writes to that
# server. A run killed before its trap ran leaves its own uniquely named database behind, never anyone else's.
#
# LOREKEEPER_DIR names the Lorekeeper checkout (default: ../lorekeeper next to this repository). Reeve never depends on
# Lorekeeper (kb/ADR-0005 R5); only this script, its mise task and its CI job read it.
#
# Builds both binaries (static-oidc), starts the server on 127.0.0.1:7476, issues an automation token for the smoke admin,
# seeds the sprints and the promises a person makes (seed_sprints and seed_promise, with an interactive token), builds a household with one accepted record, runs Lorekeeper's reeve-real-server and reeve-reference tests against them
# (every test must run and pass: none skipped, todo or cancelled), then stops the server and drops its database.
#
# Sourcing the script defines its functions without running the smoke: scripts/tests/skill-smoke.sh drives them
# (`mise run script-test`).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
url="http://127.0.0.1:7476"
email="skill-smoke@example.com"
work="$root/target/skill-smoke"
config="$work/reeve.toml"
# The fewest tests the Lorekeeper run may report: 12 top-level tests plus one subtest per Commands row (36) and per
# refusal case (7, a person's promise commands). A lower count means a test stopped running; raise it when Lorekeeper
# adds tests, never lower it to make a run pass.
min_tests=55

# State the cleanup reads; main fills it in as the run creates things.
use_url=""
admin_via="" # compose | psql | docker
pg_env=()    # PGHOST=… and the rest, split once from DATABASE_URL; exported only in admin_sql's subshell
db_name=""
db_created=""
pid=""
tests_pid=""
cli_dir=""

log() { printf '\n== %s\n' "$*" >&2; }

# smoke_database_url URL NAME: URL with its database replaced by NAME, the server, credentials and other parameters kept.
# sqlx takes the database from the URL's path and then from a `dbname` query parameter, which wins (sqlx-postgres 0.8.6,
# src/options/parse.rs: the path at lines 41-48, then `"dbname" => options.database` in the query loop, line 84; no other
# parameter selects a database). So the path is replaced and every `dbname` removed; a URL that still names another
# database, or is not a postgres:// URL with a host (a key=value string, an opaque `postgres:x`), is refused with a fixed
# message that never echoes the input (it holds the admin password). So is a URL whose query sets `host`, `hostaddr`,
# `port`, `user` or `password`: sqlx honours them, admin_pg_env reads only the authority, and the admin connection and
# the server would reach different servers or users.
smoke_database_url() {
	node -e '
		const [raw, name] = process.argv.slice(1);
		const refuse = () => {
			console.error("DATABASE_URL must be a postgres:// or postgresql:// URL with a host; refusing (the value is not printed)");
			process.exit(1);
		};
		let u;
		try { u = new URL(raw); } catch { refuse(); }
		if (!["postgres:", "postgresql:"].includes(u.protocol) || !u.host) refuse();
		if (["host", "hostaddr", "port", "user", "password"].some((k) => u.searchParams.has(k))) refuse();
		u.pathname = "/" + name;
		if (u.searchParams.has("dbname")) u.searchParams.delete("dbname");
		if (u.pathname !== "/" + name || u.searchParams.has("dbname")) refuse();
		process.stdout.write(u.toString());' "$1" "$2"
}

# admin_pg_env URL: the URL as libpq environment variables, one NAME=value per line (PGHOST, PGPORT, PGUSER, PGPASSWORD,
# PGDATABASE, PGSSLMODE; only those the URL sets), so the password never reaches a command line. Refused like above,
# connection parameters in the query included.
admin_pg_env() {
	node -e '
		const refuse = () => {
			console.error("DATABASE_URL must be a postgres:// or postgresql:// URL with a host; refusing (the value is not printed)");
			process.exit(1);
		};
		let u;
		try { u = new URL(process.argv[1]); } catch { refuse(); }
		if (!["postgres:", "postgresql:"].includes(u.protocol) || !u.host) refuse();
		if (["host", "hostaddr", "port", "user", "password"].some((k) => u.searchParams.has(k))) refuse();
		const d = (s) => { try { return decodeURIComponent(s); } catch { refuse(); } };
		const env = {
			PGHOST: u.hostname.replace(/^\[|\]$/g, ""),
			PGPORT: u.port,
			PGUSER: d(u.username),
			PGPASSWORD: d(u.password),
			PGDATABASE: u.searchParams.get("dbname") ?? d(u.pathname.replace(/^\//, "")),
			PGSSLMODE: u.searchParams.get("sslmode") ?? "",
		};
		for (const [k, v] of Object.entries(env)) {
			if (/[\n\r\0]/.test(v)) refuse();
			if (v) process.stdout.write(k + "=" + v + "\n");
		}' "$1"
}

# One SQL statement on the admin connection: the compose container's psql, or for DATABASE_URL the host's psql when
# there is one, else the postgres:16 image's psql on the host network. That connection comes from pg_env, exported in
# a subshell and passed to docker by name.
admin_sql() {
	case "$admin_via" in
	compose) docker compose exec -T postgres psql -q -v ON_ERROR_STOP=1 -U reeve -d reeve -c "$1" ;;
	psql) (export "${pg_env[@]}" && psql -q -v ON_ERROR_STOP=1 -c "$1") ;;
	docker)
		local kv names=()
		for kv in "${pg_env[@]}"; do names+=(-e "${kv%%=*}"); done
		(export "${pg_env[@]}" && docker run --rm --network host "${names[@]}" postgres:16 psql -q -v ON_ERROR_STOP=1 -c "$1")
		;;
	*)
		echo "admin_sql: no admin connection chosen" >&2
		return 1
		;;
	esac
}

# The only place db_created is set, and only after the CREATE succeeded: a failed CREATE (a name that exists) drops nothing.
create_smoke_database() {
	admin_sql "CREATE DATABASE \"$db_name\"" >/dev/null
	db_created=1
}

# tap_verdict FILE MIN: the summary node writes after the stream's last top-level plan line must hold exactly one each of
# `# tests N` (N >= MIN), `# pass N` (the same N), `# fail 0`, `# cancelled 0`, `# skipped 0` and `# todo 0`. A skipped
# or todo test reports green while checking nothing (lore/knowledge/general/testing.md); an empty or truncated stream
# has no summary. Test output reaches the stream as `# …` lines too, but only before the last plan line.
tap_verdict() {
	local file=$1 min=$2 summary key count n tests=""
	[[ -s "$file" ]] || { echo "the Lorekeeper smoke wrote no TAP stream ($file is missing or empty)" >&2; return 1; }
	if ! grep -Eq '^1\.\.[0-9]+$' "$file"; then
		echo "the Lorekeeper smoke's TAP stream has no top-level plan line (truncated?): $file" >&2
		return 1
	fi
	summary="$(awk '/^1\.\.[0-9]+$/ { s = ""; next } { s = s $0 "\n" } END { printf "%s", s }' "$file")"
	for key in tests pass fail cancelled skipped todo; do
		count="$(grep -Ec "^# $key [0-9]+$" <<<"$summary" || true)"
		if [[ "$count" != 1 ]]; then
			echo "the Lorekeeper smoke's TAP summary has $count '# $key N' lines after its last plan line, wanted exactly one: $file" >&2
			return 1
		fi
		n="$(sed -En "s/^# $key ([0-9]+)$/\1/p" <<<"$summary")"
		case "$key" in
		tests)
			tests=$n
			if ((n < min)); then
				echo "the Lorekeeper smoke reported '# tests $n', wanted at least $min; a test stopped running" >&2
				return 1
			fi
			;;
		pass)
			if ((n != tests)); then
				echo "the Lorekeeper smoke reported '# pass $n' of '# tests $tests'; every test must pass" >&2
				return 1
			fi
			;;
		*)
			if ((n != 0)); then
				echo "the Lorekeeper smoke reported '# $key $n'; every test must run against the server and pass" >&2
				return 1
			fi
			;;
		esac
	done
}

# seed_sprints REEVE: what only a person can do (kb/ADR-0014 R5), so the automation token's smoke has sprints to read and
# goals to link and suggest on: a sprint closed with a confidence and a verdict, then the active one with a goal and a recorded confidence
# (opening it leaves the next draft). The closed sprint's planned end is a week from today, before the active one's
# (the default length), so the active sprint's promise range (kb/ADR-0025 R8) holds the date seed_promise gives
# `this-sprint`. Run with an interactive token in REEVE_TOKEN; any refusal or error stops the smoke.
seed_sprints() {
	local reeve=$1 out week
	week="$(node -e 'const d = new Date(); d.setUTCDate(d.getUTCDate() + 7); process.stdout.write(d.toISOString().slice(0, 10))')"
	sp() {
		out="$("$reeve" --json "$@")" || { echo "seeding the sprints: reeve $* answered $out" >&2; return 1; }
		node -e 'const e = JSON.parse(process.argv[1]); process.stdout.write(String(e.value.goal ? e.value.goal.id : e.value.id))' "$out"
	}
	local closed active
	closed="$(sp goal add --outcome "Skill smoke closed goal" --check "the smoke reads its verdict" --idempotency-key skill-smoke:sprint:1)" || return 1
	sp sprint open --ends "$week" >/dev/null || return 1
	sp goal confidence "$closed" on_track "seeded by the skill smoke" >/dev/null || return 1
	sp sprint close --verdict "$closed=met:seeded by the skill smoke" >/dev/null || return 1
	active="$(sp goal add --outcome "Skill smoke goal" --check "the smoke links work to it" --idempotency-key skill-smoke:sprint:2)" || return 1
	sp sprint open >/dev/null || return 1
	sp goal confidence "$active" on_track "seeded by the skill smoke" >/dev/null || return 1
}

# seed_promise REEVE: a story promised by the active sprint's end and an epic (migrated from PRM-1) promised by the next
# sprint's end (kb/ADR-0025 R2: only a person promises), so Lorekeeper's `story ls --promised` and `epic show PRM-1`
# entries read a promise. Run after seed_sprints (the shortcuts need the active sprint and the draft), with an
# interactive token in REEVE_TOKEN; any refusal or error stops the smoke.
seed_promise() {
	local reeve=$1 out key
	pr() {
		out="$("$reeve" --json "$@")" || { echo "seeding the promises: reeve $* answered $out" >&2; return 1; }
		node -e 'process.stdout.write(JSON.parse(process.argv[1]).value.key)' "$out"
	}
	key="$(pr story create --title "Skill smoke promised story" --kind chore --criterion "It is promised" \
		--idempotency-key skill-smoke:promise:1)" || return 1
	pr story promise "$key" --on this-sprint --to "the skill smoke" >/dev/null || return 1
	key="$(pr epic create --title "Skill smoke promised epic" --external-ref PRM-1 --idempotency-key skill-smoke:promise:2)" || return 1
	pr epic promise "$key" --on next-sprint --to "the skill smoke" >/dev/null || return 1
}

# Installed before anything is created: it stops what this run started and drops only the database this run created.
# `cleanup STATUS` is the TERM trap: bash runs the EXIT trap after a SIGTERM in `wait` with $? = 0, which would skip the
# server log's tail, so the signal's status is passed in. The traps are cleared first, so it runs once.
cleanup() {
	local status=${1:-$?}
	trap - EXIT TERM
	if [[ -n "$tests_pid" ]]; then
		# The tests run in a process group of their own (set -m in main): end node --test and its children with it.
		kill -- -"$tests_pid" 2>/dev/null || true
		wait "$tests_pid" 2>/dev/null || true
	fi
	if [[ -n "$pid" ]]; then
		kill "$pid" 2>/dev/null || true
		wait "$pid" 2>/dev/null || true
	fi
	if [[ -n "$cli_dir" ]]; then rm -rf "$cli_dir"; fi
	# The config holds the admin password on the DATABASE_URL route: never left behind.
	rm -f "$config"
	if [[ $status -ne 0 && -f "$work/server.log" ]]; then
		echo "--- the last 40 lines of $work/server.log" >&2
		tail -n 40 "$work/server.log" >&2 || true
	fi
	if [[ -n "$db_created" ]]; then
		admin_sql "DROP DATABASE IF EXISTS \"$db_name\" WITH (FORCE)" >/dev/null ||
			echo "warning: could not drop the smoke's database $db_name; drop it by hand" >&2
	fi
	exit "$status"
}

main() {
	cd "$root"
	local lorekeeper="${LOREKEEPER_DIR:-$root/../lorekeeper}" tool db env_lines

	# Everything that can be missing is checked before the build.
	if [[ ! -f "$lorekeeper/scripts/__tests__/reeve-real-server.test.js" ]]; then
		echo "no Lorekeeper checkout with the skill smoke at $lorekeeper; clone it next to this repository:" >&2
		echo "  git clone https://github.com/Mindful-Stack/lorekeeper.git \"$root/../lorekeeper\"" >&2
		echo "or set LOREKEEPER_DIR to a checkout that has scripts/__tests__/reeve-real-server.test.js" >&2
		exit 1
	fi
	lorekeeper="$(cd "$lorekeeper" && pwd)"
	command -v node >/dev/null || { echo "node 22 or later is required (mise installs it: mise install)" >&2; exit 1; }
	node -e 'process.exit(Number(process.versions.node.split(".")[0]) >= 22 ? 0 : 1)' ||
		{ echo "node 22 or later is required, found $(node --version)" >&2; exit 1; }
	for tool in cargo curl git; do
		command -v "$tool" >/dev/null || { echo "$tool is required and is not on PATH" >&2; exit 1; }
	done

	if [[ -n "${DATABASE_URL:-}" && ("${CI:-}" == "true" || "${REEVE_SMOKE_ALLOW_DATABASE_URL:-}" == "1") ]]; then
		use_url=1
		if command -v psql >/dev/null; then
			admin_via=psql
		elif command -v docker >/dev/null; then
			admin_via=docker
		else
			echo "the DATABASE_URL route runs psql, the host's or the postgres:16 image's: install psql or docker" >&2
			exit 1
		fi
	else
		if [[ -n "${DATABASE_URL:-}" ]]; then
			echo "warning: ignoring the exported DATABASE_URL (set REEVE_SMOKE_ALLOW_DATABASE_URL=1 to use its server); using the compose Postgres" >&2
		fi
		if ! docker compose version >/dev/null 2>&1; then
			printf '%s\n' >&2 \
				"the compose PostgreSQL needs the docker compose plugin, which is not installed; use a throwaway server:" \
				"  docker run --rm -d --name reeve-skill-smoke-pg -e POSTGRES_PASSWORD=postgres -p 127.0.0.1:55432:5432 postgres:16" \
				"  REEVE_SMOKE_ALLOW_DATABASE_URL=1 DATABASE_URL=postgres://postgres:postgres@127.0.0.1:55432/postgres mise run skill-smoke" \
				"  docker rm -fv reeve-skill-smoke-pg"
			exit 1
		fi
		admin_via=compose
	fi
	# A leftover server on the port would answer the health check while the new one fails to bind.
	if (exec 3<>/dev/tcp/127.0.0.1/7476) 2>/dev/null; then
		echo "port 7476 is already in use; stop whatever listens there (a previous skill smoke?) and retry" >&2
		exit 1
	fi

	db_name="reeve_skill_smoke_$$_$(node -e 'process.stdout.write(require("node:crypto").randomBytes(4).toString("hex"))')"
	# Both connections are worked out before anything is built or created: a bad DATABASE_URL fails here, creating nothing.
	if [[ -n "$use_url" ]]; then
		env_lines="$(admin_pg_env "$DATABASE_URL")" || exit 1
		mapfile -t pg_env <<<"$env_lines"
		db="$(smoke_database_url "$DATABASE_URL" "$db_name")" || exit 1
	else
		db="postgres://reeve:reeve@localhost:5433/$db_name"
	fi

	trap cleanup EXIT
	trap 'cleanup 143' TERM

	rm -rf "$work"
	mkdir -p "$work"

	log "building reeve-server (static-oidc) and reeve"
	cargo build -p reeve-server -p reeve --features reeve-server/static-oidc
	local server="$root/target/debug/reeve-server" reeve="$root/target/debug/reeve"

	if [[ -z "$use_url" ]]; then
		log "starting the compose Postgres"
		mise run db-up
	fi
	# A server that just started (a throwaway container, the compose service) refuses connections for a few seconds.
	log "waiting for the database server"
	local ready=""
	for _ in $(seq 30); do
		if admin_sql 'SELECT 1' >/dev/null 2>&1; then ready=1; break; fi
		sleep 1
	done
	[[ -n "$ready" ]] || { admin_sql 'SELECT 1' || true; echo "the database server never accepted a connection" >&2; exit 1; }

	log "creating the smoke's own database $db_name"
	create_smoke_database

	# Escape sed's replacement specials (`\`, `&` and the `|` delimiter) so a URL such as `?sslmode=require&...` survives.
	# Owner-only: on the DATABASE_URL route the config holds the admin password (the cleanup removes it).
	local db_sed
	db_sed="$(printf '%s' "$db" | sed -e 's/[\\&|]/\\&/g')"
	(umask 077 && sed "s|@DATABASE_URL@|$db_sed|" reeve.skill-smoke.toml >"$config")
	export REEVE_CONFIG="$config"

	log "starting reeve-server on $url"
	RUST_LOG="${RUST_LOG:-info,reeve_server=debug,reeve_identity=debug}" "$server" serve >"$work/server.log" 2>&1 &
	pid=$!
	cli_dir="$(mktemp -d)"

	for _ in $(seq 60); do
		curl -fsS --max-time 2 "$url/health" >/dev/null 2>&1 && break
		if ! kill -0 "$pid" 2>/dev/null; then
			echo "reeve-server exited before it was healthy" >&2
			exit 1
		fi
		sleep 1
	done
	curl -fsS --max-time 2 "$url/health" >/dev/null || { echo "reeve-server never became healthy" >&2; exit 1; }

	log "issuing an automation token for $email"
	local token status_json
	token="$("$server" token-issue --email "$email" --kind automation --label skill-smoke)"
	# An empty CLI config: nothing of the developer's own login is read.
	export REEVE_CONFIG_DIR="$cli_dir"
	status_json="$(REEVE_URL="$url" REEVE_TOKEN="$token" "$reeve" --json auth status || true)"
	node -e '
		let env = {};
		try { env = JSON.parse(process.argv[1]); } catch {}
		const v = env.value || {};
		if (env.outcome !== "ok" || v.role !== "admin" || v.token_kind !== "automation" || v.token_from_env !== true) {
			console.error("the smoke needs an admin automation token from REEVE_TOKEN; reeve auth status answered " + process.argv[1]);
			process.exit(1);
		}' "$status_json"

	log "seeding the sprints with an interactive token for $email"
	local person
	person="$("$server" token-issue --email "$email" --kind interactive --label skill-smoke-person)"
	REEVE_URL="$url" REEVE_TOKEN="$person" seed_sprints "$reeve" || exit 1
	REEVE_URL="$url" REEVE_TOKEN="$person" seed_promise "$reeve" || exit 1

	log "building the household: $work/household/kb with an accepted record on main"
	local household="$work/household"
	mkdir -p "$household/kb/adrs"
	printf '%s\n' '---' 'id: kb/ADR-0001' 'status: accepted' '---' "# ADR-0001: The skill smoke's record" >"$household/kb/adrs/0001-x.md"
	# Git without the developer's or the system's configuration (signing, hooks, default branch) and with a fixed identity.
	g() {
		GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 GIT_AUTHOR_NAME=skill-smoke GIT_AUTHOR_EMAIL="$email" \
			GIT_COMMITTER_NAME=skill-smoke GIT_COMMITTER_EMAIL="$email" git "$@"
	}
	g -C "$household/kb" init -q -b main
	g -C "$household/kb" add -A
	g -C "$household/kb" commit -q -m 'accept ADR-0001'
	g clone -q --bare "$household/kb" "$work/kb-origin.git"
	g -C "$household/kb" remote add origin "$work/kb-origin.git"
	g -C "$household/kb" fetch -q origin
	g -C "$household/kb" remote set-head origin -a >/dev/null

	node -e '
		const [file, household, email] = process.argv.slice(1);
		const seed = { household, email, repo: "kb", record: { id: "kb/ADR-0001", path: "adrs/0001-x.md" } };
		require("node:fs").writeFileSync(file, JSON.stringify(seed, null, 2) + "\n");' "$work/seed.json" "$household" "$email"

	log "running the Lorekeeper skill smoke in $lorekeeper"
	local tap="$work/lorekeeper.tap"
	# In the background, in a process group of its own (set -m), and waited on: a signal to this script alone runs the
	# cleanup, which ends the tests too instead of leaving them running against a stopped server.
	set -m
	(
		cd "$lorekeeper"
		REEVE_REAL_BIN="$reeve" REEVE_URL="$url" REEVE_TOKEN="$token" REEVE_SMOKE_SEED="$work/seed.json" \
			REEVE_SMOKE_OUT="$work/answers" \
			exec node --test --test-reporter=spec --test-reporter-destination=stdout \
			--test-reporter=tap --test-reporter-destination="$tap" \
			scripts/__tests__/reeve-real-server.test.js scripts/__tests__/reeve-reference.test.js
	) &
	tests_pid=$!
	set +m
	wait "$tests_pid"
	tests_pid=""
	tap_verdict "$tap" "$min_tests" || exit 1
	log "the skill smoke passed; every answer is in $work/answers"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
	main "$@"
fi
