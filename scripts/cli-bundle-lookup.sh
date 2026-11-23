#!/usr/bin/env bash
# scripts/cli-bundle-lookup.sh <cli-version> — whether ghcr.io/mindful-stack/reeve-cli:<cli-version> is published
# (kb/ADR-0030 R1), for cli.yml. Asks the registry with `docker buildx imagetools inspect` (logged in beforehand) and
# classifies the answer conservatively:
#
#   published <digest>   the tag exists (exit 0)
#   missing              the registry clearly said the tag or the package does not exist: "not found",
#                        "manifest unknown" or "name unknown", with no sign of an authorization failure (exit 0)
#   (anything else)      an authorization failure (denied, unauthorized, forbidden, 401, 403), any other error, or an
#                        unexpected answer: the registry's message on stderr, exit 1. Never read as "missing", so a
#                        registry or token problem fails the run instead of rebuilding or publishing over a bundle.
#
# REEVE_CLI_IMAGE overrides the repository (scripts/tests/cli-bundle-lookup.sh).
set -euo pipefail
die() { echo "cli-bundle-lookup: $*" >&2; exit 1; }
[ $# -eq 1 ] || die "usage: cli-bundle-lookup.sh <cli-version>"
[[ "$1" =~ ^[0-9a-f]{40}$ ]] || die "the CLI version must be 40 lowercase hex characters, got '$1'"
ref=${REEVE_CLI_IMAGE:-ghcr.io/mindful-stack/reeve-cli}:$1
err=$(mktemp)
trap 'rm -f "$err"' EXIT
if digest=$(docker buildx imagetools inspect "$ref" --format '{{.Manifest.Digest}}' 2>"$err"); then
	[[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]] || die "unexpected answer for $ref: '$digest'"
	echo "published $digest"
elif grep -qiE 'denied|unauthori[sz]ed|forbidden|\b40[13]\b' "$err"; then
	cat "$err" >&2
	die "the registry refused the lookup of $ref (an authorization failure, not an answer about the tag)"
elif grep -qiE ': not found$|manifest unknown|name unknown' "$err"; then
	echo "missing"
else
	cat "$err" >&2
	die "the lookup of $ref failed"
fi
