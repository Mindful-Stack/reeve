#!/usr/bin/env bash
# scripts/tests/action-pins.sh — scripts/check-action-pins.sh accepts the repository's workflows and rejects every
# unpinned shape (a tag, a branch, a SHA without its version comment, a short SHA, a quoted tag) in any YAML spelling,
# at step and job level, and ignores text that only looks like a `uses:` key.
set -euo pipefail
cd "$(dirname "$0")/../.."
script=scripts/check-action-pins.sh
bash -n "$script"
if command -v shellcheck >/dev/null 2>&1; then
	shellcheck "$script" scripts/tests/action-pins.sh
	echo "shellcheck ok"
elif [ -n "${CI:-}" ]; then
	echo "shellcheck required in CI" >&2
	exit 1
else
	echo "shellcheck not installed: syntax checked with bash -n only"
fi

"$script" >/dev/null || { echo "FAIL: the repository's workflows are not all pinned" >&2; exit 1; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
sha=11d5960a326750d5838078e36cf38b85af677262

# check <expected status> <uses value>: one workflow holding a single step with that `uses:`.
check() {
	local want=$1 uses=$2 got=0
	printf 'jobs:\n  a:\n    steps:\n      - name: step\n        uses: %s\n' "$uses" >"$tmp/w.yml"
	"$script" "$tmp" >/dev/null 2>&1 || got=$?
	if [ "$got" != "$want" ]; then
		echo "FAIL: uses: $uses exited $got, want $want" >&2
		exit 1
	fi
}

check 0 "actions/checkout@$sha # v4.4.0"
check 0 "\"actions/checkout@$sha\" # v4.4.0"
check 0 "docker/build-push-action@$sha #v7.4.0"
check 0 "./.github/actions/local"
check 0 "docker://alpine:3.20"
check 1 "actions/checkout@v4"
check 1 "\"actions/checkout@v4\" # v4"
check 1 "dtolnay/rust-toolchain@stable"
check 1 "actions/checkout@$sha"
check 1 "actions/checkout@${sha:0:12} # v4.4.0"
check 1 "actions/checkout@${sha}0 # v4.4.0"
check 1 "actions/checkout # v4.4.0"

# checkraw <expected status> <step line>: one workflow whose single step is written as given.
checkraw() {
	local want=$1 step=$2 got=0
	printf 'jobs:\n  a:\n    steps:\n      %s\n' "$step" >"$tmp/w.yml"
	"$script" "$tmp" >/dev/null 2>&1 || got=$?
	if [ "$got" != "$want" ]; then
		echo "FAIL: step $step exited $got, want $want" >&2
		exit 1
	fi
}

checkraw 1 "- { uses: actions/checkout@v4 }"
checkraw 1 "- { name: x, uses: actions/checkout@$sha }"
checkraw 1 "- 'uses': actions/checkout@v4"
checkraw 0 "- \"uses\" : actions/checkout@$sha # v4.4.0"
checkraw 1 "- uses : actions/checkout@v4"
checkraw 1 "- { name: x, uses: actions/checkout@$sha # v4.4.0 }"
checkraw 0 "- uses: actions/checkout@$sha # v4.4.0"
checkraw 0 "# - uses: actions/checkout@v4"
checkraw 0 "- run: echo pinned"
# In a flow mapping the comment belongs to the mapping, not the value: rejected, write the block form.
checkraw 1 "- { uses: actions/checkout@$sha } # v4.4.0"
checkraw 0 "- 'uses': actions/checkout@$sha # v4.4.0"

# Text that only looks like a key is not one: a block scalar mentioning uses:, an input named uses.
printf 'jobs:\n  a:\n    steps:\n      - run: |\n          echo "uses: actions/checkout@v4"\n      - uses: ./local\n        with:\n          uses: actions/checkout@v4\n' >"$tmp/w.yml"
"$script" "$tmp" >/dev/null || { echo "FAIL: a block scalar or an input named uses was taken for a key" >&2; exit 1; }

# A reusable workflow at job level is checked too; an unparsable file fails.
printf 'jobs:\n  a:\n    uses: org/repo/.github/workflows/x.yml@main\n' >"$tmp/w.yml"
if "$script" "$tmp" >/dev/null 2>&1; then echo "FAIL: an unpinned reusable workflow passed" >&2; exit 1; fi
# A step behind a YAML alias (Actions resolves anchors) is checked as the step it names: pinned passes, a tag fails.
printf 'x: &s\n  uses: actions/checkout@%s # v4.4.0\njobs:\n  a:\n    steps:\n      - *s\n' "$sha" >"$tmp/w.yml"
"$script" "$tmp" >/dev/null || { echo "FAIL: a pinned step behind an alias was rejected" >&2; exit 1; }
printf 'x: &s\n  uses: actions/checkout@v4\njobs:\n  a:\n    steps:\n      - *s\n' >"$tmp/w.yml"
if "$script" "$tmp" >/dev/null 2>&1; then echo "FAIL: an unpinned step behind an alias passed" >&2; exit 1; fi
printf 'jobs: [\n' >"$tmp/w.yml"
if "$script" "$tmp" >/dev/null 2>&1; then echo "FAIL: an unparsable workflow passed" >&2; exit 1; fi
echo "action-pins: all cases ok"
