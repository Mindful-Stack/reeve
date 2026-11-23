#!/usr/bin/env bash
# scripts/tests/cli-version.sh — scripts/cli-version.sh moves the CLI version on a commit that changes one of its
# inputs and on no other, and refuses a shallow clone. Runs against a throwaway git history, never this repository's.
set -euo pipefail
cd "$(dirname "$0")/../.."
script=scripts/cli-version.sh
bash -n "$script"
if command -v shellcheck >/dev/null 2>&1; then
	shellcheck "$script" scripts/tests/cli-version.sh
elif [ -n "${CI:-}" ]; then
	echo "shellcheck required in CI" >&2
	exit 1
fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
repo=$tmp/repo
g() { git -C "$repo" -c user.name=test -c user.email=test@example.com -c init.defaultBranch=main "$@"; }
mkdir -p "$repo/scripts"
g init -q
cp "$script" "$repo/scripts/cli-version.sh"
# commit <message> <path>...: appends a comment line to each path, commits, prints HEAD.
commit() {
	local msg=$1 p
	shift
	for p in "$@"; do
		mkdir -p "$repo/$(dirname "$p")"
		echo "# $msg" >>"$repo/$p"
	done
	g add -A
	g commit -q -m "$msg"
	g rev-parse HEAD
}
fails=0
expect() {
	local got
	got=$(cd "$repo" && bash scripts/cli-version.sh)
	if [ "$got" != "$1" ]; then
		echo "FAIL: $2: got '$got', want '$1'" >&2
		fails=$((fails + 1))
	fi
}
base=$(commit base crates/reeve/src/main.rs)
expect "$base" "the first commit"
for p in crates/reeve-server/src/lib.rs crates/reeve-planning/src/lib.rs web/src/app.css docs/x.md lore/x.md \
	.github/workflows/cli.yml .github/workflows/release.yml rust-toolchain.toml deploy/cli/install.sh \
	deploy/cli/install.ps1 deploy/README.md Dockerfile README.md mise.toml; do
	commit "not an input: $p" "$p" >/dev/null
	expect "$base" "a change to $p"
done
for p in crates/reeve/src/lib.rs crates/reeve-protocol/src/lib.rs Cargo.toml Cargo.lock scripts/cli-version.sh \
	scripts/cli-build.sh deploy/cli/Dockerfile; do
	head=$(commit "input: $p" "$p")
	expect "$head" "a change to $p"
done
git clone -q --depth 1 "file://$repo" "$tmp/shallow"
if out=$(cd "$tmp/shallow" && bash scripts/cli-version.sh 2>/dev/null); then
	echo "FAIL: a shallow clone printed '$out'" >&2
	fails=$((fails + 1))
fi
[ "$fails" -eq 0 ] || exit 1
echo "cli-version ok"
