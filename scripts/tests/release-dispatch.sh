#!/usr/bin/env bash
# scripts/tests/release-dispatch.sh — scripts/release-dispatch.sh against a fake curl: unconfigured it calls nothing;
# configured it dispatches the named workflow on the named ref, or on the repository's default branch, with the token
# in a header and never on the command line; every failure is a warning, exit 0, that never names the target.
set -euo pipefail
cd "$(dirname "$0")/../.."
script=scripts/release-dispatch.sh
bash -n "$script"
if command -v shellcheck >/dev/null 2>&1; then
	shellcheck "$script" scripts/tests/release-dispatch.sh
	echo "shellcheck ok"
elif [ -n "${CI:-}" ]; then
	echo "shellcheck required in CI" >&2
	exit 1
else
	echo "shellcheck not installed: syntax checked with bash -n only"
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir "$work/bin"
# The fake curl: logs one line per call (method, URL, data, the header file's contents, every argument), answers GET
# with FAKE_GET_CODE and a repository naming FAKE_BRANCH, POST with FAKE_POST_CODE; FAKE_EXIT makes curl itself fail.
cat >"$work/bin/curl" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
method=GET out="" url="" data="" headers=""
args="$*"
while [ $# -gt 0 ]; do
	case "$1" in
	-X) method=$2; shift ;;
	-o) out=$2; shift ;;
	-w | --max-time) shift ;;
	--data) data=$2; shift ;;
	-H) case "$2" in @*) headers="$headers$(cat "${2#@}")" ;; esac; shift ;;
	-*) ;;
	*) url=$1 ;;
	esac
	shift
done
printf '%s %s data=%s headers=%s args=%s\n' "$method" "$url" "$data" "$headers" "$args" >>"$FAKE_LOG"
[ "${FAKE_EXIT:-0}" -eq 0 ] || { echo "curl: (6) Could not resolve host" >&2; exit "$FAKE_EXIT"; }
if [ "$method" = GET ]; then
	printf '{"default_branch":"%s"}' "${FAKE_BRANCH-trunk}" >"$out"
	printf '%s' "${FAKE_GET_CODE:-200}"
else
	: >"$out"
	printf '%s' "${FAKE_POST_CODE:-204}"
fi
FAKE
chmod +x "$work/bin/curl"

fails=0
fail() {
	echo "FAIL: $*" >&2
	fails=$((fails + 1))
}
# run <target> <token> [VAR=value...]: runs the script with a fresh log; sets $out, $rc and $log.
run() {
	local target=$1 token=$2
	shift 2
	: >"$work/log"
	rc=0
	out=$(env -i PATH="$work/bin:$PATH" HOME="$HOME" FAKE_LOG="$work/log" GITHUB_API_URL=https://api.example \
		RELEASE_DISPATCH_TARGET="$target" RELEASE_DISPATCH_TOKEN="$token" "$@" bash "$script" 2>&1) || rc=$?
	log=$(cat "$work/log")
}
calls() { if [ -z "$log" ]; then echo 0; else printf '%s\n' "$log" | wc -l | tr -d ' '; fi; }
# expect_ok <output> <what>: exit 0 with exactly that output.
expect_ok() {
	[ "$rc" -eq 0 ] || fail "$2: exit $rc, want 0: $out"
	[ "$out" = "$1" ] || fail "$2: output '$out', want '$1'"
}
# expect_warning <what>: exit 0, one warning annotation, and the target's owner and repository named nowhere.
expect_warning() {
	[ "$rc" -eq 0 ] || fail "$1: exit $rc, want 0"
	printf '%s\n' "$out" | grep -q '^::warning::release dispatch: ' || fail "$1: no warning in: $out"
	if printf '%s\n' "$out" | grep -qE 'acme|deployments'; then fail "$1: the output names the target: $out"; fi
}

run "" ""
expect_ok "release dispatch: not configured, nothing to do" "unconfigured"
[ "$(calls)" -eq 0 ] || fail "unconfigured: curl was called: $log"

run "acme/deployments/bump.yml" ""
expect_warning "token missing"
[ "$(calls)" -eq 0 ] || fail "token missing: curl was called"
run "" "s3cret"
expect_warning "target missing"
[ "$(calls)" -eq 0 ] || fail "target missing: curl was called"

for bad in "acme/deployments" "acme/deployments/bump" "acme/deployments/bump.yml@" "acme/deploy ments/bump.yml" "https://github.com/acme/deployments/bump.yml"; do
	run "$bad" "s3cret"
	expect_warning "bad target '$bad'"
	[ "$(calls)" -eq 0 ] || fail "bad target '$bad': curl was called"
done

# A ref given: one POST to that workflow, on that ref, the token only in the header file.
run "acme/deployments/bump.yml@release" "s3cret"
expect_ok "release dispatch: started the configured workflow" "with ref"
[ "$(calls)" -eq 1 ] || fail "with ref: $(calls) calls, want 1: $log"
want='POST https://api.example/repos/acme/deployments/actions/workflows/bump.yml/dispatches data={"ref":"release"} headers=Authorization: Bearer s3cret'
case "$log" in "$want"*) ;; *) fail "with ref: got $log" ;; esac
case "${log#*args=}" in *s3cret*) fail "with ref: the token is on curl's command line" ;; esac

# No ref: read the default branch, then dispatch on it.
run "acme/deployments/bump.yaml" "s3cret" FAKE_BRANCH=trunk
expect_ok "release dispatch: started the configured workflow" "default branch"
[ "$(calls)" -eq 2 ] || fail "default branch: $(calls) calls, want 2: $log"
first=$(printf '%s\n' "$log" | sed -n 1p)
second=$(printf '%s\n' "$log" | sed -n 2p)
case "$first" in "GET https://api.example/repos/acme/deployments data= "*) ;; *) fail "default branch: first call $first" ;; esac
case "$second" in 'POST https://api.example/repos/acme/deployments/actions/workflows/bump.yaml/dispatches data={"ref":"trunk"} '*) ;; *) fail "default branch: second call $second" ;; esac

# Every refusal: a warning with the status, exit 0, the target unnamed.
run "acme/deployments/bump.yml@main" "s3cret" FAKE_POST_CODE=404
expect_warning "dispatch refused"
printf '%s\n' "$out" | grep -q 'HTTP 404' || fail "dispatch refused: no status in: $out"
run "acme/deployments/bump.yml" "s3cret" FAKE_GET_CODE=401
expect_warning "repository unreadable"
printf '%s\n' "$out" | grep -q 'HTTP 401' || fail "repository unreadable: no status in: $out"
[ "$(calls)" -eq 1 ] || fail "repository unreadable: dispatched anyway: $log"
run "acme/deployments/bump.yml" "s3cret" FAKE_BRANCH=
expect_warning "no default branch"
[ "$(calls)" -eq 1 ] || fail "no default branch: dispatched anyway: $log"
run "acme/deployments/bump.yml@main" "s3cret" FAKE_EXIT=6
expect_warning "curl failed"
printf '%s\n' "$out" | grep -q 'HTTP 000' || fail "curl failed: no status in: $out"

if [ "$fails" -ne 0 ]; then
	echo "release-dispatch test: $fails failure(s)" >&2
	exit 1
fi
echo "release-dispatch test ok"
