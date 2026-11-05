#!/usr/bin/env bash
# scripts/tests/skill-smoke.sh — the skill smoke's safety logic, sourced without running the smoke (no Docker, server or
# Lorekeeper needed): a failed CREATE drops nothing and a successful one drops exactly its own database; the database
# URL rewrite; the admin connection kept off the command line; the TAP verdict. Also lints both smoke scripts.
# The smoke's code runs in a child bash, so `$1` in single quotes is meant.
# shellcheck disable=SC2016
set -euo pipefail
cd "$(dirname "$0")/../.."
script=scripts/skill-smoke.sh
bash -n "$script"
bash -n scripts/web-smoke.sh
bash -n scripts/test.sh
if command -v shellcheck >/dev/null 2>&1; then
	shellcheck "$script" scripts/web-smoke.sh scripts/test.sh scripts/tests/skill-smoke.sh
	echo "shellcheck ok"
elif [ -n "${CI:-}" ]; then
	echo "shellcheck required in CI" >&2
	exit 1
else
	echo "shellcheck not installed: syntax checked with bash -n only"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fail() {
	echo "FAIL: $*" >&2
	exit 1
}

# in_smoke CODE [ARG...]: run CODE in a fresh bash with the smoke's functions sourced (main does not run) and ARGs as
# its positional parameters; stdout to $tmp/out, stderr to $tmp/err, the exit status in $status.
in_smoke() {
	status=0
	SCRIPT="$script" bash -c 'source "$SCRIPT"; '"$1" bash "${@:2}" >"$tmp/out" 2>"$tmp/err" || status=$?
}

# --- the database the smoke creates is the only one it drops ----------------------------------------------------------
# admin_sql logs every statement; CREATE fails when FAIL_CREATE is set (a name that already exists).
lifecycle='
	admin_sql() { printf "%s\n" "$1" >>"$SQL_LOG"; [[ "$1" != CREATE* || -z "${FAIL_CREATE:-}" ]]; }
	db_name=reeve_skill_smoke_1_cafe
	work="$WORK"
	config="$WORK/reeve.toml"
	trap cleanup EXIT
	create_smoke_database
	echo created
	false'
mkdir -p "$tmp/work"
export SQL_LOG="$tmp/sql" WORK="$tmp/work"

# 1. the CREATE fails: the run fails, and no DROP follows
rm -f "$tmp/sql"
FAIL_CREATE=1 in_smoke "$lifecycle"
[ "$status" -ne 0 ] || fail "a failed CREATE did not fail the run"
[ "$(cat "$tmp/sql")" = 'CREATE DATABASE "reeve_skill_smoke_1_cafe"' ] ||
	fail "a failed CREATE must be followed by no statement; got: $(cat "$tmp/sql")"

# 2. the CREATE succeeds and a later step fails: exactly one DROP, of that database; the config is removed
rm -f "$tmp/sql"
printf 'database_url = "postgres://u:secret@h/x"\n' >"$tmp/work/reeve.toml"
in_smoke "$lifecycle"
[ "$status" -eq 1 ] || fail "wanted exit 1 after a failure past the CREATE, got $status"
grep -qx created "$tmp/out" || fail "the CREATE did not succeed in case 2"
want='CREATE DATABASE "reeve_skill_smoke_1_cafe"
DROP DATABASE IF EXISTS "reeve_skill_smoke_1_cafe" WITH (FORCE)'
[ "$(cat "$tmp/sql")" = "$want" ] || fail "wanted one CREATE then one DROP of its own database; got: $(cat "$tmp/sql")"
[ ! -e "$tmp/work/reeve.toml" ] || fail "the cleanup left the config (it holds the admin password)"

# 3. nothing created yet (an early failure): no statement at all
rm -f "$tmp/sql"
in_smoke 'admin_sql() { printf "%s\n" "$1" >>"$SQL_LOG"; }; work="$WORK"; config="$WORK/reeve.toml"; trap cleanup EXIT; false'
[ "$status" -eq 1 ] || fail "wanted exit 1, got $status"
[ ! -e "$tmp/sql" ] || fail "an early failure ran SQL: $(cat "$tmp/sql")"

# 4. a SIGTERM while waiting (a CI cancel): the cleanup reports the failure with the server log's tail, exit 143
rm -f "$tmp/sql"
printf 'the server said why\n' >"$tmp/work/server.log"
in_smoke 'admin_sql() { printf "%s\n" "$1" >>"$SQL_LOG"; }; work="$WORK"; config="$WORK/reeve.toml"; db_name=x
	trap cleanup EXIT; trap "cleanup 143" TERM
	(sleep 0.2; kill -TERM $$) & sleep 5 & wait $!'
