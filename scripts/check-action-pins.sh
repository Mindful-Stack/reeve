#!/usr/bin/env bash
# Every third-party action in .github/workflows is pinned by a full commit SHA with its version as a trailing comment
# (`uses: owner/repo@<40 hex> # vX.Y.Z`), so a moved or hijacked tag cannot change what CI runs. Dependabot
# (.github/dependabot.yml) proposes the updates. Local actions (`./...`) and `docker://` images are exempt.
#
# The workflows are parsed with yq (mise.toml [tools]), so only real `uses:` keys count, in any YAML spelling: a job's
# (a reusable workflow) and a step's, also behind a YAML alias (Actions resolves anchors; explode(.) does too here). A
# file yq cannot parse fails.
set -euo pipefail
dir="${1:-.github/workflows}"
command -v yq >/dev/null || { echo "yq not found: run through mise (mise run lint)" >&2; exit 1; }
bad=0
shopt -s nullglob
for f in "$dir"/*.yml "$dir"/*.yaml; do
  if ! refs=$(yq -r 'explode(.) | .jobs[] | ((select(has("uses")) | .uses), (.steps[]? | select(has("uses")) | .uses)) | [., line_comment] | @tsv' "$f"); then
    echo "cannot parse $f" >&2
    bad=1
    continue
  fi
  while IFS=$'\t' read -r ref comment; do
    [ -n "$ref" ] || continue
    case "$ref" in
      ./* | docker://*) continue ;;
    esac
    if ! [[ "$ref" =~ ^[^@[:space:]]+@[0-9a-f]{40}$ ]] || [ -z "${comment//[[:space:]#]/}" ]; then
      echo "$f: not pinned by SHA with a version comment: uses: $ref${comment:+ # $comment}" >&2
      bad=1
    fi
  done <<<"$refs"
done
if [ "$bad" = 0 ]; then echo "action pins ok"; fi
exit "$bad"
