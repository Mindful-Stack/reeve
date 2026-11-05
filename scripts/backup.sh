#!/usr/bin/env bash
# scripts/backup.sh — back up the reference compose stack (deploy/README.md, "Back up"): a pg_dump archive and the
# offline export beside it, published only when both succeeded. Both read one exported snapshot (a psql session held
# open as a coprocess while they run), so a write between them cannot make a pair whose restored export differs. Run from the directory holding docker-compose.yml;
# this is the command the host's backup script runs.
#
#   scripts/backup.sh <directory>
#
# Writes <directory>/reeve-<stamp>.dump and <directory>/reeve-<stamp>.json and prints their paths. Each is written to
# a fresh temporary file in <directory> (created exclusively with mode 0600 by mktemp) and linked to its final name
# only after both commands succeeded and the archive read back whole; ln -T refuses any existing name (a directory or
# a symlink to one included), so nothing is ever overwritten or linked into the wrong place.
# Exit status: 0 both published; 2 refused or failed (nothing published, no temporary left).
# REEVE_PG_USER and REEVE_PG_DATABASE (default reeve), REEVE_COMPOSE (default "docker compose"; the test uses a fake),
# REEVE_BACKUP_STAMP (default the UTC time to the second, date -u +%Y%m%dT%H%M%SZ; the test fixes it).
set -euo pipefail

fail() {
	echo "backup: $*" >&2
	exit 2
}
[ "$#" -eq 1 ] || fail "usage: $0 <directory>"
dir=$1
if [ ! -d "$dir" ] || [ ! -w "$dir" ]; then fail "$dir is not a writable directory"; fi
user=${REEVE_PG_USER:-reeve}
database=${REEVE_PG_DATABASE:-reeve}
stamp=${REEVE_BACKUP_STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}
[[ $stamp =~ ^[0-9A-Za-z]+$ ]] || fail "REEVE_BACKUP_STAMP must be letters and digits only"
read -r -a compose <<<"${REEVE_COMPOSE:-docker compose}"
dump="$dir/reeve-$stamp.dump"
json="$dir/reeve-$stamp.json"
for f in "$dump" "$json"; do
	[ ! -e "$f" ] || fail "$f exists already; nothing written"
done

tmp_dump=""
tmp_json=""
snap_pid=""
cleanup() {
	local status=$?
	[ -z "$snap_pid" ] || kill "$snap_pid" 2>/dev/null || true
	[ -z "$tmp_dump" ] || rm -f -- "$tmp_dump"
	[ -z "$tmp_json" ] || rm -f -- "$tmp_json"
	exit "$status"
}
trap cleanup EXIT
tmp_dump=$(mktemp "$dir/.reeve-$stamp.dump.XXXXXX") || fail "cannot create a temporary file in $dir"
tmp_json=$(mktemp "$dir/.reeve-$stamp.json.XXXXXX") || fail "cannot create a temporary file in $dir"

# One snapshot for both files (PostgreSQL, "Snapshot Synchronization Functions"): this session exports it and keeps it
# importable until it commits, after the export. -q keeps psql from printing the BEGIN tag, so the first line is the id.
coproc SNAP { "${compose[@]}" exec -T postgres psql -U "$user" -d "$database" -At -q -v ON_ERROR_STOP=1; }
# Ordinary copies of the coprocess's fds at once, the originals closed: bash unsets SNAP and closes its fds when it
# reaps a session that ended early, and a subshell (the COMMIT below) does not inherit coprocess fds.
# shellcheck disable=SC2153 # coproc sets SNAP_PID
snap_pid=$SNAP_PID co_out=${SNAP[0]} co_in=${SNAP[1]}
exec {snap_out}<&"$co_out" {snap_in}>&"$co_in" {co_out}<&- {co_in}>&-
printf 'BEGIN ISOLATION LEVEL REPEATABLE READ;\nSELECT pg_export_snapshot();\n' >&"$snap_in"
snapshot=""
IFS= read -r -t 60 snapshot <&"$snap_out" || true
[[ $snapshot =~ ^[0-9A-F]{8}-[0-9A-F]{8}-[0-9]+$ ]] || fail "could not export a snapshot; nothing published"

"${compose[@]}" exec -T postgres pg_dump -U "$user" -d "$database" --format=custom --no-owner --snapshot="$snapshot" >"$tmp_dump" ||
	fail "pg_dump failed; nothing published"
# A header check only: the custom format's magic.
[ "$(head -c 5 "$tmp_dump")" = PGDMP ] || fail "pg_dump wrote no custom-format archive; nothing published"
# A full read: every entry of the archive, data included, restored into no database.
"${compose[@]}" exec -T postgres pg_restore -f /dev/null <"$tmp_dump" ||
	fail "the archive does not read back whole; nothing published"
"${compose[@]}" exec -T server reeve-server export --snapshot "$snapshot" >"$tmp_json" ||
	fail "the offline export failed; nothing published"
# An export is a JSON object; an empty or foreign file is not a backup.
if [ ! -s "$tmp_json" ] || [ "$(head -c 1 "$tmp_json")" != "{" ]; then
	fail "the offline export is empty or not JSON; nothing published"
fi
# In a subshell: a write to a session that has ended raises SIGPIPE, which would kill this shell before its EXIT trap.
(printf 'COMMIT;\n' >&"$snap_in") 2>/dev/null || fail "the snapshot session ended early; nothing published"
exec {snap_in}>&- {snap_out}<&-
wait "$snap_pid" || fail "the snapshot session did not end cleanly; nothing published"
snap_pid=""

# Both files reach the disk before either is named.
sync -- "$tmp_dump" "$tmp_json"

ln -T -- "$tmp_dump" "$dump" || fail "could not publish $dump"
if ! ln -T -- "$tmp_json" "$json"; then
	rm -f -- "$dump"
	fail "could not publish $json"
fi
echo "$dump"
echo "$json"
