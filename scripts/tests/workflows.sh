#!/usr/bin/env bash
# scripts/tests/workflows.sh — the release workflow's shell: release.yml sets `defaults: run: shell: bash` at workflow
# level, which GitHub runs as `bash --noprofile --norc -eo pipefail {0}` (the implicit default is `bash -e {0}`,
# without pipefail, where a failure before a pipe is lost), and no job or step overrides it.
set -euo pipefail
cd "$(dirname "$0")/../.."
die() { echo "FAIL: $*" >&2; exit 1; }
if command -v shellcheck >/dev/null 2>&1; then shellcheck scripts/tests/workflows.sh; elif [ -n "${CI:-}" ]; then die "shellcheck required in CI"; fi

# The default, at workflow level, and no other shell: anywhere in the file.
has_default() { # the three lines, top level, consecutive
	awk 'prev2 == "defaults:" && prev1 == "  run:" && $0 == "    shell: bash" { found = 1 } { prev2 = prev1; prev1 = $0 } END { exit !found }' "$1"
}
f=.github/workflows/release.yml
if grep -q '| *tee' "$f" && ! has_default "$f"; then die "$f pipes into tee without defaults: run: shell: bash (the pipe's status is lost)"; fi
has_default "$f" || die "$f lacks the workflow-level defaults: run: shell: bash"
others=$(grep -n '^ *shell:' "$f" | grep -v '^[0-9]*:    shell: bash$' || true)
[ -z "$others" ] || die "$f overrides the shell: $others"
echo "workflows test ok"
