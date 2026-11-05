#!/usr/bin/env bash
# scripts/tests/decision-check.sh — deploy/decision-check.sh against fake `reeve` and `reeve-server` on PATH, a git
# wrapper that can hang or fail on cue, and a real git remote (a local bare repository served over file://, so a
# partial clone is really partial): clones a missing repository once with origin/HEAD set, never touches an existing
# one, recovers from a clone that was killed or failed half way, passes the check's exit status through, runs the check
# strictly, kills a run that exceeds its deadline with every process it started, refuses a second run while one holds
# the lock, refuses a member's token, an unusable name and a failed clone before checking, never prints the token or
# passes it as an argument. The check itself is proven by crates/reeve (cli.rs, acceptance.rs).
# shellcheck disable=SC2015 # "A && B || { report; exit 1; }" is the assertion idiom: the report never fails
set -euo pipefail
cd "$(dirname "$0")/../.."
# The cases set what they need; a developer's own REEVE_TOKEN (which wins over stdin), deadline, ssh command or git
# repository variables would otherwise change what the script does and fail the cases misleadingly.
unset REEVE_TOKEN REEVE_URL REEVE_HOUSEHOLD REEVE_CHECK_TIMEOUT GIT_SSH_COMMAND GIT_DIR GIT_WORK_TREE \
	GIT_CONFIG_COUNT GIT_CONFIG_PARAMETERS
script=deploy/decision-check.sh
bash -n "$script"
if command -v shellcheck >/dev/null 2>&1; then
	shellcheck "$script" "$0"
	echo "shellcheck ok"
elif [ -n "${CI:-}" ]; then
	echo "shellcheck is not installed and CI is set: the script is not linted" >&2
	exit 1
else
	echo "shellcheck not installed: syntax checked with bash -n only"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export FAKE_DIR="$tmp"
REAL_GIT=$(command -v git)
export REAL_GIT
mkdir "$tmp/bin"
cat >"$tmp/bin/reeve" <<'EOF'
#!/usr/bin/env bash
# Stands in for the reeve CLI: logs its arguments, keeps the token it was given, plays the part the case asks for.
set -euo pipefail
echo "reeve $*" >>"$FAKE_DIR/calls"
case "$*" in
"--json auth status")
	printf '%s' "$REEVE_TOKEN" >"$FAKE_DIR/token-seen"
	echo "{\"outcome\":\"ok\",\"value\":{\"email\":\"check@example.com\",\"role\":\"${FAKE_ROLE:-admin}\",\"token_from_env\":true,\"token_kind\":\"automation\",\"url\":\"$REEVE_URL\"}}"
	exit "${FAKE_AUTH_EXIT:-0}"
	;;
"decision check --strict --household "*)
	printf '%s' "${REEVE_TOKEN-}" >"$FAKE_DIR/token-seen-check"
	# What the git this run starts would do about housekeeping (it inherits this environment). The developer's own
	# ~/.gitconfig and the system's are left out, so only what the environment sets is read.
	for k in gc.autoDetach maintenance.autoDetach; do
		printf '%s=%s\n' "$k" "$(GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 "$REAL_GIT" config --get "$k" || true)"
	done >"$FAKE_DIR/git-detach-at-check"
	# And whether a setting the operator passed through GIT_CONFIG_* reached it.
	GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 "$REAL_GIT" config --get http.proxy >"$FAKE_DIR/git-proxy-at-check" || true
	ls -A "${@: -1}" | grep -v '^\.' >"$FAKE_DIR/household-at-check" || true
	if [ -n "${FAKE_CHECK_HANG:-}" ]; then
		sleep 300 &
		echo "$$ $!" >"$FAKE_DIR/worker"
		wait
	fi
	echo "checked 1 decision(s), 0 held, 0 skipped, 0 unreadable, 0 failed"
	exit "${FAKE_CHECK_EXIT:-0}"
	;;
