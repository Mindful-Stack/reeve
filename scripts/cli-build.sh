#!/usr/bin/env bash
# scripts/cli-build.sh — builds one release target of the reeve CLI stamped with its CLI version, runs and checks the
# binary on this runner, and copies it to <out-dir>/<file name> (kb/ADR-0030 R2). The targets and their bundle file
# names live here; crates/reeve-server/tests/cli_files.rs keeps them equal to reeve_protocol::CLI_BINARIES. This script
# is a CLI input (scripts/cli-version.sh names it). The compiler is rust-toolchain.toml's channel: each CLI version is
# built once, so a floating stable never makes two different builds of one version.
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
# absolute <path>: <path> against the caller's directory (a Windows drive path counts as absolute), before the cd below.
absolute() {
	case "$1" in
	/* | [A-Za-z]:[/\\]*) echo "$1" ;;
	*) echo "$PWD/$1" ;;
	esac
}
out=$(absolute "$out")
# The target directory is named here, so a build.target-dir in a cargo config cannot move the binary elsewhere.
[ -z "${CARGO_TARGET_DIR:-}" ] || CARGO_TARGET_DIR=$(absolute "$CARGO_TARGET_DIR")
cd "$(dirname "$0")/.."
# Relative to the repository root, where cargo runs (a POSIX absolute path would need converting for cargo on Windows).
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-target}
unset CARGO_BUILD_TARGET_DIR

# The release binary is what the inputs in scripts/cli-version.sh make, not what the runner's environment adds: no
# flags or profile overrides from it. The script sets only the variables below.
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS CARGO_BUILD_RUSTFLAGS
for v in $(compgen -e); do
	case "$v" in
	CARGO_PROFILE_RELEASE_* | CARGO_TARGET_*_RUSTFLAGS) unset "$v" ;;
	esac
done

command -v rustup >/dev/null || fail "rustup not found"
rustup target add "$target"
case "$target" in
*-linux-musl)
	# ring compiles C; the cc crate reads CC_<target> (underscores) for the target's compiler.
	command -v musl-gcc >/dev/null || fail "musl-gcc not found (apt-get install musl-tools)"
	export "CC_${target//-/_}=musl-gcc"
	;;
*-windows-msvc)
	# The C runtime linked statically: a clean Windows has no VCRUNTIME140.dll unless something installed it.
	export "CARGO_TARGET_$(printf '%s' "${target//-/_}" | tr '[:lower:]' '[:upper:]')_RUSTFLAGS=-C target-feature=+crt-static"
	;;
# The oldest macOS each binary runs on, pinned (today's rustc defaults) so a toolchain update cannot raise it unseen;
# checked below.
aarch64-apple-darwin) export MACOSX_DEPLOYMENT_TARGET=11.0 ;;
x86_64-apple-darwin) export MACOSX_DEPLOYMENT_TARGET=10.12 ;;
esac
REEVE_CLI_VERSION=$version cargo build -p reeve --release --locked --target "$target"

built=$CARGO_TARGET_DIR/$target/release/reeve
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
*-windows-msvc)
	# An import of the runtime DLL names it in the import table; a static runtime does not.
	if grep -qaiF vcruntime140.dll "$built"; then fail "$built links VCRUNTIME140.dll dynamically"; fi
	;;
*-apple-darwin)
	case "$target" in aarch64-*) arch=arm64 ;; *) arch=x86_64 ;; esac
	[ "$(lipo -archs "$built")" = "$arch" ] || fail "$built is not $arch only"
	# The minimum macOS: LC_BUILD_VERSION's minos, or LC_VERSION_MIN_MACOSX's version on older deployment targets.
	minos=$(otool -l "$built" | awk '
		$1 == "cmd" { cmd = $2 }
		cmd == "LC_BUILD_VERSION" && $1 == "minos" { print $2; exit }
		cmd == "LC_VERSION_MIN_MACOSX" && $1 == "version" { print $2; exit }')
	[ "$minos" = "$MACOSX_DEPLOYMENT_TARGET" ] ||
		fail "$built needs macOS ${minos:-unknown}, want $MACOSX_DEPLOYMENT_TARGET"
	;;
esac
mkdir -p "$out"
cp "$built" "$out/$file"
echo "$out/$file"
