#!/usr/bin/env bash
# scripts/tests/ci-needs-rust.sh — scripts/ci-needs-rust.sh says `false` (ci.yml skips lint and test) only when every
# changed path is one the Rust jobs cannot see: the web client (bar the two files the Rust tests read), docs, lore and
# top-level markdown. Anything else, a path it does not know, and an empty list say `true`. Also the workflow wiring
# around it: lint and test still run when `changes` fails.
set -euo pipefail
cd "$(dirname "$0")/../.."
script=scripts/ci-needs-rust.sh
bash -n "$script"
if command -v shellcheck >/dev/null 2>&1; then
	shellcheck "$script" scripts/tests/ci-needs-rust.sh
	echo "shellcheck ok"
elif [ -n "${CI:-}" ]; then
	echo "shellcheck required in CI" >&2
	exit 1
else
	echo "shellcheck not installed: syntax checked with bash -n only"
fi

fails=0
# check <expected> <path>...: the paths, one per line on stdin.
check() {
	local want=$1 got
	shift
	got=$(printf '%s\n' "$@" | "$script")
	if [ "$got" != "$want" ]; then
		echo "FAIL: [$*] gave '$got', want '$want'" >&2
		fails=$((fails + 1))
	fi
}

check false web/src/lib/components/DecisionList.svelte
check false web/src/lib/components/DecisionList.svelte web/tests/smoke.spec.ts docs/specs/x.md
check false lore/knowledge/adrs/0014-sprints-and-team-goals.md README.md CLAUDE.md
check false web/package.json web/package-lock.json
# The Rust tests read these two web files (crates/reeve/src/tui/words.rs, crates/reeve/tests/glyphs_snapshot.rs).
check true web/src/lib/board.ts
check true web/src/lib/glyphs.json
check true web/src/lib/promise.fixture.json
check true web/src/app.css web/src/lib/glyphs.json
check true crates/reeve-server/src/main.rs
check true Cargo.lock
check true rust-toolchain.toml
check true mise.toml
check true reeve.example.toml
check true .github/workflows/ci.yml
check true scripts/test.sh
# Markdown inside a crate or a script directory is not top-level markdown.
check true crates/reeve/README.md
# A path it does not know runs everything; so does nothing at all (a failed listing must never skip the jobs).
check true some-new-file.txt
check true web/src/app.css new-dir/thing
printf '' | "$script" | grep -qx true || { echo "FAIL: an empty list did not say true" >&2; fails=$((fails + 1)); }

# The workflows around it. A job whose `needs` failed is skipped unless its `if` checks a status itself, and a skipped
# job reports success: lint and test must evaluate their own condition (`!cancelled()`), so a failed `changes` job
# runs them (its output is then empty) instead of skipping them.
for job in lint test; do
	cond=$(yq -r ".jobs.$job.if" .github/workflows/ci.yml)
	case "$cond" in
	*'!cancelled()'*"needs.changes.outputs.rust != 'false'"*) ;;
	*) echo "FAIL: ci.yml's $job runs on '$cond', not on !cancelled() and the changes output" >&2; fails=$((fails + 1)) ;;
	esac
done

[ "$fails" -eq 0 ] || exit 1
echo "ci-needs-rust ok"
