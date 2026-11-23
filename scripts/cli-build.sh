#!/usr/bin/env bash
# scripts/cli-build.sh — builds one release target of the reeve CLI stamped with its CLI version, runs and checks the
# binary on this runner, and copies it to <out-dir>/<file name> (kb/ADR-0030 R2). The targets and their bundle file
# names live here; crates/reeve-server/tests/cli_files.rs keeps them equal to reeve_protocol::CLI_BINARIES. A CLI input
# (scripts/cli-version.sh). The compiler is rust-toolchain.toml's channel: each CLI version is built once, so a
# floating stable never makes two different builds of one version.
#
#   scripts/cli-build.sh <target> <cli-version> <out-dir>   build, check, copy; prints the copied file's path
#   scripts/cli-build.sh --targets                          the supported targets, one per line
#   scripts/cli-build.sh --file <target>                    a target's file name in the bundle
set -euo pipefail
TARGETS=(x86_64-unknown-linux-musl aarch64-unknown-linux-musl aarch64-apple-darwin x86_64-apple-darwin x86_64-pc-windows-msvc)
file_of() {
	case "$1" in
	x86_64-unknown-linux-musl) echo reeve-linux-x86_64 ;;
	aarch64-unknown-linux-musl) echo reeve-linux-aarch64 ;;
	aarch64-apple-darwin) echo reeve-macos-aarch64 ;;
	x86_64-apple-darwin) echo reeve-macos-x86_64 ;;
	x86_64-pc-windows-msvc) echo reeve-windows-x86_64.exe ;;
	*)
		echo "cli-build: unknown target '$1' (known: ${TARGETS[*]})" >&2
		return 1
		;;
	esac
}
fail() {
	echo "cli-build: $*" >&2
	exit 1
}
case "${1:-}" in
--targets)
	printf '%s\n' "${TARGETS[@]}"
	exit 0
	;;
--file)
	file_of "${2:?usage: cli-build.sh --file <target>}"
	exit 0
	;;
esac
[ $# -eq 3 ] || fail "usage: cli-build.sh <target> <cli-version> <out-dir>"
target=$1 version=$2 out=$3
file=$(file_of "$target")
[[ "$version" =~ ^[0-9a-f]{40}$ ]] || fail "the CLI version must be 40 lowercase hex characters, got '$version'"
cd "$(dirname "$0")/.."

command -v rustup >/dev/null || fail "rustup not found"
rustup target add "$target"
case "$target" in
*-linux-musl)
	# ring compiles C; the cc crate reads CC_<target> (underscores) for the target's compiler.
	command -v musl-gcc >/dev/null || fail "musl-gcc not found (apt-get install musl-tools)"
	export "CC_${target//-/_}=musl-gcc"
	;;
esac
REEVE_CLI_VERSION=$version cargo build -p reeve --release --locked --target "$target"

built=${CARGO_TARGET_DIR:-target}/$target/release/reeve
case "$target" in *-windows-*) built=$built.exe ;; esac
[ -f "$built" ] || fail "no binary at $built"

# Run it on this runner (R2): macOS x86_64 under Rosetta. --version never touches the network.
run=("$built")
[ "$target" != x86_64-apple-darwin ] || run=(arch -x86_64 "$built")
said=$("${run[@]}" --version) || fail "$built --version failed"
case "$said" in "reeve-planner "*) ;; *) fail "$built --version printed '$said'" ;; esac
# The stamp is compiled in (option_env!), so the version string is in the bytes; without it this is an unstamped build.
grep -qaF "$version" "$built" || fail "$built does not carry the CLI version $version"
case "$target" in
*-linux-musl)
	command -v readelf >/dev/null || fail "readelf not found"
	# LC_ALL=C: readelf's messages are translated, and "program interpreter" is the C locale's wording.
	if LC_ALL=C readelf -l "$built" | grep -q 'program interpreter'; then fail "$built is not static"; fi
	;;
aarch64-apple-darwin) [ "$(lipo -archs "$built")" = arm64 ] || fail "$built is not arm64 only" ;;
x86_64-apple-darwin) [ "$(lipo -archs "$built")" = x86_64 ] || fail "$built is not x86_64 only" ;;
esac
mkdir -p "$out"
cp "$built" "$out/$file"
echo "$out/$file"
