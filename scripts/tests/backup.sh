#!/usr/bin/env bash
# scripts/tests/backup.sh — the backup's control flow against scripts/tests/fake-compose.sh: both commands in order,
# both files published only after both succeeded, fresh 0600 files, an existing file never touched, no temporary left.
set -euo pipefail
cd "$(dirname "$0")/../.."
script=scripts/backup.sh
bash -n "$script"
if command -v shellcheck >/dev/null 2>&1; then
	shellcheck "$script" scripts/tests/backup.sh
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
fake="$PWD/scripts/tests/fake-compose.sh"
out="$tmp/backups"
stamp=20261026T100000Z
dump="$out/reeve-$stamp.dump"
json="$out/reeve-$stamp.json"
printf 'PGDMP fake archive\n' >"$tmp/dump-output"
printf '{\n  "exported_at": "2026-10-26T10:00:00Z"\n}\n' >"$tmp/export-output"

# run <expected status> [VAR=value ...]: a fresh call log and backup directory contents as the case left them. SIGPIPE
# at its default (mise runs its children with it ignored; a shell started over ssh has it at the default).
run() {
	local want=$1 s=0
	shift
	rm -f "$tmp/calls" "$tmp/snapshot-uses" "$tmp/export-done" "$tmp/session-pid"
	env --default-signal=PIPE REEVE_COMPOSE="$fake" REEVE_BACKUP_STAMP="$stamp" "$@" bash "$script" "$out" >"$tmp/out" 2>&1 || s=$?
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
# the directory holds exactly these names (sorted), so no temporary and no stray final file is left
expect_files() {
	local got
	got=$(cd "$out" && find . -mindepth 1 -printf '%P\n' | sort | tr '\n' ' ')
	[ "$got" = "$1" ] || { echo "files: wanted '$1', got '$got'" >&2; exit 1; }
}
session='exec -T postgres psql -U reeve -d reeve -At -q -v ON_ERROR_STOP=1'
snap=00000003-0000001B-1
dumpcall="exec -T postgres pg_dump -U reeve -d reeve --format=custom --no-owner --snapshot=$snap"
steps="$session
$dumpcall
exec -T postgres pg_restore -f /dev/null
exec -T server reeve-server export --snapshot $snap"

# 1. all succeed: 0, the three commands in order, the dump fully read from stdin, both files published with mode 0600
mkdir -p "$out"
run 0
expect_calls "$steps"
cmp -s "$tmp/dump-output" "$tmp/restore-stdin" || { echo "the full read did not get the dump" >&2; exit 1; }
expect_files "reeve-$stamp.dump reeve-$stamp.json "
if ! cmp -s "$tmp/dump-output" "$dump" || ! cmp -s "$tmp/export-output" "$json"; then echo "published contents differ" >&2; exit 1; fi
[ "$(stat -c %a "$dump") $(stat -c %a "$json")" = "600 600" ] || { echo "modes: $(stat -c %a "$dump" "$json")" >&2; exit 1; }
if ! grep -qx -- "$dump" "$tmp/out" || ! grep -qx -- "$json" "$tmp/out"; then echo "the paths are not printed" >&2; exit 1; fi

# 1b. one snapshot for both files: the same id reached pg_dump and the export while the session was open (the fake
#     refuses a --snapshot otherwise), the session began REPEATABLE READ, and it ended after the export.
[ "$(cat "$tmp/snapshot-uses")" = "$snap"$'\n'"$snap" ] || { echo "the dump and the export did not both read $snap" >&2; exit 1; }
[ "$(head -2 "$tmp/session-input")" = $'BEGIN ISOLATION LEVEL REPEATABLE READ;\nSELECT pg_export_snapshot();' ] || { echo "session input: $(cat "$tmp/session-input")" >&2; exit 1; }
grep -qx 'COMMIT;' "$tmp/session-input" || { echo "the session was not committed" >&2; exit 1; }
[ ! -e "$tmp/snapshot-open" ] || { echo "the session was left open" >&2; exit 1; }

# 1d. the session ends after the export, before its COMMIT: 2 with a message (not a shell error, not SIGPIPE), nothing
#     published, no temporary left
rm -rf "$out" && mkdir -p "$out"
run 2 FAKE_SESSION_END=1
grep -q 'the snapshot session ended early' "$tmp/out" || { echo "no message for the session that ended early: $(cat "$tmp/out")" >&2; exit 1; }
expect_files ""

# 1e. the same session when the script fails before the export (pg_dump fails): the session ends on its own; none is
#     left running (FAKE_SESSION_WAIT=20: 1 s)
rm -rf "$out" && mkdir -p "$out"
run 2 FAKE_SESSION_END=1 FAKE_SESSION_WAIT=20 FAKE_DUMP=1
read -r session_pid _ <"$tmp/session-pid"
for _ in $(seq 100); do kill -0 "$session_pid" 2>/dev/null || break; sleep 0.05; done
if kill -0 "$session_pid" 2>/dev/null; then kill "$session_pid"; echo "the session outlived a backup that failed before its export" >&2; exit 1; fi
expect_files ""

# 1c. the snapshot cannot be exported: 2, neither pg_dump nor the export runs, nothing published
rm -rf "$out" && mkdir -p "$out"
run 2 FAKE_SNAPSHOT=1
expect_calls "$session"
expect_files ""

# 2. a file of that name exists (0644, a previous run's): 2, nothing run, the file untouched
rm -rf "$out" && mkdir -p "$out"
printf 'old\n' >"$dump"
chmod 644 "$dump"
run 2
expect_calls ""
expect_files "reeve-$stamp.dump "
[ "$(cat "$dump") $(stat -c %a "$dump")" = "old 644" ] || { echo "the existing file was touched" >&2; exit 1; }

# 3. pg_dump fails: 2, the export never runs, nothing published, no temporary left
rm -rf "$out" && mkdir -p "$out"
run 2 FAKE_DUMP=1
expect_calls "$session
$dumpcall"
expect_files ""

# 4. pg_dump exits 0 without writing an archive (the header check): 2, nothing published
printf 'not an archive\n' >"$tmp/dump-output"
run 2
expect_calls "$session
$dumpcall"
expect_files ""
printf 'PGDMP fake archive\n' >"$tmp/dump-output"

# 5. the archive does not read back whole (the full read fails): 2, the export never runs, nothing published
run 2 FAKE_RESTORE=1
expect_calls "$session
$dumpcall
exec -T postgres pg_restore -f /dev/null"
expect_files ""

# 6. the export fails after a good dump: 2, neither file published, no temporary left
run 2 FAKE_EXPORT=1
expect_calls "$steps"
expect_files ""

# 7. a directory appears at the dump's final name after the pre-check: 2, nothing linked into it, nothing published
run 2 FAKE_MKDIR="$dump"
expect_files "reeve-$stamp.dump "
if [ ! -d "$dump" ] || [ -n "$(ls -A "$dump")" ]; then echo "something was linked into $dump" >&2; exit 1; fi
rm -rf "$out" && mkdir -p "$out"

# 8. the same at the export's name: 2, and the dump already linked is taken back, so the pair is all or nothing
run 2 FAKE_MKDIR="$json"
expect_files "reeve-$stamp.json "
if [ ! -d "$json" ] || [ -n "$(ls -A "$json")" ]; then echo "something was linked into $json" >&2; exit 1; fi
rm -rf "$out" && mkdir -p "$out"

# 8b. a regular file appears at the dump's final name after the pre-check: 2, its content unchanged (ln -T, never -f)
run 2 FAKE_TOUCH="$dump"
expect_files "reeve-$stamp.dump "
[ "$(cat "$dump")" = "appeared mid-backup" ] || { echo "the file that appeared was overwritten" >&2; exit 1; }
rm -rf "$out" && mkdir -p "$out"

# 8c. the same at the export's name: 2, the file untouched and the dump taken back
run 2 FAKE_TOUCH="$json"
expect_files "reeve-$stamp.json "
[ "$(cat "$json")" = "appeared mid-backup" ] || { echo "the file that appeared was overwritten" >&2; exit 1; }
rm -rf "$out" && mkdir -p "$out"

# 8d. an empty export, then a stamp with a path in it: 2, nothing published
: >"$tmp/export-output"
run 2
expect_files ""
printf '{\n  "exported_at": "2026-10-26T10:00:00Z"\n}\n' >"$tmp/export-output"
rm -f "$tmp/calls"
run 2 REEVE_BACKUP_STAMP=../x
expect_calls ""
expect_files ""

# 9. no directory argument, or not a directory: 2, nothing run
rm -f "$tmp/calls"
s=0
REEVE_COMPOSE="$fake" bash "$script" >/dev/null 2>&1 || s=$?
if [ "$s" -ne 2 ] || [ -e "$tmp/calls" ]; then echo "a missing argument exited $s or ran commands" >&2; exit 1; fi
s=0
REEVE_COMPOSE="$fake" bash "$script" "$tmp/no-such-dir" >/dev/null 2>&1 || s=$?
if [ "$s" -ne 2 ] || [ -e "$tmp/calls" ]; then echo "a missing directory exited $s or ran commands" >&2; exit 1; fi

echo "backup test ok"
