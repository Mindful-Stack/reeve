#!/bin/sh
# Installs the reeve CLI from the Reeve server that served this script (kb/ADR-0030). The server replaces the quoted
# placeholder below with its public_url when it starts; everything else is the same on every instance.
#
#   curl -q --proto '=https' --tlsv1.2 -fsS https://<server>/cli/install.sh | sh
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
# anything but a 200 fails. A loopback download never goes through a proxy (--noproxy '*'): a configured http_proxy
# would otherwise carry plain http off the machine, where the binary and its checksum could both be replaced.
fetch() {
	if [ "$proto" = '=https' ]; then
		code=$(curl -q --proto "$proto" --tlsv1.2 -sS -o "$tmp/$1" -w '%{http_code}' "$REEVE_URL/cli/$1") ||
			die "downloading $1 failed"
	else
		code=$(curl -q --noproxy '*' --proto "$proto" -sS -o "$tmp/$1" -w '%{http_code}' "$REEVE_URL/cli/$1") ||
			die "downloading $1 failed"
	fi
	case "$code" in
	200) ;;
	429) die "downloading $1: the server's proxy is rate-limiting /cli/; wait a minute and rerun" ;;
	5??) die "downloading $1: the server answered HTTP $code; it may be restarting: wait a minute and rerun" ;;
	*) die "downloading $1: the server answered HTTP $code" ;;
	esac
}

