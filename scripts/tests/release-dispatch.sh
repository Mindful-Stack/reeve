#!/usr/bin/env bash
# scripts/tests/release-dispatch.sh — scripts/release-dispatch.sh against a fake GitHub API (a fake curl): the App's
# token is a valid RS256 JWT for its ID; every installation gets an Actions-only token; every repository it covers,
# across pages, gets reeve-release.yml dispatched on its default branch with the commit and digest; a repository
# without the workflow, a refused installation and a refused dispatch are counted, never fatal, and never named; the
# limit caps the dispatches; and no bearer token is ever on curl's command line.
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
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$work/app.pem" 2>/dev/null
openssl pkey -in "$work/app.pem" -pubout -out "$work/app.pub" 2>/dev/null
KEY=$(cat "$work/app.pem")
COMMIT=0123456789abcdef0123456789abcdef01234567
DIGEST=sha256:$(printf 'a%.0s' {1..64})

# The fake curl answers from $FAKE_ROUTES: lines `METHOD PATH AUTH<TAB>CODE<TAB>BODY`, where PATH is the URL without
# https://api.example and AUTH is `jwt` for a request carrying a valid App token for app 12345 (verified against the
# test key, at most ten minutes long, not expired), `bad-jwt` for any other JWT, otherwise the bearer itself. An
# unmatched request is a 404. Every call is logged: `METHOD PATH AUTH data=… args=…`.
cat >"$work/bin/curl" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
method=GET out="" url="" data="" bearer=""
args="$*"
while [ $# -gt 0 ]; do
	case "$1" in
	-X) method=$2; shift ;;
	-o) out=$2; shift ;;
	-w | --max-time) shift ;;
	--data) data=$2; shift ;;
	-H) case "$2" in @*) bearer=$(sed -n 's/^Authorization: Bearer //p' "${2#@}") ;; esac; shift ;;
	-*) ;;
	*) url=$1 ;;
	esac
	shift
