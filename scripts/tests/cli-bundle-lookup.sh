#!/usr/bin/env bash
# scripts/tests/cli-bundle-lookup.sh — scripts/cli-bundle-lookup.sh's classification against a fake `docker` that
# answers as the registry might: only a clear "not found" / "manifest unknown" / "name unknown" is "missing", and every
# authorization failure or other error fails rather than read as missing.
set -euo pipefail
cd "$(dirname "$0")/../.."
die() { echo "FAIL: $*" >&2; exit 1; }
if command -v shellcheck >/dev/null 2>&1; then
	shellcheck scripts/cli-bundle-lookup.sh scripts/tests/cli-bundle-lookup.sh
elif [ -n "${CI:-}" ]; then die "shellcheck required in CI"; fi
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir "$work/bin"
# The fake prints $FAKE_OUT on stdout and $FAKE_ERR on stderr, and exits $FAKE_CODE.
cat >"$work/bin/docker" <<'EOF'
#!/usr/bin/env bash
[ "$1 $2 $3" = "buildx imagetools inspect" ] || { echo "fake docker: unexpected $*" >&2; exit 99; }
printf '%s' "${FAKE_OUT:-}"
[ -z "${FAKE_ERR:-}" ] || printf '%s\n' "$FAKE_ERR" >&2
exit "${FAKE_CODE:-0}"
EOF
chmod +x "$work/bin/docker"
v=$(printf 'a%.0s' $(seq 40))
digest=sha256:$(printf 'b%.0s' $(seq 64))
lookup() { PATH="$work/bin:$PATH" scripts/cli-bundle-lookup.sh "$v" 2>"$work/err"; }

# expect <want: "published …" | missing | fail> <code> <stdout> <stderr>
expect() {
	local want=$1 got code=0
	got=$(FAKE_CODE=$2 FAKE_OUT=$3 FAKE_ERR=$4 lookup) || code=$?
	if [ "$want" = fail ]; then
		[ "$code" != 0 ] || die "'$4' (exit $2) was read as '$got', want a failure"
		[ -z "$got" ] || die "a failure printed '$got' on stdout"
	else
		[ "$code:$got" = "0:$want" ] || die "'$4' (exit $2): got '$got' (exit $code), want '$want': $(cat "$work/err")"
	fi
}
expect "published $digest" 0 "$digest" ""
expect fail 0 "sha256:short" ""
expect fail 0 "" ""
expect missing 1 "" "ERROR: ghcr.io/mindful-stack/reeve-cli:$v: not found"
expect missing 1 "" "ERROR: failed to resolve: MANIFEST_UNKNOWN: manifest unknown"
expect missing 1 "" "ERROR: NAME_UNKNOWN: name unknown: repository name not known to registry"
expect fail 1 "" "ERROR: denied: denied"
expect fail 1 "" "ERROR: unexpected status from HEAD request: 403 Forbidden"
expect fail 1 "" "ERROR: failed to authorize: 401 Unauthorized"
expect fail 1 "" "ERROR: denied: requested access to the resource is denied: not found"
expect fail 1 "" "ERROR: dial tcp: lookup ghcr.io: no such host"
expect fail 1 "" "ERROR: unexpected status: 500 Internal Server Error"
if PATH="$work/bin:$PATH" scripts/cli-bundle-lookup.sh not-a-version >/dev/null 2>&1; then die "accepted a bad CLI version"; fi
echo "cli-bundle-lookup test ok"