*) echo "fake reeve: unexpected call: $*" >&2; exit 98 ;;
esac
EOF
cat >"$tmp/bin/reeve-server" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo "reeve-server $*" >>"$FAKE_DIR/calls"
[ -z "${REEVE_TOKEN+x}" ] || echo "reeve-server" >>"$FAKE_DIR/env-leak"
[ "$*" = repos ] || { echo "fake reeve-server: unexpected call: $*" >&2; exit 98; }
cat "$FAKE_DIR/repos"
exit "${FAKE_REPOS_EXIT:-0}"
EOF
cat >"$tmp/bin/git" <<'EOF'
#!/usr/bin/env bash
# The real git, except that a clone can hang half way (leaving a partial directory) and set-head can fail, on cue.
set -euo pipefail
echo "git $*" >>"$FAKE_DIR/git-args"
[ -z "${REEVE_TOKEN+x}" ] || echo "git" >>"$FAKE_DIR/env-leak"
if [ "${1:-}" = clone ] && [ -n "${FAKE_GIT_HANG:-}" ]; then
	dest=${*: -1}
	mkdir -p "$dest"
	echo partial >"$dest/junk"
	sleep 300 &
	echo "$$ $!" >"$FAKE_DIR/worker"
	wait
	exit 1
fi
if [ "${3:-}" = remote ] && [ "${4:-}" = set-head ] && [ -n "${FAKE_SETHEAD_FAIL:-}" ]; then
	echo "fake git: set-head failed" >&2
	exit 1
fi
exec "$REAL_GIT" "$@"
EOF
cat >"$tmp/bin/mv" <<'EOF'
#!/usr/bin/env bash
# The real mv, except that it fails on cue.
[ -z "${FAKE_MV_FAIL:-}" ] || { echo "fake mv: failed" >&2; exit 1; }
exec /bin/mv "$@"
EOF
chmod +x "$tmp/bin/reeve" "$tmp/bin/reeve-server" "$tmp/bin/git" "$tmp/bin/mv"

# The remote: a bare repository whose default branch is `trunk`, allowing partial clones over file://.
git init -q -b trunk "$tmp/work"
git -C "$tmp/work" -c user.email=t@example.com -c user.name=t commit -q --allow-empty -m init
git clone -q --bare "$tmp/work" "$tmp/kb.git"
git -C "$tmp/kb.git" config uploadpack.allowFilter true
remote="file://$tmp/kb.git"
token=reeve_sentinel_token_7f3a

household=$tmp/household
# run <expected status> [VAR=value ...]: runs the script with the token on stdin; fails on any other status. The test's
# own 30-second bound keeps a script without a working deadline from hanging the test; `took` is the run's duration.
took=0
run() {
	local want=$1 s=0 start=$SECONDS
	shift
	rm -f "$tmp/calls" "$tmp/git-detach-at-check" "$tmp/git-proxy-at-check" "$tmp/token-seen" "$tmp/token-seen-check" "$tmp/household-at-check" "$tmp/worker" "$tmp/git-args" "$tmp/env-leak"
	printf '%s\n' "$token" | timeout -k 5 30 env PATH="$tmp/bin:$PATH" REEVE_URL=http://127.0.0.1:7474 \
		REEVE_HOUSEHOLD="$household" "$@" bash "$script" >"$tmp/out" 2>&1 || s=$?
	took=$((SECONDS - start))
	if [ "$s" -ne "$want" ]; then
		echo "wanted exit $want, got $s; output:" >&2
		cat "$tmp/out" >&2
		exit 1
	fi
	if grep -q "$token" "$tmp/out" "$tmp/calls" "$tmp/git-args" 2>/dev/null; then
		echo "the token was printed or passed as an argument" >&2
		exit 1
	fi
	if [ -e "$tmp/env-leak" ]; then
		echo "the token reached the environment of: $(cat "$tmp/env-leak")" >&2
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
# alive <pid>: the process exists and is not a zombie (a killed orphan may linger as one until it is reaped).
alive() {
	local st
	st=$(ps -o stat= -p "$1" 2>/dev/null) || return 1
	case $st in Z*) return 1 ;; esac
}
# expect_no_worker: every process the hung run started is gone (polled for up to five seconds).
expect_no_worker() {
	local pid
	[ -s "$tmp/worker" ] || { echo "the hung step never started" >&2; exit 1; }
	# shellcheck disable=SC2013 # two pids, split on whitespace
	for pid in $(cat "$tmp/worker"); do
		for _ in $(seq 50); do
			alive "$pid" || break
			sleep 0.1
		done
		if alive "$pid"; then
			echo "process $pid of a finished run is still alive" >&2
			exit 1
		fi
	done
}
is_clone() {
	[ "$(git -C "$household/kb" symbolic-ref refs/remotes/origin/HEAD)" = refs/remotes/origin/trunk ] ||
		{ echo "origin/HEAD does not name the remote's default branch" >&2; exit 1; }
	[ "$(git -C "$household/kb" config --get remote.origin.promisor)" = true ] || { echo "the clone is not partial" >&2; exit 1; }
	[ ! -e "$household/.clone-kb" ] || { echo "a temporary clone was left behind" >&2; exit 1; }
}
all="reeve --json auth status
reeve-server repos
reeve decision check --strict --household $household"

