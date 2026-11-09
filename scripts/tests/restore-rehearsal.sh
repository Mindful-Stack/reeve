#!/usr/bin/env bash
# scripts/tests/restore-rehearsal.sh — the rehearsal's control flow against scripts/tests/fake-compose.sh: the
# documented commands in order, the dump streamed to pg_restore, exit 0/1/2, the scratch database dropped after every
# run that created it and never otherwise, and a failed drop reported as a failure. The PostgreSQL commands themselves
# are proven by crates/reeve-server/tests/backup_restore.rs.
set -euo pipefail
cd "$(dirname "$0")/../.."
script=scripts/restore-rehearsal.sh
bash -n "$script"
if command -v shellcheck >/dev/null 2>&1; then
	shellcheck -x "$script" scripts/tests/restore-rehearsal.sh scripts/tests/fake-compose.sh
	echo "shellcheck ok"
elif [ -n "${CI:-}" ]; then
	echo "shellcheck required in CI" >&2
	exit 1
else
	echo "shellcheck not installed: syntax checked with bash -n only"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export FAKE_DIR="$tmp"
cp scripts/tests/fake-compose.sh "$tmp/fake-compose"
fake="$tmp/fake-compose"

printf 'PGDMP fake archive\n' >"$tmp/live.dump"
doc() { printf '{\n  "activity": [],\n  "exported_at": "%s",\n  "key_prefix": "%s"\n}\n' "$1" "$2"; }
doc 2026-10-26T10:00:00Z ABC >"$tmp/live.json"

# run <expected status> [VAR=value ...]: a fresh call log, the script with the fake, fails on any other status.
run() {
	local want=$1 s=0
	shift
	rm -f "$tmp/calls" "$tmp/restore-stdin"
	env REEVE_COMPOSE="$fake" "$@" bash "$script" "$tmp/live.dump" "$tmp/live.json" >"$tmp/out" 2>&1 || s=$?
	if [ "$s" -ne "$want" ]; then
		echo "wanted exit $want, got $s; output:" >&2
		cat "$tmp/out" >&2
		exit 1
	fi
}
calls() { cat "$tmp/calls" 2>/dev/null || true; }
expect_calls() {
	if [ "$(calls)" != "$1" ]; then
		printf 'calls differ.\nwanted:\n%s\ngot:\n%s\n' "$1" "$(calls)" >&2
		exit 1
	fi
}
expect_output() { grep -q -- "$1" "$tmp/out" || { echo "output lacks '$1':" >&2; cat "$tmp/out" >&2; exit 1; }; }
all=$'exec -T postgres createdb -U reeve -O reeve reeve_rehearsal
exec -T postgres pg_restore -U reeve -d reeve_rehearsal --no-owner --exit-on-error
exec -T server reeve-server export --help
exec -T server reeve-server export --database reeve_rehearsal --at 2026-10-26T10:00:00Z
exec -T postgres dropdb -U reeve reeve_rehearsal'

# 1. documents equal apart from exported_at: 0, the README's commands in order, the dump on pg_restore's stdin
doc 2026-10-26T10:05:00Z ABC >"$tmp/export-output"
run 0
expect_calls "$all"
cmp -s "$tmp/live.dump" "$tmp/restore-stdin" || { echo "the dump was not streamed to pg_restore" >&2; exit 1; }

# 2. documents differ: 1, the diff printed, the scratch database still dropped
doc 2026-10-26T10:05:00Z TST >"$tmp/export-output"
run 1
expect_calls "$all"
expect_output '"key_prefix": "TST"'

# 3. the live database's name: 2, nothing run
run 2 REEVE_REHEARSAL_DB=reeve
expect_calls ""

# 4. the scratch database exists already (createdb fails): 2, never dropped
run 2 FAKE_CREATEDB=1
expect_calls "exec -T postgres createdb -U reeve -O reeve reeve_rehearsal"

# 5. the restore fails: 2, the scratch database dropped
run 2 FAKE_RESTORE=1
expect_calls $'exec -T postgres createdb -U reeve -O reeve reeve_rehearsal
exec -T postgres pg_restore -U reeve -d reeve_rehearsal --no-owner --exit-on-error
exec -T postgres dropdb -U reeve reeve_rehearsal'

# 6. matching documents but the drop fails: 2, not a success that leaves the scratch database behind
doc 2026-10-26T10:05:00Z ABC >"$tmp/export-output"
run 2 FAKE_DROPDB=1
expect_calls "$all"
expect_output "could not drop reeve_rehearsal"

# 7. a missing argument: 2, nothing run (a fresh call log: this case does not go through run)
rm -f "$tmp/calls"
s=0
REEVE_COMPOSE="$fake" bash "$script" "$tmp/live.dump" >/dev/null 2>&1 || s=$?
[ "$s" -eq 2 ] || { echo "a missing argument exited $s, wanted 2" >&2; exit 1; }
[ ! -e "$tmp/calls" ] || { echo "a missing argument ran commands" >&2; exit 1; }

