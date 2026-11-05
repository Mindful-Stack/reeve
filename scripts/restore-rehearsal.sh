#!/usr/bin/env bash
# scripts/restore-rehearsal.sh — rehearse a restore without touching the live database (deploy/README.md, "Rehearse
# the restore"): restore a dump into a scratch database, export it, and compare with the export taken with the dump,
# apart from exported_at.
#
#   scripts/restore-rehearsal.sh <dump-file> <live-export.json>
#       the reference compose stack; run from the directory holding docker-compose.yml.
#
# Exit status: 0 the documents match; 1 they differ (the diff is printed); 2 refused or a step failed, the final drop
# of the scratch database included.
# REEVE_REHEARSAL_DB names the scratch database (default reeve_rehearsal; reeve is refused), REEVE_PG_USER the
# PostgreSQL user (default reeve), REEVE_COMPOSE the compose command (default "docker compose"; the tests use fakes).
# shellcheck disable=SC2015 # `a && b || c` is deliberate here: c runs when a or b fails, and c never returns
set -euo pipefail

fail() {
	echo "restore-rehearsal: $*" >&2
	exit 2
}
[ "$#" -eq 2 ] || fail "usage: $0 <dump-file> <live-export.json>"
dump=$1
live=$2
db=${REEVE_REHEARSAL_DB:-reeve_rehearsal}
user=${REEVE_PG_USER:-reeve}
[[ $db =~ ^[a-z_][a-z0-9_]*$ ]] || fail "REEVE_REHEARSAL_DB must be a lower-case name, got '$db'"
[ "$db" != reeve ] || fail "refusing the database named reeve: that is the live database"
read -r -a compose <<<"${REEVE_COMPOSE:-docker compose}"
[ "${#compose[@]}" -gt 0 ] || fail "REEVE_COMPOSE is empty"

# mktemp creates the file exclusively with mode 0600: the export holds every user's email.
scratch=$(mktemp -d) || fail "could not create a temporary directory"
restored=$scratch/restored.json
created=0
cleanup() {
	local status=$?
	rm -rf -- "$scratch"
	if [ "$created" -eq 1 ] && ! "${compose[@]}" exec -T postgres dropdb -U "$user" "$db"; then
		echo "restore-rehearsal: could not drop $db; drop it by hand" >&2
		status=2
	fi
	exit "$status"
}
trap cleanup EXIT
# A signal exits through the trap too (status 128+n would otherwise be kept; any non-zero is a failure here).
trap 'exit 2' INT TERM HUP

if [ ! -f "$dump" ] || [ ! -r "$dump" ]; then fail "cannot read the dump $dump"; fi
if [ ! -f "$live" ] || [ ! -r "$live" ]; then fail "cannot read the live export $live"; fi
[ -s "$live" ] || fail "the live export $live is empty"

# createdb fails when the database exists, so this run only ever drops a database it created itself.
"${compose[@]}" exec -T postgres createdb -U "$user" -O "$user" "$db" ||
	fail "could not create $db (does it exist already?)"
created=1
"${compose[@]}" exec -T postgres pg_restore -U "$user" -d "$db" --no-owner --exit-on-error <"$dump" ||
	fail "pg_restore into $db failed"
"${compose[@]}" exec -T server reeve-server export --database "$db" >"$restored" ||
	fail "the offline export of $db failed"

# Both documents must be non-empty and carry exactly one exported_at line, or the comparison could pass vacuously.
strip() {
	[ -s "$1" ] || fail "$2 is empty"
	[ "$(grep -c '^  "exported_at": ' "$1")" -eq 1 ] || fail "$2 does not hold exactly one exported_at line"
	# grep -v exits 1 when nothing is left; that is checked below, any other status is an error
	local g=0
	grep -v '^  "exported_at": ' "$1" >"$3" || g=$?
	[ "$g" -le 1 ] || fail "could not strip exported_at from $2"
	[ -s "$3" ] || fail "$2 holds nothing but its exported_at line"
}
strip "$live" "the live export" "$scratch/live.stripped"
strip "$restored" "the restored export" "$scratch/restored.stripped"
status=0
diff "$scratch/live.stripped" "$scratch/restored.stripped" || status=$?
case "$status" in
0) echo "restore rehearsal: the restored instance matches the live export" ;;
1)
	echo "restore rehearsal: the restored instance differs from the live export (diff above)" >&2
	exit 1
	;;
*) fail "diff failed (status $status)" ;;
esac
