#!/usr/bin/env bash
# deploy/decision-check.sh, installed in the server image as /usr/local/bin/reeve-decision-check: the scheduled
# decision check (deploy/README.md, "Scheduled decision check"). It clones every configured repository that is not in
# the household yet, then runs `reeve decision check --strict` over the household and exits with its status.
#
#   docker compose exec -T server reeve-decision-check < /etc/reeve/decision-check.token
#
# Environment: REEVE_URL (required; the server's own address), REEVE_TOKEN (an admin's automation token; when unset
# or empty, the first line of standard input), REEVE_HOUSEHOLD (default /var/lib/reeve/household),
# REEVE_CHECK_TIMEOUT (seconds, default 480: the whole run, clones included, is killed after it), GIT_SSH_COMMAND
# (optional; default `ssh -o BatchMode=yes`). The token is never printed and never passed as an argument.
#
# One run at a time: the run holds a lock on $REEVE_HOUSEHOLD/.decision-check.lock for as long as any of its processes
# lives, and the deadline is enforced here, inside the container, on the whole process group (the host's compose
# client can be killed without its command dying).
#
# Exit status: the check's own (0 every record read and every result stored, or skipped because its decision was
# closed or changed after the check listed it, which the next run checks; 1 a record unreadable, a result not
# stored, or a result held because the token lost its admin role; 2 the server refused the caller); 2 when this script
# refuses to start (no url, no token, a token that is not an admin's, another run still holding the lock, an unusable
# repository name, a clone that failed); 124 (137 if it had to be killed) when the run exceeded REEVE_CHECK_TIMEOUT;
# the status of `reeve-server repos` or `reeve auth status` when either fails.
set -euo pipefail

fail() {
	echo "reeve-decision-check: $*" >&2
	exit 2
}

household=${REEVE_HOUSEHOLD:-/var/lib/reeve/household}

# The inner run is the one started below. It skips the outer steps only when fd 9 is exactly this household's lock file
# AND its parent process is `timeout`; a stray variable, or a caller's own fd 9 on some other file, cannot skip the
# checks. (A caller that opens the real lock file on fd 9 and runs this script under `timeout` could still skip them:
# that is the caller choosing to hold the lock, not an accident.) The flock below is a no-op on the inherited lock.
inner=0
if [ "$(readlink "/proc/$$/fd/9" 2>/dev/null || true)" = "$(realpath -m -- "$household")/.decision-check.lock" ] &&
	[ "$(cat "/proc/$PPID/comm" 2>/dev/null || true)" = timeout ]; then
	inner=1
	flock -n 9 || fail "the inner run does not hold the lock on $household/.decision-check.lock"
fi

if [ "$inner" -eq 0 ]; then
	[ -n "${REEVE_URL:-}" ] || fail "REEVE_URL is not set (the server's own address, e.g. http://127.0.0.1:7474)"
	if [ -z "${REEVE_TOKEN:-}" ] && [ ! -t 0 ]; then
		IFS= read -r -t 10 REEVE_TOKEN || true
		REEVE_TOKEN=${REEVE_TOKEN%$'\r'}
	fi
	[ -n "${REEVE_TOKEN:-}" ] || fail "no token: set REEVE_TOKEN or pass an admin's automation token on standard input"
	limit=${REEVE_CHECK_TIMEOUT:-480}
	# Digits with at least one that is not zero: GNU timeout takes 0 (and 00, 000) as "no deadline at all".
	case $limit in
	'' | *[!0-9]*) fail "REEVE_CHECK_TIMEOUT must be a positive number of seconds, not '$limit'" ;;
	*[1-9]*) ;;
	*) fail "REEVE_CHECK_TIMEOUT must be a positive number of seconds, not '$limit'" ;;
	esac
	mkdir -p -- "$household" || fail "cannot create the household directory $household"
	# fd 9 stays open in every process of the run (timeout, this script, git, ssh, reeve), so the lock is free only
	# once all of them are gone.
	exec 9>>"$household/.decision-check.lock" || fail "cannot open $household/.decision-check.lock"
	lock=0
	flock -n 9 || lock=$?
	case $lock in
	0) ;;
	1) fail "another decision check is still running (it holds $household/.decision-check.lock)" ;;
	*) fail "could not lock $household/.decision-check.lock (flock exited $lock)" ;;
	esac
	export REEVE_URL REEVE_TOKEN
	# GNU timeout runs the command in its own process group and signals the whole group, so a hung git or ssh dies too.
	exec timeout --kill-after=10 "$limit" bash "$0" </dev/null
