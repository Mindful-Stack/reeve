#!/usr/bin/env bash
# scripts/tests/workflows.sh — the release and CLI workflows' shell: release.yml and cli.yml set
# `defaults: run: shell: bash` at workflow level, which GitHub runs as `bash --noprofile --norc -eo pipefail {0}` (the
# implicit default is `bash -e {0}`, without pipefail, where a failure before a pipe is lost), and no job or step
# overrides it.
set -euo pipefail
cd "$(dirname "$0")/../.."
die() { echo "FAIL: $*" >&2; exit 1; }
if command -v shellcheck >/dev/null 2>&1; then shellcheck scripts/tests/workflows.sh; elif [ -n "${CI:-}" ]; then die "shellcheck required in CI"; fi

# The default, at workflow level, and no other shell: anywhere in the file.
has_default() { # the three lines, top level, consecutive
	awk 'prev2 == "defaults:" && prev1 == "  run:" && $0 == "    shell: bash" { found = 1 } { prev2 = prev1; prev1 = $0 } END { exit !found }' "$1"
}
for f in .github/workflows/release.yml .github/workflows/cli.yml; do
	if grep -q '| *tee' "$f" && ! has_default "$f"; then die "$f pipes into tee without defaults: run: shell: bash (the pipe's status is lost)"; fi
	has_default "$f" || die "$f lacks the workflow-level defaults: run: shell: bash"
	others=$(grep -n '^ *shell:' "$f" | grep -v '^[0-9]*:    shell: bash$' || true)
	[ -z "$others" ] || die "$f overrides the shell: $others"
done

# cli.yml runs on every pull request that can change the CLI: each input scripts/cli-version.sh names (a directory, or
# a file) is matched by one of its pull_request.paths patterns (bash globbing: `*` and `**` both cross `/`, which is
# looser than GitHub's `*` only for patterns that contain `/` after it; none here does).
mapfile -t inputs < <(awk '/^inputs=\($/ { on = 1; next } on && /^\)$/ { exit } on { gsub(/^[ \t]+|[ \t]+$/, ""); print }' scripts/cli-version.sh)
[ "${#inputs[@]}" -gt 0 ] || die "found no inputs=( … ) list in scripts/cli-version.sh"
mapfile -t paths < <(awk '
	/^  pull_request:$/ { pr = 1; next }
	pr && /^    paths:$/ { on = 1; next }
	on && /^      - / { sub(/^      - /, ""); gsub(/"/, ""); print; next }
	on || (pr && /^  [^ ]/) { exit }' .github/workflows/cli.yml)
[ "${#paths[@]}" -gt 0 ] || die "found no pull_request.paths in .github/workflows/cli.yml"
for i in "${inputs[@]}"; do
	hit=
	for p in "${paths[@]}"; do
		# shellcheck disable=SC2053 # $p is the glob
		if [[ "$i" == $p || "$i/x" == $p ]]; then hit=1; break; fi
	done
	[ -n "$hit" ] || die "the CLI input $i is not in .github/workflows/cli.yml's pull_request.paths: a change to it would run no CLI build before main"
done
echo "workflows test ok"
