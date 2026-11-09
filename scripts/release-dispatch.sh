#!/usr/bin/env bash
# scripts/release-dispatch.sh — release.yml's `notify` job: once an image is published, start one workflow in another
# repository through GitHub's workflow_dispatch. An operator who deploys from their own repository then hears about
# the release within minutes, instead of from a scheduled poll, which GitHub may start hours late.
#
# Configured by two repository secrets, so a public repository's run logs mask them and name no operator:
#   RELEASE_DISPATCH_TARGET  owner/repo/workflow-file.yml, optionally @ref (default: that repository's default branch)
#   RELEASE_DISPATCH_TOKEN   a token that may dispatch it: a fine-grained token with Actions read and write on that one
#                            repository and nothing else (deploy/README.md, "Tell your deployment about a release")
#
# Never fails the release. Unconfigured, it does nothing. Misconfigured, or when GitHub refuses, it writes a warning
# that gives the HTTP status and never the target, and exits 0: the image is published either way, and the operator's
# own schedule still catches up.
set -euo pipefail
target=${RELEASE_DISPATCH_TARGET:-}
token=${RELEASE_DISPATCH_TOKEN:-}
api=${GITHUB_API_URL:-https://api.github.com}
warn() {
	echo "::warning::release dispatch: $*"
	exit 0
}

if [ -z "$target" ] && [ -z "$token" ]; then
	echo "release dispatch: not configured, nothing to do"
	exit 0
fi
[ -n "$target" ] || warn "RELEASE_DISPATCH_TOKEN is set but RELEASE_DISPATCH_TARGET is not"
[ -n "$token" ] || warn "RELEASE_DISPATCH_TARGET is set but RELEASE_DISPATCH_TOKEN is not"
re='^([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+\.ya?ml)(@([A-Za-z0-9_./-]+))?$'
[[ $target =~ $re ]] || warn "RELEASE_DISPATCH_TARGET is not owner/repo/workflow.yml[@ref]"
repo="${BASH_REMATCH[1]}/${BASH_REMATCH[2]}"
workflow=${BASH_REMATCH[3]}
ref=${BASH_REMATCH[5]}

body=$(mktemp)
trap 'rm -f "$body"' EXIT
# call <method> <path> [json]: prints the HTTP status, or 000 when curl itself failed. The token goes to curl as a
# header file, never as an argument.
call() {
	local code
	code=$(curl -sS --max-time 30 -X "$1" -o "$body" -w '%{http_code}' \
		-H @<(printf 'Authorization: Bearer %s\n' "$token") \
		-H 'Accept: application/vnd.github+json' -H 'X-GitHub-Api-Version: 2022-11-28' \
		${3:+--data "$3"} "$api/repos/$repo$2" 2>/dev/null) || code=000
	echo "$code"
}

if [ -z "$ref" ]; then
	code=$(call GET "")
	[ "$code" = 200 ] || warn "could not read the target repository (HTTP $code)"
	ref=$(jq -r '.default_branch // empty' "$body")
	[ -n "$ref" ] || warn "the target repository reported no default branch"
fi
code=$(call POST "/actions/workflows/$workflow/dispatches" "$(jq -cn --arg ref "$ref" '{ref: $ref}')")
[ "$code" = 204 ] || warn "GitHub refused the dispatch (HTTP $code)"
echo "release dispatch: started the configured workflow"