# 1. A missing repository is cloned (partially, origin/HEAD on the remote's default branch), then the check runs,
#    strictly, on the household with the token from stdin, and its status 0 is the script's.
printf 'kb\t%s\n' "$remote" >"$tmp/repos"
run 0
expect_calls "$all"
[ "$(cat "$tmp/token-seen")" = "$token" ] && [ "$(cat "$tmp/token-seen-check")" = "$token" ] ||
	{ echo "the CLI did not get the token from stdin on both calls" >&2; exit 1; }
[ -s "$tmp/git-args" ] || { echo "the fake git was never called" >&2; exit 1; }
[ "$(cat "$tmp/household-at-check")" = kb ] || { echo "the check ran before the clone" >&2; exit 1; }
is_clone
grep -q "cloning kb" "$tmp/out" || { echo "the clone was not reported" >&2; exit 1; }
# Housekeeping that git starts (gc --auto, maintenance --auto) runs in the foreground, inside the lock and the deadline.
[ "$(cat "$tmp/git-detach-at-check")" = $'gc.autoDetach=false\nmaintenance.autoDetach=false' ] ||
	{ echo "git may detach its housekeeping: $(cat "$tmp/git-detach-at-check")" >&2; exit 1; }

# 2. An existing clone is left alone (the check fetches it): no clone line, the marker survives.
touch "$household/kb/.git/marker"
run 0
expect_calls "$all"
[ -e "$household/kb/.git/marker" ] || { echo "the existing clone was replaced" >&2; exit 1; }
! grep -q "cloning" "$tmp/out" || { echo "an existing clone was cloned again" >&2; exit 1; }

