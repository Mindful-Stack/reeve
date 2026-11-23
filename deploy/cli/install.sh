#!/bin/sh
# Installs the reeve CLI from the Reeve server that served this script (kb/ADR-0030). The server replaces the quoted
# placeholder below with its public_url when it starts; everything else is the same on every instance.
#
#   curl --proto '=https' --tlsv1.2 -fsS https://<server>/cli/install.sh | sh
#
# REEVE_INSTALL_DIR chooses the directory (default ~/.local/bin); REEVE_FORCE=1 replaces a `reeve` there that is not
# the Reeve planner CLI. Everything runs from `main`, called on the last line, so a download cut short runs nothing.
set -eu

REEVE_URL='@REEVE_URL@'

die() {
	echo "reeve install: $*" >&2
	exit 1
}

# fetch <name>: <server>/cli/<name> into $tmp/<name>. No credentials: -q, first, skips ~/.curlrc, and curl reads
# ~/.netrc only with -n/--netrc, never given. No redirect: without -L curl does not follow one but saves its body, so
# anything but a 200 fails.
fetch() {
	if [ "$proto" = '=https' ]; then
		code=$(curl -q --proto "$proto" --tlsv1.2 -sS -o "$tmp/$1" -w '%{http_code}' "$REEVE_URL/cli/$1") ||
			die "downloading $1 failed"
	else
		code=$(curl -q --proto "$proto" -sS -o "$tmp/$1" -w '%{http_code}' "$REEVE_URL/cli/$1") ||
			die "downloading $1 failed"
	fi
	[ "$code" = 200 ] || die "downloading $1: the server answered HTTP $code"
}

main() {
	# https only; plain http only to a loopback address, for local testing. Checked before anything is downloaded.
	case "$REEVE_URL" in
	https://*)
		proto='=https'
		rerun="curl --proto '=https' --tlsv1.2 -fsS $REEVE_URL/cli/install.sh"
		;;
	http://127.0.0.1 | http://127.0.0.1:* | http://localhost | http://localhost:* | 'http://[::1]' | 'http://[::1]:'*)
		proto='=http'
		rerun="curl -fsS $REEVE_URL/cli/install.sh"
		;;
	*) die "refusing $REEVE_URL: downloads go over https only (plain http only to a loopback address)" ;;
	esac

	os=$(uname -s)
	arch=$(uname -m)
	case "$os/$arch" in
	Linux/x86_64 | Linux/amd64) file=reeve-linux-x86_64 ;;
	Linux/aarch64 | Linux/arm64) file=reeve-linux-aarch64 ;;
	Darwin/arm64 | Darwin/aarch64) file=reeve-macos-aarch64 ;;
	Darwin/x86_64) file=reeve-macos-x86_64 ;;
	*) die "no build for $os $arch. Supported: Linux x86_64 and arm64, macOS arm64 and x86_64; on Windows, use install.ps1" ;;
	esac

	command -v curl >/dev/null 2>&1 || die "needs curl"
	if command -v sha256sum >/dev/null 2>&1; then
		hasher="sha256sum"
	elif command -v shasum >/dev/null 2>&1; then
		hasher="shasum -a 256"
	else
		die "needs sha256sum or shasum"
	fi

	tmp=$(mktemp -d)
	staged=""
	trap 'rm -rf "$tmp"; [ -z "$staged" ] || rm -f "$staged"' EXIT
	trap 'exit 130' INT
	trap 'exit 143' TERM

	# The build the server offers: a 40-character commit id, shown by its first seven.
	fetch VERSION
	build=$(head -n 1 "$tmp/VERSION")
	case "$build" in
	*[!0-9a-f]* | '') die "the server's VERSION is not a build ($build): is this a Reeve server with CLI distribution on?" ;;
	esac
	[ "${#build}" -eq 40 ] || die "the server's VERSION is not a build ($build)"
	short=$(printf '%s\n' "$build" | awk '{ print substr($0, 1, 7) }')

	fetch SHA256SUMS
	lines=$(awk -v f="$file" '$2 == f { n++ } END { print n + 0 }' "$tmp/SHA256SUMS")
	[ "$lines" = 1 ] || die "SHA256SUMS has $lines lines for $file, want exactly 1"
	want=$(awk -v f="$file" '$2 == f { print $1 }' "$tmp/SHA256SUMS")
	fetch "$file"
	# shellcheck disable=SC2086 # $hasher is a command and its flags
	got=$($hasher "$tmp/$file" | awk '{ print $1 }')
	[ "$got" = "$want" ] || die "checksum mismatch for $file: nothing was installed"

	dir=${REEVE_INSTALL_DIR:-$HOME/.local/bin}
	target=$dir/reeve
	if [ -e "$target" ]; then
		first=$("$target" --version 2>/dev/null | head -n 1) || first=""
		case "$first" in
		reeve-planner*) ;;
		*)
			[ "${REEVE_FORCE:-}" = 1 ] ||
				die "$target is not the Reeve planner CLI (it printed: ${first:-nothing}). To replace it, rerun with:
  $rerun | REEVE_FORCE=1 sh"
			;;
		esac
	fi
	elsewhere="To install somewhere else, rerun with:
  $rerun | REEVE_INSTALL_DIR=<directory> sh"
	mkdir -p "$dir" 2>/dev/null || die "cannot create $dir. $elsewhere"
	# Staged in the target directory, made executable after the copy (cp applies the umask), run once, then renamed
	# over the target: an existing reeve keeps working until the rename, and on any failure.
	staged=$dir/.reeve.new.$$
	cp "$tmp/$file" "$staged" 2>/dev/null || die "cannot write to $dir. $elsewhere"
	chmod 755 "$staged"
	"$staged" --version >/dev/null 2>&1 || die "the downloaded reeve does not run on this machine; nothing was replaced"
	mv -f "$staged" "$target" || die "cannot replace $target; nothing was replaced"
	staged=""

	echo "installed reeve build $short at $target"
	case ":${PATH:-}:" in
	*":$dir:"*) ;;
	*)
		echo
		echo "$dir is not on your PATH. For this shell, run:"
		echo "  export PATH=\"$dir:\$PATH\""
		case "${SHELL:-}" in
		*/fish) echo "To keep it in fish, run: fish_add_path $dir" ;;
		*/zsh) echo "To keep it, add that line to ~/.zshrc" ;;
		*) echo "To keep it, add that line to ~/.profile (or your shell's startup file)" ;;
		esac
		echo "Or run it by its full path: $target"
		;;
	esac
	echo
	echo "next: reeve auth login --url $REEVE_URL"
	echo "  (or: $target auth login --url $REEVE_URL)"
	echo "then: reeve tui"
}

main "$@"