done
[ "${FAKE_EXIT:-0}" -eq 0 ] || { echo "curl: (6) Could not resolve host" >&2; exit "$FAKE_EXIT"; }
path=${url#https://api.example}
unb64() { local s; s=$(tr -- '-_' '+/'); while [ $(( ${#s} % 4 )) -ne 0 ]; do s="$s="; done; printf '%s' "$s" | openssl base64 -d -A; }
auth=$bearer
IFS=. read -r h p s <<<"$bearer"
if [ -n "${s:-}" ]; then
	printf '%s' "$s" | unb64 >"$FAKE_DIR/sig"
	if printf '%s.%s' "$h" "$p" | openssl dgst -sha256 -verify "$FAKE_DIR/app.pub" -signature "$FAKE_DIR/sig" >/dev/null 2>&1 &&
		printf '%s' "$p" | unb64 | jq -e '.iss == "12345" and (.exp - .iat) <= 600 and .exp > now' >/dev/null; then
		auth=jwt
	else
		auth=bad-jwt
	fi
fi
printf '%s %s %s data=%s args=%s\n' "$method" "$path" "$auth" "$data" "$args" >>"$FAKE_DIR/log"
line=$(awk -F'\t' -v k="$method $path $auth" '$1 == k { print; exit }' "$FAKE_ROUTES")
if [ -z "$line" ]; then
	echo '{"message":"Not Found"}' >"$out"
	printf 404
	exit 0
fi
printf '%s' "$(cut -f3 <<<"$line")" >"$out"
printf '%s' "$(cut -f2 <<<"$line")"
FAKE
chmod +x "$work/bin/curl"

fails=0
fail() {
	echo "FAIL: $*" >&2
	fails=$((fails + 1))
}
routes=$work/routes
# route <method> <path> <auth> <code> <body>
route() { printf '%s %s %s\t%s\t%s\n' "$1" "$2" "$3" "$4" "$5" >>"$routes"; }
# run [VAR=value...]: runs the script as release.yml does; sets $out, $rc and $log.
run() {
	: >"$work/log"
	rc=0
	out=$(env -i PATH="$work/bin:$PATH" HOME="$HOME" FAKE_DIR="$work" FAKE_ROUTES="$routes" \
		GITHUB_API_URL=https://api.example RELEASE_APP_ID=12345 RELEASE_APP_PRIVATE_KEY="$KEY" \
		RELEASE_COMMIT="$COMMIT" RELEASE_DIGEST="$DIGEST" "$@" bash "$script" 2>&1) || rc=$?
	log=$(cat "$work/log")
}
calls() { if [ -z "$log" ]; then echo 0; else printf '%s\n' "$log" | wc -l | tr -d ' '; fi; }
expect() { # <output> <what>
	[ "$rc" -eq 0 ] || fail "$2: exit $rc, want 0: $out"
	[ "$out" = "$1" ] || fail "$2: output '$out', want '$1'"
}
# expect_warning <text> <what>: exit 0, one warning containing the text, and no subscriber named anywhere.
expect_warning() {
	[ "$rc" -eq 0 ] || fail "$2: exit $rc, want 0"
	case "$out" in "::warning::release dispatch: "*"$1"*) ;; *) fail "$2: want a warning with '$1', got: $out" ;; esac
	if printf '%s\n' "$out" | grep -qE 'acme|globex|initech'; then fail "$2: the output names a subscriber: $out"; fi
}
dispatched() { printf '%s\n' "$log" | grep -c '^POST /repos/.*/dispatches ' || true; }
repos_json() { # <full_name:branch>...
	local items=() r
	for r in "$@"; do items+=("$(jq -cn --arg n "${r%%:*}" --arg b "${r#*:}" '{full_name: $n, default_branch: $b}')"); done
	printf '{"total_count":%d,"repositories":[%s]}' "$#" "$(
		IFS=,
		echo "${items[*]}"
	)"
}

# Unconfigured, half-configured, malformed: nothing is called.
: >"$routes"
run RELEASE_APP_ID= RELEASE_APP_PRIVATE_KEY=
expect "release dispatch: not configured, nothing to do" "unconfigured"
[ "$(calls)" -eq 0 ] || fail "unconfigured: curl was called"
run RELEASE_APP_PRIVATE_KEY=
expect_warning "RELEASE_APP_ID is set but RELEASE_APP_PRIVATE_KEY is not" "key missing"
run RELEASE_APP_ID=
expect_warning "RELEASE_APP_PRIVATE_KEY is set but RELEASE_APP_ID is not" "id missing"
run RELEASE_APP_ID="12 345"
expect_warning "not an App ID" "bad id"
run RELEASE_APP_PRIVATE_KEY="not a key"
expect_warning "does not sign" "bad key"
run RELEASE_COMMIT=main
expect_warning "RELEASE_COMMIT" "bad commit"
run RELEASE_DIGEST=latest
expect_warning "RELEASE_DIGEST" "bad digest"
[ "$(calls)" -eq 0 ] || fail "malformed: curl was called"

# Three installations. 11 covers two repositories (one without the workflow); 22 covers 101 repositories across two
# pages; 33's token is refused. One of 22's dispatches is refused (422: the workflow does not declare the inputs).
: >"$routes"
route GET "/app/installations?per_page=100&page=1" jwt 200 '[{"id":11},{"id":22},{"id":33}]'
for id in 11 22; do route POST "/app/installations/$id/access_tokens" jwt 201 "{\"token\":\"tok-$id\"}"; done
route POST "/app/installations/33/access_tokens" jwt 403 '{"message":"suspended"}'
route GET "/installation/repositories?per_page=100&page=1" tok-11 200 "$(repos_json acme/deployments:main acme/website:trunk)"
page1=()
for i in $(seq 1 100); do page1+=("globex/r$i:main"); done
route GET "/installation/repositories?per_page=100&page=1" tok-22 200 "$(repos_json "${page1[@]}")"
route GET "/installation/repositories?per_page=100&page=2" tok-22 200 "$(repos_json globex/r101:release)"
route POST "/repos/acme/deployments/actions/workflows/reeve-release.yml/dispatches" tok-11 204 ""
for i in $(seq 1 101); do
	code=204
	[ "$i" -ne 7 ] || code=422
	route POST "/repos/globex/r$i/actions/workflows/reeve-release.yml/dispatches" tok-22 "$code" ""
done

run RELEASE_DISPATCH_MAX=200
expect_warning "3 installation(s), 103 repositor(ies): 101 started, 1 without .github/workflows/reeve-release.yml, 2 refused (HTTP 422 403)" "subscribers"
if printf '%s\n' "$log" | grep -q ' bad-jwt '; then fail "subscribers: an invalid App token was sent"; fi
want='POST /repos/acme/deployments/actions/workflows/reeve-release.yml/dispatches tok-11 data={"ref":"main","inputs":{"commit":"'$COMMIT'","digest":"'$DIGEST'"}}'
case "$log" in *"$want "*) ;; *) fail "subscribers: no dispatch $want" ;; esac
printf '%s\n' "$log" | grep -q '^POST /repos/globex/r101/actions/workflows/reeve-release.yml/dispatches tok-22 data={"ref":"release",' ||
	fail "subscribers: the second page's repository was not dispatched on its default branch"
if printf '%s\n' "$log" | grep '^POST /app/installations/[0-9]*/access_tokens' | grep -qv 'data={"permissions":{"actions":"write"}}'; then
	fail "subscribers: an installation token asked for more than actions: write"
fi
if printf '%s\n' "$log" | grep -q 'args=.*\(tok-\|eyJ\)'; then fail "subscribers: a bearer token is on curl's command line"; fi
[ "$(dispatched)" -eq 103 ] || fail "subscribers: $(dispatched) dispatches, want 103"

# Everyone subscribed and accepted: no warning.
: >"$routes"
route GET "/app/installations?per_page=100&page=1" jwt 200 '[{"id":11}]'
route POST "/app/installations/11/access_tokens" jwt 201 '{"token":"tok-11"}'
route GET "/installation/repositories?per_page=100&page=1" tok-11 200 "$(repos_json initech/infra:main)"
route POST "/repos/initech/infra/actions/workflows/reeve-release.yml/dispatches" tok-11 204 ""
run
expect "release dispatch: 1 installation(s), 1 repositor(ies): 1 started, 0 without .github/workflows/reeve-release.yml" "one subscriber"

# The limit: past it, nothing more is dispatched and a warning says how many were left out.
: >"$routes"
route GET "/app/installations?per_page=100&page=1" jwt 200 '[{"id":11}]'
route POST "/app/installations/11/access_tokens" jwt 201 '{"token":"tok-11"}'
route GET "/installation/repositories?per_page=100&page=1" tok-11 200 "$(repos_json acme/a:main acme/b:main acme/c:main)"
for r in a b c; do route POST "/repos/acme/$r/actions/workflows/reeve-release.yml/dispatches" tok-11 204 ""; done
run RELEASE_DISPATCH_MAX=2
expect_warning "3 repositor(ies): 2 started, 0 without .github/workflows/reeve-release.yml, 1 over the limit of 2" "limit"
[ "$(dispatched)" -eq 2 ] || fail "limit: $(dispatched) dispatches, want 2"

# No installations: nothing to do, no warning.
: >"$routes"
route GET "/app/installations?per_page=100&page=1" jwt 200 '[]'
run
expect "release dispatch: 0 installation(s), 0 repositor(ies): 0 started, 0 without .github/workflows/reeve-release.yml" "no subscribers"

# The App itself refused (a JWT for another App ID: the fake answers it 401), and curl failing.
: >"$routes"
route GET "/app/installations?per_page=100&page=1" bad-jwt 401 '{"message":"bad credentials"}'
run RELEASE_APP_ID=99999
expect_warning "installation list (HTTP 401)" "wrong app"
run FAKE_EXIT=6
expect_warning "installation list (HTTP 000)" "curl failed"

if [ "$fails" -ne 0 ]; then
	echo "release-dispatch test: $fails failure(s)" >&2
	exit 1
fi
echo "release-dispatch test ok"