[ "$status" -eq 143 ] || fail "wanted exit 143 after SIGTERM, got $status"
grep -qx 'the server said why' "$tmp/err" || fail "a SIGTERM skipped the server log's tail: $(cat "$tmp/err")"
[ "$(grep -c 'last 40 lines' "$tmp/err")" = 1 ] || fail "the cleanup ran more than once: $(cat "$tmp/err")"
rm -f "$tmp/work/server.log"
echo "create/drop ok"

# --- the URL rewrite ---------------------------------------------------------------------------------------------------
# rewrite URL WANT: smoke_database_url URL NAME prints exactly WANT
rewrite() {
	in_smoke 'smoke_database_url "$1" reeve_skill_smoke_1_cafe' "$1"
	[ "$status" -eq 0 ] || fail "the rewrite refused $1: $(cat "$tmp/err")"
	[ "$(cat "$tmp/out")" = "$2" ] || fail "rewrite of $1: wanted $2, got $(cat "$tmp/out")"
}
rewrite 'postgres://postgres:secret@127.0.0.1:5432/postgres' \
	'postgres://postgres:secret@127.0.0.1:5432/reeve_skill_smoke_1_cafe'
rewrite 'postgres://postgres:secret@db:5432/postgres?dbname=postgres&sslmode=disable&dbname=prod' \
	'postgres://postgres:secret@db:5432/reeve_skill_smoke_1_cafe?sslmode=disable'
rewrite 'postgres://postgres:secret@db/%70rod?%64bname=prod&sslmode=require&options=-c%20x%3D1' \
	'postgres://postgres:secret@db/reeve_skill_smoke_1_cafe?sslmode=require&options=-c+x%3D1'
rewrite 'postgresql://u:p%40ss@[::1]:6543/x?sslmode=verify-full' \
	'postgresql://u:p%40ss@[::1]:6543/reeve_skill_smoke_1_cafe?sslmode=verify-full'

# refused URL: refused with the fixed message, never echoing the input or its password
refused_msg='DATABASE_URL must be a postgres:// or postgresql:// URL with a host; refusing (the value is not printed)'
refused() {
	local fn
	for fn in 'smoke_database_url "$1" reeve_skill_smoke_1_cafe' 'admin_pg_env "$1"'; do
		in_smoke "$fn" "$1"
		[ "$status" -ne 0 ] || fail "${fn%% *} accepted $1"
		[ ! -s "$tmp/out" ] || fail "${fn%% *} printed something for $1: $(cat "$tmp/out")"
		[ "$(cat "$tmp/err")" = "$refused_msg" ] || fail "${fn%% *} on $1: wanted the fixed message, got: $(cat "$tmp/err")"
	done
}
refused 'host=127.0.0.1 user=postgres password=secret dbname=postgres'
refused 'postgres:secret'
refused 'mysql://root:secret@db/prod'
refused 'postgres:///postgres?password=secret'
refused 'not a url secret'
# connection parameters in the query, which sqlx honours: the admin connection and the server would part ways
refused 'postgres://u@localhost/postgres?host=/var/run/postgresql&password=secret'
refused 'postgres://u:secret@db/postgres?port=6543'
refused 'postgres://u:secret@db/postgres?user=admin'
refused 'postgres://u@db/postgres?sslmode=require&password=secret'
refused 'postgres://u:secret@db/postgres?hostaddr=10.0.0.1'
echo "url rewrite ok"

