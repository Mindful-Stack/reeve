#!/usr/bin/env bash
# scripts/tests/cli-unix.sh <binary-a> <version-a> <binary-b> <version-b> <target> — one end-to-end smoke on a Linux or
# macOS leg of cli.yml (kb/ADR-0030 R2, R5, R7), with two stamped builds the leg just made: install.sh from a loopback
# stand-in (scripts/tests/cli-serve.py) with the runner's real curl and checksum tools, under umask 077, then
# `reeve update` from build A to build B with a poisoned REEVE_TOKEN, a poisoned stored login and a proxy that does not
# answer. Checks the installed bytes, mode 755, that the binary runs, that the update asked for this target's own file,
# sent no credentials and used no proxy.
#
# On the macOS x86_64 leg (an arm64 runner, the build under Rosetta) install.sh rightly picks the native arm64 build,
# which this leg does not have, so build A is copied into place instead and only the update runs.
set -euo pipefail
cd "$(dirname "$0")/../.."
die() { echo "FAIL: $*" >&2; exit 1; }
[ $# -eq 5 ] || die "usage: cli-unix.sh <binary-a> <version-a> <binary-b> <version-b> <target>"
a=$1 va=$2 b=$3 vb=$4 target=$5
for tool in python3 curl; do command -v "$tool" >/dev/null || die "$tool not found"; done
if command -v sha256sum >/dev/null; then hash_of() { sha256sum "$1" | cut -d' ' -f1; }; else hash_of() { shasum -a 256 "$1" | cut -d' ' -f1; }; fi
[ "$(hash_of "$a")" != "$(hash_of "$b")" ] || die "builds A and B are the same bytes: the update would prove nothing"
export REEVE_UPDATE_NOTICE=0
file=$(scripts/cli-build.sh --file "$target")
work=$(mktemp -d)
pid=
trap '[ -z "$pid" ] || kill "$pid" 2>/dev/null || true; rm -rf "$work"' EXIT

# bundle <root> <binary> <version>: cli/ holding VERSION, this target's file and a SHA256SUMS naming every target.
bundle() {
	mkdir -p "$1/cli"
	cp "$2" "$1/cli/$file"
	printf '%s\n' "$3" >"$1/cli/VERSION"
	local t f
	for t in $(scripts/cli-build.sh --targets); do
		f=$(scripts/cli-build.sh --file "$t")
		if [ "$f" = "$file" ]; then printf '%s  %s\n' "$(hash_of "$2")" "$f"; else printf '%064d  %s\n' 0 "$f"; fi
	done >"$1/cli/SHA256SUMS"
}
# serve <root>: (re)starts the stand-in on <root>; sets $url; renders install.sh with it.
serve() {
	[ -z "$pid" ] || { kill "$pid"; wait "$pid" 2>/dev/null || true; }
	rm -f "$work/port"
	: >"$work/log"
	python3 -I scripts/tests/cli-serve.py "$1" "$work/port" "$work/log" 2>"$work/serve.err" &
	pid=$!
	local _
	for _ in $(seq 100); do [ -s "$work/port" ] && break; sleep 0.1; done
	[ -s "$work/port" ] || die "the stand-in did not start: $(cat "$work/serve.err")"
	url="http://127.0.0.1:$(cat "$work/port")"
	sed "s|@REEVE_URL@|$url|" deploy/cli/install.sh >"$1/cli/install.sh"
}

bundle "$work/a" "$a" "$va"
serve "$work/a"
dir=$work/bin
if [ "$target" = x86_64-apple-darwin ] && [ "$(uname -m)" = arm64 ]; then
	mkdir -p "$dir"
	cp "$a" "$dir/reeve"
	chmod 755 "$dir/reeve"
	echo "cli-unix: x86_64 macOS build on an arm64 runner: install.sh picks the arm64 build here, so A is copied in place"
else
	(umask 077 && curl -q -fsS "$url/cli/install.sh" | env -u REEVE_FORCE REEVE_INSTALL_DIR="$dir" HOME="$work" sh) >"$work/out" 2>&1 ||
		die "install.sh failed: $(cat "$work/out")"
	[ "$(hash_of "$dir/reeve")" = "$(hash_of "$a")" ] || die "install.sh installed other bytes than the served build"
	[ -n "$(find "$dir/reeve" -perm 755)" ] || die "under umask 077 the installed reeve is not mode 755"
	grep -q "^installed reeve build ${va:0:7} at " "$work/out" || die "install.sh did not name the build: $(cat "$work/out")"
	grep -qF "GET /cli/$file " "$work/log" || die "install.sh did not ask for $file: $(cat "$work/log")"
	if grep -q ' auth$' "$work/log"; then die "install.sh sent credentials: $(cat "$work/log")"; fi
fi
"$dir/reeve" --version | grep -q '^reeve-planner ' || die "the installed reeve does not run"

# The update reads the server url from a stored login whose token is poisoned, with REEVE_TOKEN poisoned too: neither
# may reach the server. A proxy that answers nothing must not carry a loopback download.
bundle "$work/b" "$b" "$vb"
serve "$work/b"
cfg=$work/config
env REEVE_CONFIG_DIR="$cfg" "$dir/reeve" auth set-token --url "$url" --token poisoned-stored >/dev/null ||
	die "storing the poisoned login failed"
env -u REEVE_URL -u NO_PROXY -u no_proxy REEVE_CONFIG_DIR="$cfg" REEVE_TOKEN=poisoned-env \
	HTTP_PROXY=http://127.0.0.1:9 http_proxy=http://127.0.0.1:9 ALL_PROXY=http://127.0.0.1:9 all_proxy=http://127.0.0.1:9 \
	"$dir/reeve" update >"$work/out" 2>&1 || die "reeve update failed: $(cat "$work/out")"
[ "$(hash_of "$dir/reeve")" = "$(hash_of "$b")" ] || die "reeve update did not install the build the server offers"
[ -n "$(find "$dir/reeve" -perm -100)" ] || die "the updated reeve is not executable"
"$dir/reeve" --version | grep -q '^reeve-planner ' || die "the updated reeve does not run"
grep -qF "GET /cli/$file " "$work/log" || die "reeve update did not ask for $file: $(cat "$work/log")"
if grep -v " /cli/$file " "$work/log" | grep -q ' /cli/reeve-'; then die "reeve update asked for another target's file: $(cat "$work/log")"; fi
if grep -q ' auth$' "$work/log"; then die "reeve update sent credentials: $(cat "$work/log")"; fi
[ -z "$(find "$dir" -mindepth 1 -maxdepth 1 ! -name reeve)" ] || die "reeve update left files beside reeve: $(ls -A "$dir")"

# Up to date: B against B replaces nothing and downloads no binary.
: >"$work/log"
env -u REEVE_URL REEVE_CONFIG_DIR="$cfg" "$dir/reeve" update >"$work/out" 2>&1 || die "an up-to-date reeve update failed: $(cat "$work/out")"
if grep -q " /cli/$file " "$work/log"; then die "an up-to-date reeve update downloaded the binary: $(cat "$work/log")"; fi
[ "$(hash_of "$dir/reeve")" = "$(hash_of "$b")" ] || die "an up-to-date reeve update changed the binary"
echo "cli-unix ok"