main() {
	# https only; plain http only to a loopback address, for local testing. Checked before anything is downloaded.
	case "$REEVE_URL" in
	https://*)
		proto='=https'
		rerun="curl -q --proto '=https' --tlsv1.2 -fsS $REEVE_URL/cli/install.sh"
		;;
	http://127.0.0.1 | http://localhost | 'http://[::1]') proto='=http' ;;
	http://127.0.0.1:* | http://localhost:* | 'http://[::1]:'*)
		# Strip the known host and colon; what is left must be a port, digits only (no userinfo, path or second host).
		case "$REEVE_URL" in
		http://127.0.0.1:*) port=${REEVE_URL#http://127.0.0.1:} ;;
		http://localhost:*) port=${REEVE_URL#http://localhost:} ;;
		*) port=${REEVE_URL#'http://[::1]:'} ;;
		esac
		case "$port" in
		*[!0-9]* | '') die "refusing $REEVE_URL: downloads go over https only (plain http only to a loopback address)" ;;
		esac
		proto='=http'
		;;
	*) die "refusing $REEVE_URL: downloads go over https only (plain http only to a loopback address)" ;;
	esac
	[ -n "${rerun:-}" ] || rerun="curl -q --noproxy '*' -fsS $REEVE_URL/cli/install.sh"

	os=$(uname -s)
	arch=$(uname -m)
	case "$os/$arch" in
	Linux/x86_64 | Linux/amd64) file=reeve-linux-x86_64 ;;
	Linux/aarch64 | Linux/arm64) file=reeve-linux-aarch64 ;;
	Darwin/arm64 | Darwin/aarch64) file=reeve-macos-aarch64 ;;
	Darwin/x86_64)
		# An x86_64 shell under Rosetta on an Apple silicon Mac: the native build runs faster.
		if [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = 1 ]; then
			file=reeve-macos-aarch64
		else
			file=reeve-macos-x86_64
		fi
		;;
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

	# The target first, so a run that would refuse it downloads nothing.
	elsewhere="To install somewhere else, rerun with:
  $rerun | REEVE_INSTALL_DIR=<directory> sh"
	if [ -z "${REEVE_INSTALL_DIR:-}" ] && [ -z "${HOME:-}" ]; then
		die "HOME is not set, so there is no default directory. $elsewhere"
	fi
	dir=${REEVE_INSTALL_DIR:-$HOME/.local/bin}
	mkdir -p -- "$dir" 2>/dev/null || die "cannot create $dir. $elsewhere"
	dir=$(CDPATH='' cd -- "$dir" && pwd) || die "cannot use $dir. $elsewhere"
	target=$dir/reeve
	[ ! -d "$target" ] || die "$target is a directory; remove it or choose another directory. $elsewhere"
	if [ -e "$target" ]; then
		first=$("$target" --version </dev/null 2>/dev/null | head -n 1) || first=""
		case "$first" in
		reeve-planner*) ;;
		*)
			[ "${REEVE_FORCE:-}" = 1 ] ||
				die "$target is not the Reeve planner CLI (it printed: ${first:-nothing}). To replace it, rerun with:
  $rerun | REEVE_FORCE=1 sh"
			;;
		esac
	fi
	# Staged in the target directory (a private file from mktemp, which also proves the directory writable), filled
	# after the download, made executable after the copy (cp applies the umask), run once, then renamed over the
	# target: an existing reeve keeps working until the rename, and on any failure.
	staged=$(mktemp "$dir/.reeve.new.XXXXXX" 2>/dev/null) || die "cannot write to $dir. $elsewhere"

	# The build the server offers: a 40-character CLI version, shown by its first seven.
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
	got=$($hasher <"$tmp/$file" | awk '{ print $1 }')
	# Mid-deploy the sums and the binary can come from two releases: a rerun gets one release's pair.
	[ "$got" = "$want" ] || die "checksum mismatch for $file: nothing was installed. Rerun; if it repeats, tell whoever runs the server"

	cp "$tmp/$file" "$staged" 2>/dev/null || die "cannot write to $dir. $elsewhere"
	chmod 755 "$staged"
	ran=$("$staged" --version </dev/null 2>/dev/null | head -n 1) || ran=""
	case "$ran" in
	reeve-planner*) ;;
	*) die "the downloaded reeve does not run on this machine; nothing was replaced" ;;
	esac
	mv -f "$staged" "$target" || die "cannot replace $target; nothing was replaced"
	staged=""

	printf 'installed reeve build %s at %s\n' "$short" "$target"
	case ":${PATH:-}:" in
	*":$dir:"* | *":$dir/:"*) ;;
	*)
		echo
		printf '%s is not on your PATH. For this shell, run:\n' "$dir"
		# shellcheck disable=SC2016 # the literal $PATH is the text to print
		printf '  export PATH="%s:%s"\n' "$dir" '$PATH'
		case "${SHELL:-}" in
		*/fish) printf 'To keep it in fish, run: fish_add_path "%s"\n' "$dir" ;;
		*/zsh) echo "To keep it, add that line to ~/.zshrc" ;;
		*/bash)
			# Terminal.app starts login shells, which read ~/.bash_profile and not ~/.bashrc.
			if [ "$os" = Darwin ]; then
				echo "To keep it, add that line to ~/.bash_profile"
			else
				echo "To keep it, add that line to ~/.bashrc"
			fi
			;;
		*) echo "To keep it, add that line to ~/.profile (or your shell's startup file)" ;;
		esac
		printf 'Or run it by its full path: %s\n' "$target"
		;;
	esac
	# Another reeve earlier on PATH (a cargo-installed one in ~/.cargo/bin, the runner's old CLI) would run instead of
	# this one, silently. Compared by physical path, so a PATH entry through a symlink to $dir is not a stranger.
	found=$(command -v reeve 2>/dev/null) || found=""
	case "$found" in
	/*)
		found_dir=$(CDPATH='' cd -- "${found%/*}" 2>/dev/null && pwd -P) || found_dir=""
		if [ "$found_dir/${found##*/}" != "$(cd -- "$dir" && pwd -P)/reeve" ]; then
			other=$("$found" --version </dev/null 2>/dev/null | head -n 1) || other=""
			echo
			printf 'warning: another reeve comes first on your PATH: %s (it prints: %s)\n' "$found" "${other:-nothing}"
			printf 'Remove it, or put %s ahead of %s in PATH; until then run %s\n' "$dir" "${found%/*}" "$target"
		fi
		;;
	esac
	echo
	printf 'next: reeve auth login --url %s\n' "$REEVE_URL"
	printf '  (or: %s auth login --url %s)\n' "$target" "$REEVE_URL"
	echo "then: reeve tui"
}

main "$@"