# --- the admin connection: libpq variables, never a URL or password in argv --------------------------------------------
in_smoke 'admin_pg_env "$1"' 'postgres://ad%40min:s%3Acret@db.example:6543/postgres?dbname=admin&sslmode=require'
[ "$status" -eq 0 ] || fail "admin_pg_env refused a valid URL: $(cat "$tmp/err")"
[ "$(cat "$tmp/out")" = 'PGHOST=db.example
PGPORT=6543
PGUSER=ad@min
PGPASSWORD=s:cret
PGDATABASE=admin
PGSSLMODE=require' ] || fail "admin_pg_env: got $(cat "$tmp/out")"

# fakes for psql and docker that log their argv and the PG variables they see
mkdir -p "$tmp/bin"
for tool in psql docker; do
	cat >"$tmp/bin/$tool" <<'EOF'
#!/usr/bin/env bash
{
	printf 'argv:'
	printf ' %s' "$(basename "$0")" "$@"
	printf '\n'
	env | grep '^PG' | sort
} >>"$FAKE_LOG"
EOF
	chmod +x "$tmp/bin/$tool"
done
export FAKE_LOG="$tmp/fake"
for via in psql docker; do
	rm -f "$tmp/fake"
	PATH="$tmp/bin:$PATH" in_smoke '
		admin_via=$2
		mapfile -t pg_env < <(admin_pg_env "$1")
		admin_sql "SELECT 1"
		env | grep "^PG" || true' 'postgres://admin:hunter2@db:5433/postgres' "$via"
	[ "$status" -eq 0 ] || fail "admin_sql via $via failed: $(cat "$tmp/err")"
	grep -q hunter2 <(grep '^argv:' "$tmp/fake") && fail "the admin password is in $via's argv: $(cat "$tmp/fake")"
	grep -qx 'PGPASSWORD=hunter2' "$tmp/fake" || fail "$via did not get PGPASSWORD: $(cat "$tmp/fake")"
	grep -qx 'PGDATABASE=postgres' "$tmp/fake" || fail "$via did not get PGDATABASE: $(cat "$tmp/fake")"
	[ ! -s "$tmp/out" ] || fail "admin_sql exported the PG variables into the script itself: $(cat "$tmp/out")"
done
grep -qx 'argv: docker run --rm --network host -e PGHOST -e PGPORT -e PGUSER -e PGPASSWORD -e PGDATABASE postgres:16 psql -q -v ON_ERROR_STOP=1 -c SELECT 1' "$tmp/fake" ||
	fail "docker is not given the variables by name: $(cat "$tmp/fake")"
echo "admin connection ok"

# --- the TAP verdict ---------------------------------------------------------------------------------------------------
summary() { # summary TESTS PASS FAIL CANCELLED SKIPPED TODO
	printf '# tests %s\n# suites 0\n# pass %s\n# fail %s\n# cancelled %s\n# skipped %s\n# todo %s\n# duration_ms 8512.0\n' "$@"
}
body() {
	printf 'TAP version 13\n# Subtest: a\n'
	printf '# skipped 3\n# todo 2\n' # a test's own stdout, which node writes as comments at column 0
	printf '    # Subtest: row\n    ok 1 - row\n    1..19\nok 1 - a\n'
}
# verdict WANT_STATUS NAME: tap_verdict on $tmp/tap.NAME with a floor of 28
verdict() {
	in_smoke 'tap_verdict "$1" 28' "$tmp/tap.$2"
	if [ "$1" -eq 0 ]; then
		[ "$status" -eq 0 ] || fail "the verdict rejected $2: $(cat "$tmp/err")"
	else
		[ "$status" -ne 0 ] || fail "the verdict accepted $2"
		[ -s "$tmp/err" ] || fail "the verdict rejected $2 without saying why"
	fi
}
{ body; printf '1..9\n'; summary 28 28 0 0 0 0; } >"$tmp/tap.full"
{ body; printf '1..9\n'; summary 31 31 0 0 0 0; } >"$tmp/tap.more"
{ body; printf '1..9\n'; summary 28 27 0 0 1 0; } >"$tmp/tap.skipped"
{ body; printf '1..9\n'; summary 28 28 0 0 1 0; } >"$tmp/tap.skippedline" # the skipped line alone decides
{ body; printf '1..9\n'; summary 28 28 0 0 0 1; } >"$tmp/tap.todo"
{ body; printf '1..9\n'; summary 28 27 1 0 0 0; } >"$tmp/tap.fail"
{ body; printf '1..9\n'; summary 28 27 0 1 0 0; } >"$tmp/tap.cancelled"
{ body; printf '1..9\n'; summary 28 25 0 0 0 0; } >"$tmp/tap.passshort"
{ body; printf '1..9\n'; summary 9 9 0 0 0 0; } >"$tmp/tap.fewer"
{ printf 'TAP version 13\n1..0\n'; summary 0 0 0 0 0 0; } >"$tmp/tap.zero"
{ body; printf '# skipped 0\n# todo 0\n# fail 0\n'; } >"$tmp/tap.truncated"
{ body; printf '1..9\n# tests 28\n# pass 28\n'; } >"$tmp/tap.cut"
{ body; printf '1..9\n'; summary 28 28 0 0 0 0; printf '# skipped 1\n'; } >"$tmp/tap.twice"
{ body; printf '1..9\n'; summary 28 28 0 0 0 0 | sed 's/^# skipped 0$/# skipped/'; } >"$tmp/tap.blank"
: >"$tmp/tap.empty"
verdict 0 full
verdict 0 more
for bad in skipped skippedline todo fail cancelled passshort fewer zero truncated cut twice blank empty missing; do
	verdict 1 "$bad"
done
echo "tap verdict ok"
echo "skill smoke test ok"