fi

# The token goes to the two reeve calls below and nowhere else: this script's own git and ssh, and reeve-server, never
# see it, and reeve removes it from the environment of the git (and so the ssh) it runs. It is still in the initial
# environment of the timeout and the bash running this script (/proc/<pid>/environ), which `unset` does not change.
token=${REEVE_TOKEN:-}
unset REEVE_TOKEN
[ -n "$token" ] || fail "no token reached the run"

export GIT_TERMINAL_PROMPT=0
export GIT_SSH_COMMAND="${GIT_SSH_COMMAND:-ssh -o BatchMode=yes}"
# The housekeeping a fetch starts (gc --auto, maintenance run --auto) detaches by default, keeping fd 9 (the lock) past
# the run's end and outside timeout's process group. In the foreground it stays inside the lock and the deadline. Set
# through the environment so the git that reeve runs gets it too, after any GIT_CONFIG_* the operator passed in (a
# compose override's proxy or url rewrite), which are kept.
n=${GIT_CONFIG_COUNT:-0}
case $n in
'' | *[!0-9]*) fail "GIT_CONFIG_COUNT must be a number, not \`$n\`" ;;
esac
n=$((10#$n))
export "GIT_CONFIG_KEY_$n=gc.autoDetach" "GIT_CONFIG_VALUE_$n=false" \
	"GIT_CONFIG_KEY_$((n + 1))=maintenance.autoDetach" "GIT_CONFIG_VALUE_$((n + 1))=false" \
	GIT_CONFIG_COUNT=$((n + 2))

# Only an admin's check stores an accepted record; a member's token would hold every accepted result. `--strict` below
# also fails the run if the role is lost after this point.
status=0
me=$(REEVE_TOKEN=$token reeve --json auth status) || status=$?
if [ "$status" -ne 0 ]; then
	printf '%s\n' "$me" >&2
	echo "reeve-decision-check: reeve auth status exited $status (token refused or server unreachable)" >&2
	exit "$status"
fi
case $me in
*'"role":"admin"'*) ;;
*) fail "the token is not an admin's; only an admin's check stores an accepted record" ;;
esac

repos=$(reeve-server repos)
while IFS=$'\t' read -r name url; do
	[ -n "$name" ] || continue
	case $name in
	.* | -* | */* | *\\*) fail "repository name '$name' is not a single directory name" ;;
	esac
	dir=$household/$name
	[ ! -e "$dir" ] || continue
	# Cloned beside its final place and renamed into it only when complete: a run killed mid-clone leaves only the
	# temporary directory, which the next run (holding the lock) removes before cloning again.
	part=$household/.clone-$name
	rm -rf -- "$part"
	echo "reeve-decision-check: cloning $name"
	git clone -q --filter=blob:none --no-checkout -- "$url" "$part" || {
		rm -rf -- "$part"
		fail "could not clone $name"
	}
	git -C "$part" remote set-head origin -a >/dev/null || {
		rm -rf -- "$part"
		fail "could not set origin/HEAD in $name"
	}
	mv -T -- "$part" "$dir" || {
		rm -rf -- "$part"
		fail "could not move the clone of $name into place"
	}
done <<<"$repos"

REEVE_TOKEN=$token exec reeve decision check --strict --household "$household"