# 8. the export fails: 2, every step called and the scratch database dropped
run 2 FAKE_EXPORT=1
expect_calls "$all"

# 9. a missing live export: 2, nothing run
s=0
rm -f "$tmp/calls"
REEVE_COMPOSE="$fake" bash "$script" "$tmp/live.dump" "$tmp/absent.json" >/dev/null 2>&1 || s=$?
[ "$s" -eq 2 ] || { echo "a missing live export exited $s, wanted 2" >&2; exit 1; }
[ ! -e "$tmp/calls" ] || { echo "a missing live export ran commands" >&2; exit 1; }

# 10. an empty restored document: 2 (never a vacuous match), the scratch database dropped
: >"$tmp/export-output"
run 2
expect_calls "$all"
expect_output "is empty"

# 11. an empty live document: 2, refused before anything runs (no scratch database is created)
doc 2026-10-26T10:05:00Z ABC >"$tmp/export-output"
cp "$tmp/live.json" "$tmp/live.keep"
: >"$tmp/live.json"
run 2
expect_calls ""
expect_output "is empty"
cp "$tmp/live.keep" "$tmp/live.json"

# 12. a document without its exported_at line: 2
printf '{\n  "key_prefix": "ABC"\n}\n' >"$tmp/export-output"
run 2
expect_output "exactly one exported_at"
doc 2026-10-26T10:05:00Z ABC >"$tmp/export-output"

# 12b. a document holding only the exported_at line: 2, with an accurate message
printf '{\n  "exported_at": "2026-10-26T10:05:00Z"\n}\n' | grep -v '^[{}]' >"$tmp/export-output"
run 2
expect_output "nothing but its exported_at"
doc 2026-10-26T10:05:00Z ABC >"$tmp/export-output"

# 13. an invalid scratch database name: 2, nothing run
run 2 'REEVE_REHEARSAL_DB=Bad-Name'
expect_calls ""

# 14. a dump path with a space still reaches pg_restore intact
cp "$tmp/live.dump" "$tmp/live copy.dump"
rm -f "$tmp/calls" "$tmp/restore-stdin"
REEVE_COMPOSE="$fake" bash "$script" "$tmp/live copy.dump" "$tmp/live.json" >"$tmp/out" 2>&1
cmp -s "$tmp/live.dump" "$tmp/restore-stdin" || { echo "a dump path with a space was not streamed" >&2; exit 1; }

# 15. the restored document is exported at the live export's exported_at (urgency is computed at exported_at), whatever
# that instant is, a fractional one with an offset included
doc 2026-11-09T08:30:15.123456+00:00 ABC >"$tmp/live.json"
doc 2026-11-20T00:00:00Z ABC >"$tmp/export-output"
run 0
expect_calls $'exec -T postgres createdb -U reeve -O reeve reeve_rehearsal
exec -T postgres pg_restore -U reeve -d reeve_rehearsal --no-owner --exit-on-error
exec -T server reeve-server export --help
exec -T server reeve-server export --database reeve_rehearsal --at 2026-11-09T08:30:15.123456+00:00
exec -T postgres dropdb -U reeve reeve_rehearsal'

# 16. a live export whose exported_at is not a time (or is missing): 2, refused before anything runs
for bad in 'yesterday' '2026-11-09T08:30:15Z; rm -rf /'; do
	doc "$bad" ABC >"$tmp/live.json"
	run 2
	expect_calls ""
done
printf '{\n  "key_prefix": "ABC"\n}\n' >"$tmp/live.json"
run 2
expect_calls ""
expect_output "exactly one exported_at"
doc 2026-10-26T10:00:00Z ABC >"$tmp/live.json"

# 17. a server image from before urgent marks has no `export --at` (and computes no urgency): the export runs without it
doc 2026-10-26T10:05:00Z ABC >"$tmp/export-output"
run 0 FAKE_NO_AT=1
expect_calls $'exec -T postgres createdb -U reeve -O reeve reeve_rehearsal
exec -T postgres pg_restore -U reeve -d reeve_rehearsal --no-owner --exit-on-error
exec -T server reeve-server export --help
exec -T server reeve-server export --database reeve_rehearsal
exec -T postgres dropdb -U reeve reeve_rehearsal'

# 18. the probe itself fails: 2, no export, the scratch database dropped
run 2 FAKE_HELP=1
expect_calls $'exec -T postgres createdb -U reeve -O reeve reeve_rehearsal
exec -T postgres pg_restore -U reeve -d reeve_rehearsal --no-owner --exit-on-error
exec -T server reeve-server export --help
exec -T postgres dropdb -U reeve reeve_rehearsal'
expect_output "export --help"

echo "restore rehearsal test ok"
