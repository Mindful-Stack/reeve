#!/usr/bin/env bash
# scripts/release-dispatch.sh — release.yml's `notify` job: tell every subscriber that an image was published, minutes
# after it was, so nobody has to poll on a schedule GitHub may start hours late.
#
# Subscribing is installing the release GitHub App on a repository: the job starts that repository's
# `.github/workflows/reeve-release.yml` on its default branch through workflow_dispatch, with two string inputs,
# `commit` (the 40-character commit) and `digest` (the image's sha256:… digest), both of which the workflow must
# declare (deploy/README.md, "Subscribe to releases"). The App asks for one permission, Actions read and write, so all
# it can do in a subscriber's repository is start, re-run and cancel workflows. There is no list of subscribers to
# keep: installing subscribes, uninstalling unsubscribes.
#
# Inputs (environment): RELEASE_APP_ID (the App's ID or client ID) and RELEASE_APP_PRIVATE_KEY (its PEM key), both
# repository secrets; RELEASE_COMMIT and RELEASE_DIGEST. GITHUB_API_URL is set by Actions. RELEASE_DISPATCH_MAX caps
# how many repositories one release starts (default 100), so the App's being installable by anyone can't stall a
# release.
#
# Never fails the release, and never names a subscriber: this repository's run logs are public. Unconfigured, it does
# nothing; otherwise it prints counts and HTTP statuses only, a warning when anything was refused, and exits 0.
set -euo pipefail
app_id=${RELEASE_APP_ID:-}
key=${RELEASE_APP_PRIVATE_KEY:-}
api=${GITHUB_API_URL:-https://api.github.com}
max=${RELEASE_DISPATCH_MAX:-100}
workflow=reeve-release.yml
warn() {
	echo "::warning::release dispatch: $*"
	exit 0
}

if [ -z "$app_id" ] && [ -z "$key" ]; then
	echo "release dispatch: not configured, nothing to do"
	exit 0
fi
[ -n "$app_id" ] || warn "RELEASE_APP_PRIVATE_KEY is set but RELEASE_APP_ID is not"
[ -n "$key" ] || warn "RELEASE_APP_ID is set but RELEASE_APP_PRIVATE_KEY is not"
[[ $app_id =~ ^[A-Za-z0-9.]+$ ]] || warn "RELEASE_APP_ID is not an App ID or client ID"
commit=${RELEASE_COMMIT:-}
digest=${RELEASE_DIGEST:-}
[[ $commit =~ ^[0-9a-f]{40}$ ]] || warn "RELEASE_COMMIT is not a 40-character commit"
[[ $digest =~ ^sha256:[0-9a-f]{64}$ ]] || warn "RELEASE_DIGEST is not a sha256 digest"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
umask 077
body=$work/body

# The App's JSON Web Token (RS256, ten minutes at most; iat a minute back for clock drift).
b64url() { openssl base64 -A | tr '+/' '-_' | tr -d '='; }
printf '%s\n' "$key" >"$work/key.pem"
now=$(date +%s)
unsigned="$(printf '{"alg":"RS256","typ":"JWT"}' | b64url).$(printf '{"iat":%d,"exp":%d,"iss":"%s"}' $((now - 60)) $((now + 540)) "$app_id" | b64url)"
sig=$(printf '%s' "$unsigned" | openssl dgst -sha256 -sign "$work/key.pem" -binary 2>/dev/null | b64url) || sig=""
[ -n "$sig" ] || warn "RELEASE_APP_PRIVATE_KEY does not sign (not a PEM private key)"
jwt="$unsigned.$sig"
rm -f "$work/key.pem"

# call <method> <path> <bearer> [json]: the HTTP status on stdout (000 when curl itself failed), the body in $body.
# The bearer goes to curl as a header file, never as an argument.
call() {
	local code
	code=$(curl -sS --max-time 30 -X "$1" -o "$body" -w '%{http_code}' \
		-H @<(printf 'Authorization: Bearer %s\n' "$3") \
		-H 'Accept: application/vnd.github+json' -H 'X-GitHub-Api-Version: 2022-11-28' \
		${4:+--data "$4"} "$api$2" 2>/dev/null) || code=000
	echo "$code"
}

inputs=$(jq -cn --arg commit "$commit" --arg digest "$digest" '{commit: $commit, digest: $digest}')
installations=0 repos=0 started=0 missing=0 refused=0 skipped=0
codes=""
refuse() {
	refused=$((refused + 1))
	codes="$codes $1"
}

page=1
while :; do
	code=$(call GET "/app/installations?per_page=100&page=$page" "$jwt")
	[ "$code" = 200 ] || warn "GitHub refused the App's installation list (HTTP $code)"
	ids=$(jq -r '.[].id' "$body")
	[ -n "$ids" ] || break
	for id in $ids; do
		installations=$((installations + 1))
		code=$(call POST "/app/installations/$id/access_tokens" "$jwt" '{"permissions":{"actions":"write"}}')
		if [ "$code" != 201 ]; then
			refuse "$code"
			continue
		fi
		token=$(jq -r '.token // empty' "$body")
		[ -n "$token" ] || {
			refuse "$code"
			continue
		}
		rpage=1
		while :; do
			code=$(call GET "/installation/repositories?per_page=100&page=$rpage" "$token")
			if [ "$code" != 200 ]; then
				refuse "$code"
				break
			fi
			jq -r '.repositories[] | "\(.full_name) \(.default_branch)"' "$body" >"$work/repos"
			[ -s "$work/repos" ] || break
			while read -r repo branch; do
				repos=$((repos + 1))
				if [ "$repos" -gt "$max" ]; then
					skipped=$((skipped + 1))
					continue
				fi
				code=$(call POST "/repos/$repo/actions/workflows/$workflow/dispatches" "$token" \
					"$(jq -cn --arg ref "$branch" --argjson inputs "$inputs" '{ref: $ref, inputs: $inputs}')")
				case "$code" in
				204) started=$((started + 1)) ;;
				404) missing=$((missing + 1)) ;;
				*) refuse "$code" ;;
				esac
			done <"$work/repos"
			[ "$(wc -l <"$work/repos")" -ge 100 ] || break
			rpage=$((rpage + 1))
		done
	done
	[ "$(printf '%s\n' "$ids" | wc -l)" -ge 100 ] || break
	page=$((page + 1))
done

summary="$installations installation(s), $repos repositor(ies): $started started, $missing without .github/workflows/$workflow"
[ "$skipped" -eq 0 ] || summary="$summary, $skipped over the limit of $max"
if [ "$refused" -gt 0 ]; then
	warn "$summary, $refused refused (HTTP$codes)"
fi
[ "$skipped" -eq 0 ] || warn "$summary"
echo "release dispatch: $summary"
