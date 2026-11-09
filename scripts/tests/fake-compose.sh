#!/usr/bin/env bash
# scripts/tests/fake-compose.sh — stands in for `docker compose` in the script tests: logs each call to
# $FAKE_DIR/calls, insists on `exec -T` (a terminal would corrupt archives and documents), and plays the part the
# FAKE_* variables set. Outputs come from files the test writes: $FAKE_DIR/dump-output, $FAKE_DIR/export-output. A
# `psql` on standard input plays the exporting session: it prints ${FAKE_SNAPSHOT_ID:-00000003-0000001B-1} for
# `SELECT pg_export_snapshot();` after a `BEGIN ISOLATION LEVEL REPEATABLE READ;`, keeps $FAKE_DIR/snapshot-open until
# COMMIT; or ROLLBACK; or the end of its input, and records its input in $FAKE_DIR/session-input; `pg_dump --snapshot=` and `export --snapshot`
# fail unless that session is open with that id. FAKE_SESSION_END ends the session after the export, before COMMIT
# (or after FAKE_SESSION_WAIT × 0.05 s when no export comes). `reeve-server export --help` prints a help text that lists
# `--at` unless FAKE_NO_AT is set (an image from before urgent marks), and exits ${FAKE_HELP:-0}.
set -euo pipefail
: "${FAKE_DIR:?FAKE_DIR must be set}"
echo "$*" >>"$FAKE_DIR/calls"
if [ "$1" != exec ] || [ "$2" != -T ]; then
	echo "fake compose: expected 'exec -T', got: $*" >&2
	exit 99
fi
service=$3
shift 3
# snap_check <args>: a --snapshot argument must name the open session's snapshot (recorded in $FAKE_DIR/snapshot-uses).
snap_check() {
	local a id="" prev=""
	for a in "$@"; do
		case $a in --snapshot=*) id=${a#--snapshot=} ;; esac
		[ "$prev" != --snapshot ] || id=$a
		prev=$a
	done
	[ -n "$id" ] || return 0
	if [ ! -f "$FAKE_DIR/snapshot-open" ] || [ "$(cat "$FAKE_DIR/snapshot-open")" != "$id" ]; then
		echo "fake compose: snapshot $id is not open" >&2
		exit 95
	fi
	printf '%s\n' "$id" >>"$FAKE_DIR/snapshot-uses"
}
case "$service $1" in
"postgres createdb") exit "${FAKE_CREATEDB:-0}" ;;
"postgres pg_restore") cat >"$FAKE_DIR/restore-stdin"; exit "${FAKE_RESTORE:-0}" ;;
"postgres dropdb") exit "${FAKE_DROPDB:-0}" ;;
"postgres pg_dump")
	snap_check "$@"
	cat "$FAKE_DIR/dump-output"
	exit "${FAKE_DUMP:-0}"
	;;
"postgres psql")
	: >"$FAKE_DIR/session-input"
	began=0
	while IFS= read -r line; do
		printf '%s\n' "$line" >>"$FAKE_DIR/session-input"
		case $line in
		"BEGIN ISOLATION LEVEL REPEATABLE READ;") began=1 ;;
		# as in PostgreSQL, the snapshot ends with the transaction that exported it
		"COMMIT;" | "ROLLBACK;") began=0; rm -f "$FAKE_DIR/snapshot-open" ;;
		"SELECT pg_export_snapshot();")
			[ "${FAKE_SNAPSHOT:-0}" -eq 0 ] || exit "$FAKE_SNAPSHOT"
			[ "$began" -eq 1 ] || { echo "fake compose: snapshot exported outside a transaction block" >&2; exit 96; }
			printf '%s\n' "${FAKE_SNAPSHOT_ID:-00000003-0000001B-1}" >"$FAKE_DIR/snapshot-open"
			printf '%s\n' "${FAKE_SNAPSHOT_ID:-00000003-0000001B-1}"
			# FAKE_SESSION_END: the session is terminated once the export has run, before the script's COMMIT (an
			# administrator's terminate, idle_in_transaction_session_timeout); psql then exits 2, as for a lost connection.
			if [ -n "${FAKE_SESSION_END:-}" ]; then
				# This process and the coprocess's own bash (its parent), which also holds the session's standard input.
				printf '%s %s\n' "$$" "$PPID" >"$FAKE_DIR/session-pid"
				# Bounded (FAKE_SESSION_WAIT × 0.05 s, default 5 s): a script that fails before its export must not
				# leave this session behind.
				for _ in $(seq "${FAKE_SESSION_WAIT:-100}"); do
					[ ! -e "$FAKE_DIR/export-done" ] || break
					sleep 0.05
				done
				rm -f "$FAKE_DIR/snapshot-open"
				echo "FATAL:  terminating connection due to administrator command" >&2
				exit 2
			fi
			;;
		esac
	done
	rm -f "$FAKE_DIR/snapshot-open"
	exit 0
	;;
"server reeve-server")
	if [ "${2:-}" = export ] && [ "${3:-}" = --help ]; then
		printf 'Usage: reeve-server export [OPTIONS]\n\nOptions:\n      --out <OUT>\n      --database <DATABASE>\n'
		printf '      --snapshot <SNAPSHOT>\n'
		[ -n "${FAKE_NO_AT:-}" ] || printf '      --at <AT>\n          The document exported_at\n'
		printf '  -h, --help\n'
		exit "${FAKE_HELP:-0}"
	fi
	snap_check "$@"
	# FAKE_MKDIR: a directory that appears at this moment (after a script's pre-checks, before it publishes)
	[ -z "${FAKE_MKDIR:-}" ] || mkdir -p "$FAKE_MKDIR"
	# FAKE_TOUCH: a regular file with this content appears at this path at the same moment
	[ -z "${FAKE_TOUCH:-}" ] || printf 'appeared mid-backup\n' >"$FAKE_TOUCH"
	cat "$FAKE_DIR/export-output"
	# FAKE_SESSION_END: the export is done; return only once the session and every holder of its input have gone, so
	# the script's COMMIT meets a closed pipe (bounded: 5 s).
	if [ -n "${FAKE_SESSION_END:-}" ]; then
		touch "$FAKE_DIR/export-done"
		read -r -a pids <"$FAKE_DIR/session-pid"
		for pid in "${pids[@]}"; do
			for _ in $(seq 100); do
				kill -0 "$pid" 2>/dev/null || break
				sleep 0.05
			done
		done
	fi
	exit "${FAKE_EXPORT:-0}"
	;;
*) echo "fake compose: unexpected call: $service $*" >&2; exit 98 ;;
esac