# 2b. Git settings the operator passes through GIT_CONFIG_* (for example from a compose override) are kept: the
#     script's own two are added after them, not written over them. A count that is not a number is refused.
run 0 GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=http.proxy GIT_CONFIG_VALUE_0=http://proxy.example:3128
expect_calls "$all"
[ "$(cat "$tmp/git-proxy-at-check")" = http://proxy.example:3128 ] ||
	{ echo "the operator's GIT_CONFIG_* setting was dropped: $(cat "$tmp/git-proxy-at-check")" >&2; exit 1; }
[ "$(cat "$tmp/git-detach-at-check")" = $'gc.autoDetach=false\nmaintenance.autoDetach=false' ] ||
	{ echo "with an operator's setting, git may detach: $(cat "$tmp/git-detach-at-check")" >&2; exit 1; }
for bad in x -1 1x; do
	run 2 GIT_CONFIG_COUNT=$bad
	grep -q "GIT_CONFIG_COUNT" "$tmp/out" || { echo "a bad GIT_CONFIG_COUNT was not reported: $bad" >&2; cat "$tmp/out" >&2; exit 1; }
done

# 3. The check's own failures pass through: 1 (unreadable, not stored or held) and 2 (the server refused the caller).
run 1 FAKE_CHECK_EXIT=1
run 2 FAKE_CHECK_EXIT=2

# 4. REEVE_TOKEN in the environment wins over stdin.
s=0
env PATH="$tmp/bin:$PATH" REEVE_URL=http://127.0.0.1:7474 REEVE_HOUSEHOLD="$household" REEVE_TOKEN=reeve_env_token \
	bash "$script" </dev/null >"$tmp/out" 2>&1 || s=$?
[ "$s" -eq 0 ] && [ "$(cat "$tmp/token-seen")" = reeve_env_token ] || { echo "REEVE_TOKEN was not used: exit $s" >&2; cat "$tmp/out" >&2; exit 1; }
! grep -q reeve_env_token "$tmp/out" "$tmp/calls" || { echo "the environment token was printed" >&2; exit 1; }

# 5. A member's token: refused before anything is cloned or checked.
run 2 FAKE_ROLE=member
expect_calls "reeve --json auth status"

# 6. A token the server refuses: the CLI's status (2), nothing else run.
run 2 FAKE_AUTH_EXIT=2
expect_calls "reeve --json auth status"

# 7. No token at all (stdin empty, REEVE_TOKEN unset), no REEVE_URL, a bad deadline: 2, nothing run.
s=0
env -u REEVE_TOKEN PATH="$tmp/bin:$PATH" REEVE_URL=http://127.0.0.1:7474 bash "$script" </dev/null >"$tmp/out" 2>&1 || s=$?
[ "$s" -eq 2 ] && grep -q "no token" "$tmp/out" || { echo "no token: exit $s" >&2; cat "$tmp/out" >&2; exit 1; }
rm -f "$tmp/calls"
run 2 REEVE_URL=
expect_calls ""
for bad in soon 0 00 000 -5 1.5; do
	run 2 REEVE_CHECK_TIMEOUT=$bad
	expect_calls ""
done

# 8. An unusable repository name: 2, nothing outside the household, no check.
for bad in .. .hidden ../escape -c a/b 'a\b'; do
	printf '%s\t%s\n' "$bad" "$remote" >"$tmp/repos"
	run 2
	expect_calls $'reeve --json auth status\nreeve-server repos'
done
[ ! -e "$tmp/escape" ] || { echo "a clone escaped the household" >&2; exit 1; }

# 9. A clone that fails: 2, no check, and nothing left behind, so the next run tries again.
printf 'gone\t%s\n' "file://$tmp/missing.git" >"$tmp/repos"
run 2
expect_calls $'reeve --json auth status\nreeve-server repos'
[ ! -e "$household/gone" ] && [ ! -e "$household/.clone-gone" ] || { echo "a failed clone left a directory" >&2; exit 1; }

# 10. `reeve-server repos` failing (a bad configuration): its status, no check.
printf 'kb\t%s\n' "$remote" >"$tmp/repos"
run 1 FAKE_REPOS_EXIT=1
expect_calls $'reeve --json auth status\nreeve-server repos'

# 11. A clone that succeeded but could not be completed (set-head failed): 2, no half clone in place; the next run
#     clones it properly.
mv "$household/kb" "$tmp/kb-saved"
run 2 FAKE_SETHEAD_FAIL=1
[ ! -e "$household/kb" ] && [ ! -e "$household/.clone-kb" ] || { echo "an incomplete clone was left behind" >&2; exit 1; }
run 0
is_clone

# 11b. A complete clone that cannot be moved into place: 2, like any clone that failed (not set -e's 1), no check, and
#     no temporary clone left; the next run clones it properly.
mv "$household/kb" "$tmp/kb-saved-mv"
run 2 FAKE_MV_FAIL=1
expect_calls $'reeve --json auth status\nreeve-server repos'
grep -q "could not move the clone of kb into place" "$tmp/out" || { echo "the failed move was not reported" >&2; cat "$tmp/out" >&2; exit 1; }
[ ! -e "$household/kb" ] && [ ! -e "$household/.clone-kb" ] || { echo "a clone that was not moved was left behind" >&2; exit 1; }
run 0
is_clone

# 12. A clone that hangs: the deadline kills the run (124) and every process it started; the half-written temporary
#     clone is not taken for a clone, and the next run replaces it with a real one.
mv "$household/kb" "$tmp/kb-saved-2"
run 124 FAKE_GIT_HANG=1 REEVE_CHECK_TIMEOUT=2
[ "$took" -lt 15 ] || { echo "the run's own deadline did not end it (took ${took}s)" >&2; exit 1; }
expect_no_worker
[ ! -e "$household/kb" ] && [ -e "$household/.clone-kb/junk" ] || { echo "the hung clone did not stop half way" >&2; exit 1; }
run 0
expect_calls "$all"
is_clone

# 13. A check that hangs: killed at the deadline with its children, and the lock is free again afterwards.
run 124 FAKE_CHECK_HANG=1 REEVE_CHECK_TIMEOUT=2
[ "$took" -lt 15 ] || { echo "the run's own deadline did not end it (took ${took}s)" >&2; exit 1; }
expect_no_worker
run 0

# 14. Runs never overlap: while one holds the lock, a second refuses (2) without calling anything; once the first is
#     stopped, with every process it started, the next run proceeds.
rm -f "$tmp/calls" "$tmp/worker"
env PATH="$tmp/bin:$PATH" REEVE_URL=http://127.0.0.1:7474 REEVE_HOUSEHOLD="$household" REEVE_TOKEN=reeve_env_token \
	FAKE_CHECK_HANG=1 REEVE_CHECK_TIMEOUT=60 bash "$script" </dev/null >"$tmp/first" 2>&1 &
first=$!
for _ in $(seq 100); do
	[ ! -s "$tmp/worker" ] || break
	sleep 0.1
done
[ -s "$tmp/worker" ] || { echo "the first run never reached the check" >&2; cat "$tmp/first" >&2; exit 1; }
cp "$tmp/worker" "$tmp/first-worker"
before=$(calls)
s=0
env PATH="$tmp/bin:$PATH" REEVE_URL=http://127.0.0.1:7474 REEVE_HOUSEHOLD="$household" REEVE_TOKEN=reeve_env_token \
	bash "$script" </dev/null >"$tmp/out" 2>&1 || s=$?
[ "$s" -eq 2 ] && grep -q "still running" "$tmp/out" || { echo "a second run overlapped: exit $s" >&2; cat "$tmp/out" >&2; exit 1; }
[ "$(calls)" = "$before" ] || { echo "the refused run called something" >&2; exit 1; }
kill -TERM "$first"
wait "$first" || true
cp "$tmp/first-worker" "$tmp/worker"
expect_no_worker
run 0

# 15. The inner run's shortcut is not open to a caller: a foreign fd 9 on a file of the same name in another directory,
#     and the real household lock opened by a caller that is not `timeout`, both still run the outer steps (no token: 2).
mkdir -p "$tmp/other"
s=0
env -u REEVE_TOKEN PATH="$tmp/bin:$PATH" REEVE_URL=http://127.0.0.1:7474 REEVE_HOUSEHOLD="$household" \
	bash "$script" </dev/null 9>>"$tmp/other/.decision-check.lock" >"$tmp/out" 2>&1 || s=$?
[ "$s" -eq 2 ] && grep -q "no token: set REEVE_TOKEN" "$tmp/out" && ! grep -q "reached the run" "$tmp/out" || { echo "a foreign fd 9 skipped the checks: exit $s" >&2; cat "$tmp/out" >&2; exit 1; }
s=0
env -u REEVE_TOKEN PATH="$tmp/bin:$PATH" REEVE_URL=http://127.0.0.1:7474 REEVE_HOUSEHOLD="$household" \
	bash "$script" </dev/null 9>>"$household/.decision-check.lock" >"$tmp/out" 2>&1 || s=$?
[ "$s" -eq 2 ] && grep -q "no token: set REEVE_TOKEN" "$tmp/out" && ! grep -q "reached the run" "$tmp/out" || { echo "the real lock under a non-timeout parent skipped the checks: exit $s" >&2; cat "$tmp/out" >&2; exit 1; }

# The host units and the compose file: checked with the tools that understand them, when this machine has them.
if command -v systemd-analyze >/dev/null 2>&1 && [ -x /usr/bin/docker ]; then
	systemd-analyze verify deploy/systemd/reeve-decision-check.timer deploy/systemd/reeve-decision-check.service
	[ "$(systemd-analyze calendar '*:0/10' | sed -n 's/^Normalized form: //p')" = '*-*-* *:00/10:00' ] ||
		{ echo "OnCalendar=*:0/10 is not every ten minutes on this systemd" >&2; exit 1; }
	echo "systemd units ok"
else
	echo "systemd-analyze or /usr/bin/docker missing: units not verified"
fi
grep -qx 'OnCalendar=\*:0/10' deploy/systemd/reeve-decision-check.timer || { echo "the timer is not every ten minutes" >&2; exit 1; }
grep -qx 'RandomizedDelaySec=60' deploy/systemd/reeve-decision-check.timer || { echo "the timer has no randomized delay" >&2; exit 1; }
# No catch-up at boot: it would fire as timers start, before compose has the server up, and fail once per boot.
! grep -q '^Persistent=' deploy/systemd/reeve-decision-check.timer || { echo "the timer catches up missed runs at boot" >&2; exit 1; }
grep -qx 'ExecStart=/usr/bin/docker compose exec -T server reeve-decision-check' deploy/systemd/reeve-decision-check.service ||
	{ echo "the service does not exec the script in the server container without a terminal" >&2; exit 1; }
if docker compose version >/dev/null 2>&1; then
	rendered=$(docker compose -f docker-compose.yml config)
	grep -q 'target: /var/lib/reeve/household' <<<"$rendered" || { echo "the server has no household volume" >&2; exit 1; }
	grep -q 'REEVE_HOUSEHOLD: /var/lib/reeve/household' <<<"$rendered" || { echo "the server does not name the household" >&2; exit 1; }
	echo "compose file ok"
else
	echo "docker compose not installed: docker-compose.yml not rendered"
fi

echo "decision check script test ok"
