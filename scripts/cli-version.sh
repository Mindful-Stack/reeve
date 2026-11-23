#!/usr/bin/env bash
# scripts/cli-version.sh — prints the CLI version: the full id of the last commit that changed one of the reeve CLI's
# build inputs (kb/ADR-0030 R1). This list is the only place the inputs are named: an include_str!/include_bytes! (or
# a build script reading a file) in the CLI's non-test code that reaches outside it must add that path here. Not
# inputs, on purpose: rust-toolchain.toml (each CLI version is published once, so its floating channel cannot make two
# published builds of one version; to ship a compiler or standard-library fix, change an input, e.g. bump the
# workspace version in Cargo.toml), .github/workflows/cli.yml (Dependabot bumps its pins; a pin changes no binary and
# must not ask anyone to update), the install scripts (part of the server). Needs the full history: a shallow clone
# fails rather than print a wrong version.
set -euo pipefail
top=$(git -C "$(dirname "$0")" rev-parse --show-toplevel 2>/dev/null) || {
	echo "cli-version: not in a git repository; run it from a full clone of Reeve" >&2
	exit 1
}
cd "$top"
inputs=(
	crates/reeve
	crates/reeve-protocol
	Cargo.toml
	Cargo.lock
	.cargo
	scripts/cli-version.sh
	scripts/cli-build.sh
	deploy/cli/Dockerfile
)
if [ "$(git rev-parse --is-shallow-repository)" != false ]; then
	echo "cli-version: a shallow clone cannot tell the CLI version; check out with fetch-depth: 0" >&2
	exit 1
fi
version=$(git log -1 --format=%H -- "${inputs[@]}")
if ! [[ "$version" =~ ^[0-9a-f]{40}$ ]]; then
	echo "cli-version: no commit changes the CLI's inputs (got '$version')" >&2
	exit 1
fi
echo "$version"
