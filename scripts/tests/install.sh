#!/usr/bin/env bash
# scripts/tests/install.sh — deploy/cli/install.sh (kb/ADR-0030 R5) under POSIX sh, against a fake curl and uname and
# the real checksum tools: the platform map, the build check, the single checksum line, the foreign-binary refusal,
# PATH guidance, an existing installation surviving every failure, and that a download sends no credentials, follows
# no redirect and goes over https (plain http only to loopback). The installer runs with PATH limited to the fakes and
# the tools it needs, so a missing sha256sum is a real absence.
#
# The fake curl behaves like curl where a mistake would matter: it reads $HOME/.curlrc unless -q comes first and
# $HOME/.netrc only with -n/--netrc*, and follows a redirect with -L/--location*. Every run has a poisoned .curlrc and
# .netrc, so an installer that drops -q or adds -n logs "poisoned", and one that adds -L installs the redirect's target.
set -euo pipefail
cd "$(dirname "$0")/../.."
# The whole suite runs under every POSIX-ish shell that is present: the default sh, dash, busybox sh, bash --posix.
if [ -z "${INSTALL_TEST_SHELL:-}" ]; then
	ran=0 bad=0
	for cand in "sh" "dash" "busybox sh" "bash --posix"; do
		first=${cand%% *}
		abs=$(command -v "$first" 2>/dev/null) || { echo "$first not installed: skipped"; continue; }
		case "$cand" in *" "*) cmd="$abs ${cand#* }" ;; *) cmd=$abs ;; esac
		echo "== installer shell: $cmd"
		INSTALL_TEST_SHELL=$cmd bash "$0" || bad=1
		ran=$((ran + 1))
	done
	if [ "$bad" != 0 ] || [ "$ran" -eq 0 ]; then
		echo "install.sh test failed" >&2
		exit 1
	fi
	exit 0
fi
read -r -a testsh <<<"$INSTALL_TEST_SHELL"
template=deploy/cli/install.sh
sh -n "$template"
if command -v shellcheck >/dev/null 2>&1; then
	shellcheck -s sh "$template"
	shellcheck scripts/tests/install.sh
	echo "shellcheck ok"
elif [ -n "${CI:-}" ]; then
	echo "shellcheck required in CI" >&2
	exit 1
else
	echo "shellcheck not installed: syntax checked with sh -n only"
fi
if [ "$(grep -c "'@REEVE_URL@'" "$template")" != 1 ]; then
	echo "FAIL: $template must hold the placeholder exactly once" >&2
	exit 1
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/install test.XXXXXX") # a space in every path
trap 'rm -rf "$work"' EXIT
fails=0
fail() {
	echo "FAIL: $*" >&2
	sed 's/^/    /' "$work/out" >&2 2>/dev/null || true
	fails=$((fails + 1))
}

# The served files: five fake binaries (each prints its own name, so a mix-up shows), a real SHA256SUMS, VERSION.
root=$work/root
mkdir -p "$root"
build=$(printf 'abcdef0%033d' 0)
names=(reeve-linux-x86_64 reeve-linux-aarch64 reeve-macos-aarch64 reeve-macos-x86_64 reeve-windows-x86_64.exe)
fixture() { printf '#!/bin/sh\necho "reeve-planner 0.7.0 (%s)"\n' "$1" >"$root/$1"; }
sums() {
	if command -v sha256sum >/dev/null 2>&1; then
		(cd "$root" && sha256sum "${names[@]}")
	else
		(cd "$root" && shasum -a 256 "${names[@]}")
	fi
}
for n in "${names[@]}"; do fixture "$n"; done
sums >"$work/SHA256SUMS.good"
cp "$work/SHA256SUMS.good" "$root/SHA256SUMS"
printf '%s\n' "$build" >"$root/VERSION"

# make_bin <dir> <tool to leave out>...: the fakes and links to the real tools the installer uses.
make_bin() {
	local dir=$1 t skip
	shift
	mkdir -p "$dir"
	cat >"$dir/curl" <<'EOF'
#!/bin/sh
printf 'curl %s\n' "$*" >>"$FAKE_LOG"
[ "$1" = -q ] || { [ ! -f "$HOME/.curlrc" ] || printf 'curlrc read: %s\n' "$(cat "$HOME/.curlrc")" >>"$FAKE_LOG"; }
out="" fmt="" url="" follow=""
while [ $# -gt 0 ]; do
	case "$1" in
	-o) out=$2; shift ;;
	-w) fmt=$2; shift ;;
	--proto) shift ;;
	-L | --location | --location-trusted) follow=1 ;;
	-n | --netrc | --netrc-optional | --netrc-file*) [ ! -f "$HOME/.netrc" ] || printf 'netrc read: %s\n' "$(cat "$HOME/.netrc")" >>"$FAKE_LOG" ;;
	http://* | https://*) url=$1 ;;
	esac
	shift
done
name=${url##*/cli/}
if [ "${FAKE_REDIRECT:-}" = "$name" ] && [ -z "$follow" ]; then
	code=302; printf 'moved' >"$out"
elif [ -f "$FAKE_ROOT/$name" ]; then
	code=200; cat "$FAKE_ROOT/$name" >"$out"
else
	code=404; printf 'not found' >"$out"
fi
[ -z "$fmt" ] || printf '%s' "$code"
EOF
	cat >"$dir/uname" <<'EOF'
#!/bin/sh
case "$1" in -s) echo "$FAKE_OS" ;; -m) echo "$FAKE_ARCH" ;; esac
EOF
	cat >"$dir/sysctl" <<'EOF'
#!/bin/sh
[ -n "$FAKE_TRANSLATED" ] || exit 1
echo "$FAKE_TRANSLATED"
EOF
	chmod 755 "$dir/curl" "$dir/uname" "$dir/sysctl"
	for t in sh awk cat chmod cp head mkdir mktemp mv rm sha256sum shasum; do
		for skip in "$@"; do [ "$t" != "$skip" ] || continue 2; done
		if command -v "$t" >/dev/null 2>&1; then ln -s "$(command -v "$t")" "$dir/$t"; fi
	done
}
make_bin "$work/bin"
make_bin "$work/bin-shasum" sha256sum

# render <url>: the template with the URL as the server writes it (single-quoted; these URLs hold no quote).
render() { sed "s|'@REEVE_URL@'|'$1'|" "$template" >"$work/install.sh"; }

# fresh [reeve --version line]: a new HOME with poisoned client configs (and, given a line, an existing reeve).
installed=$work/home/.local/bin/reeve
fresh() {
	rm -rf "${work:?}/home" "${work:?}/tmp"
	mkdir -p "$work/home" "$work/tmp"
	: >"$work/log"
	printf 'header = "Authorization: Bearer poisoned-curlrc"\n' >"$work/home/.curlrc"
	printf 'machine reeve.test login someone password poisoned-netrc\n' >"$work/home/.netrc"
	if [ $# -gt 0 ]; then
		mkdir -p "$work/home/.local/bin"
		printf '#!/bin/sh\necho "%s"\n' "$1" >"$installed"
		chmod 755 "$installed"
	fi
}
# run [VAR=value...]: the installer under sh with PATH=$bin; status in $status, output in $work/out.
bin=$work/bin
run() {
	status=0
	umask "${mask:-022}"
	env -i HOME="$work/home" PATH="$bin" TMPDIR="$work/tmp" FAKE_ROOT="$root" FAKE_LOG="$work/log" \
		FAKE_OS="${os:-Linux}" FAKE_ARCH="${arch:-x86_64}" FAKE_TRANSLATED="${translated:-}" "$@" "${testsh[@]}" "$work/install.sh" >"$work/out" 2>&1 || status=$?
	umask 022
}
install() {
	fresh
	run "$@"
}
# refused <what> <message pattern>: the run failed with that message and installed nothing.
refused() {
	if [ "$status" = 0 ]; then
		fail "$1: accepted"
	elif ! grep -q -- "$2" "$work/out"; then
		fail "$1: no '$2' in the output"
	elif [ -e "$installed" ]; then
		fail "$1: a reeve was installed anyway"
	fi
}
# installed_from <what> <served name>: the run succeeded and installed exactly that file, executable.
installed_from() {
	if [ "$status" != 0 ]; then
		fail "$1: exited $status"
	elif ! cmp -s "$root/$2" "$installed"; then
		fail "$1: did not install $2"
	elif [ ! -x "$installed" ]; then
		fail "$1: the installed reeve is not executable"
	elif [ -n "$(find "$(dirname "$installed")" -name '.reeve.new.*')" ]; then
		fail "$1: a staged file was left behind"
	fi
}
no_credentials() {
	if grep -q 'poisoned' "$work/log"; then fail "$1: a client config or .netrc was read: $(cat "$work/log")"; fi
	if grep -Eq -- '(^| )(-H|-b|-u|-n|--header|--cookie|--user|--netrc[^ ]*)( |$)' "$work/log"; then
		fail "$1: a download carried credentials: $(cat "$work/log")"
	fi
}
# kept <what> <version line>: the run failed and the existing reeve still answers.
kept() {
	if [ "$status" = 0 ]; then
		fail "$1: accepted"
	elif [ "$("$installed" --version)" != "$2" ]; then
		fail "$1: the existing reeve was changed"
	elif [ -n "$(find "$(dirname "$installed")" -name '.reeve.new.*')" ]; then
		fail "$1: a staged file was left behind"
	fi
}
nothing_left() { [ -z "$(ls -A "$work/tmp")" ] || fail "$1: the temporary directory was left behind"; }

render https://reeve.test

# Every supported platform downloads its own file and installs it from the checksum it was given.
for case in Linux/x86_64:reeve-linux-x86_64 Linux/amd64:reeve-linux-x86_64 Linux/aarch64:reeve-linux-aarch64 \
	Linux/arm64:reeve-linux-aarch64 Darwin/arm64:reeve-macos-aarch64 Darwin/x86_64:reeve-macos-x86_64; do
	plat=${case%%:*} want=${case#*:}
	os=${plat%/*} arch=${plat#*/} install
	installed_from "$plat" "$want"
	grep -q "curl .*https://reeve.test/cli/$want\$" "$work/log" || fail "$plat did not download $want"
	nothing_left "$plat"
done

# Rosetta: an x86_64 macOS shell on an Apple silicon Mac installs the native build; elsewhere sysctl is not consulted
# or says 0 or is unknown.
os=Darwin arch=x86_64 translated=1 install
installed_from "macOS x86_64 under Rosetta" reeve-macos-aarch64
os=Darwin arch=x86_64 translated=0 install
installed_from "macOS x86_64, not translated" reeve-macos-x86_64
os=Darwin arch=x86_64 translated="" install
installed_from "macOS x86_64, sysctl key unknown" reeve-macos-x86_64
os=Linux arch=x86_64 translated=1 install
installed_from "Linux with a stray sysctl answer" reeve-linux-x86_64

# Unsupported platforms fail before downloading, and say what is supported.
for plat in Linux/riscv64 FreeBSD/amd64 Darwin/ppc; do
	os=${plat%/*} arch=${plat#*/} install
	refused "$plat" 'Supported: Linux x86_64 and arm64, macOS arm64 and x86_64'
	[ ! -s "$work/log" ] || fail "$plat downloaded something"
done

# curl: -q first (no .curlrc), https only with TLS 1.2, no .netrc, never a header, cookie or user. The first line of
# the output names the build the server offers.
install
installed_from "the default run" reeve-linux-x86_64
while read -r line; do
	case "$line" in
	"curl -q --proto =https --tlsv1.2 "*) ;;
	*) fail "a curl call did not start with -q --proto =https --tlsv1.2: $line" ;;
	esac
done <"$work/log"
no_credentials "curl"
[ "$(head -n 1 "$work/out")" = "installed reeve build abcdef0 at $installed" ] || fail "the first line does not name the build"
grep -q 'next: reeve auth login --url https://reeve.test' "$work/out" || fail "no next step printed"

# shasum when there is no sha256sum.
if command -v shasum >/dev/null 2>&1; then
	bin=$work/bin-shasum install
	installed_from "the shasum fallback" reeve-linux-x86_64
elif [ -n "${CI:-}" ]; then
	fail "shasum is required in CI"
else
	echo "shasum not installed: its fallback case skipped"
fi

# A VERSION that is not a build (an older server's web page), a checksum mismatch, a missing line and a duplicated line
# install nothing.
printf '<!doctype html>\n' >"$root/VERSION"
install
refused "a VERSION that is not a build" 'is not a build'
printf '%s\n' "${build%?}" >"$root/VERSION"
install
refused "a 39-character VERSION" 'is not a build'
printf '%s\n' "$build" | tr a-f A-F >"$root/VERSION"
install
refused "an uppercase VERSION" 'is not a build'
printf '%s\n' "$build" >"$root/VERSION"
printf 'tampered' >>"$root/reeve-linux-x86_64"
install
refused "a tampered binary" 'checksum mismatch'
nothing_left "mismatch"
fresh "reeve-planner 0.6.0"
run
kept "a checksum mismatch over an existing reeve" "reeve-planner 0.6.0"
fixture reeve-linux-x86_64
grep -v ' reeve-linux-x86_64$' "$work/SHA256SUMS.good" >"$root/SHA256SUMS"
install
refused "a missing checksum line" 'has 0 lines for reeve-linux-x86_64'
{
	cat "$work/SHA256SUMS.good"
	grep ' reeve-linux-x86_64$' "$work/SHA256SUMS.good"
} >"$root/SHA256SUMS"
install
refused "a duplicated checksum line" 'has 2 lines for reeve-linux-x86_64'
cp "$work/SHA256SUMS.good" "$root/SHA256SUMS"

# A redirect is refused (the fake follows one when asked to, so -L would install the target and fail here), and a
# failed download installs nothing.
install FAKE_REDIRECT=reeve-linux-x86_64
refused "a redirect" 'HTTP 302'
fresh "reeve-planner 0.6.0"
run FAKE_REDIRECT=reeve-linux-x86_64
kept "a redirect over an existing reeve" "reeve-planner 0.6.0"
mv "$root/reeve-linux-x86_64" "$work/held"
install
refused "a 404" 'HTTP 404'
fresh "reeve-planner 0.6.0"
run
kept "a 404 over an existing reeve" "reeve-planner 0.6.0"
mv "$work/held" "$root/reeve-linux-x86_64"

# A foreign reeve is kept unless REEVE_FORCE=1, and the refusal prints the line to run; a planner CLI is replaced.
fresh "reeve 2.1 (runner)"
run
if [ "$status" = 0 ] || ! grep -qF "curl -q --proto '=https' --tlsv1.2 -fsS https://reeve.test/cli/install.sh | REEVE_FORCE=1 sh" "$work/out"; then
	fail "a foreign reeve was not refused with a runnable REEVE_FORCE=1 command"
fi
grep -q 'runner' "$installed" || fail "a foreign reeve was overwritten without REEVE_FORCE"
fresh "reeve 2.1 (runner)"
run REEVE_FORCE=1
installed_from "REEVE_FORCE=1 over a foreign reeve" reeve-linux-x86_64
for val in true 0 yes ""; do
	fresh "reeve 2.1 (runner)"
	run REEVE_FORCE="$val"
	kept "REEVE_FORCE='$val' over a foreign reeve" "reeve 2.1 (runner)"
done
fresh
mkdir -p "$work/home/.local/bin/reeve"
run
if [ "$status" = 0 ] || ! grep -q 'is a directory' "$work/out"; then fail "a directory at the target was not refused"; fi
fresh
mkdir -p "$work/home/.local/bin" "$work/home/elsewhere"
ln -s "$work/home/elsewhere" "$work/home/.local/bin/reeve"
run
if [ "$status" = 0 ] || ! grep -q 'is a directory' "$work/out"; then fail "a symlink to a directory at the target was not refused"; fi
fresh "reeve-planner 0.6.0"
run
installed_from "an older planner CLI" reeve-linux-x86_64

# REEVE_INSTALL_DIR, and the PATH block before the next steps when the directory is not on PATH: an export line for
# this shell, where to keep it for the user's shell, and the full path that works without it.
opt=$work/home/opt/bin
install REEVE_INSTALL_DIR="$opt" SHELL=/bin/sh
if [ "$status" != 0 ] || [ ! -x "$opt/reeve" ]; then fail "REEVE_INSTALL_DIR was not used"; fi
grep -qF "export PATH=\"$opt:\$PATH\"" "$work/out" || fail "no export line for a directory off PATH"
grep -qF "$opt/reeve auth login --url https://reeve.test" "$work/out" || fail "no full-path fallback"
export_at=$(grep -nF 'export PATH=' "$work/out" | cut -d: -f1)
next_at=$(grep -n '^next:' "$work/out" | cut -d: -f1)
[ "${export_at:-99}" -lt "${next_at:-0}" ] || fail "the PATH block does not come before the next steps"
grep -qF 'that line to ~/.profile' "$work/out" || fail "no startup file named for a plain shell"
install REEVE_INSTALL_DIR="$opt" SHELL=/bin/zsh
grep -qF 'that line to ~/.zshrc' "$work/out" || fail "no ~/.zshrc for zsh"
install REEVE_INSTALL_DIR="$opt" SHELL=/usr/bin/fish
grep -qF "fish_add_path \"$opt\"" "$work/out" || fail "no fish_add_path for fish"
install REEVE_INSTALL_DIR="$opt" SHELL=/bin/bash
grep -qF 'that line to ~/.bashrc' "$work/out" || fail "no ~/.bashrc for bash"
os=Darwin install REEVE_INSTALL_DIR="$opt" SHELL=/bin/bash
grep -qF 'that line to ~/.bash_profile' "$work/out" || fail "no ~/.bash_profile for bash on macOS"
install PATH="$bin:$work/home/.local/bin"
[ "$status" = 0 ] || fail "the on-PATH run exited $status"
if grep -q 'export PATH=' "$work/out"; then fail "a PATH block for a directory already on PATH"; fi

# The directory is kept as written, not resolved through symlinks: ~/.local a symlink, ~/.local/bin on PATH.
fresh
mkdir -p "$work/real-local/bin"
ln -s "$work/real-local" "$work/home/.local"
run PATH="$bin:$work/home/.local/bin"
rm -f "$work/home/.local"
[ "$status" = 0 ] || fail "the symlinked ~/.local run exited $status"
if grep -q 'not on your PATH' "$work/out"; then fail "a directory on PATH through a symlink was reported as off PATH"; fi
rm -rf "$work/real-local"

# An exported CDPATH must not leak into the directory change: a relative REEVE_INSTALL_DIR still lands where asked.
fresh
mkdir -p "$work/home/cwd/rel" "$work/home/cdp/rel"
status=0
(cd "$work/home/cwd" && env -i HOME="$work/home" PATH="$bin" TMPDIR="$work/tmp" FAKE_ROOT="$root" FAKE_LOG="$work/log" \
	FAKE_OS=Linux FAKE_ARCH=x86_64 FAKE_TRANSLATED= CDPATH="$work/home/cdp" REEVE_INSTALL_DIR=rel \
	"${testsh[@]}" "$work/install.sh") >"$work/out" 2>&1 || status=$?
if [ "$status" != 0 ] || [ ! -x "$work/home/cwd/rel/reeve" ] || [ -e "$work/home/cdp/rel/reeve" ]; then
	fail "a relative REEVE_INSTALL_DIR with CDPATH set went to the wrong place"
fi

# The mode is 755 whatever the umask (cp applies the umask; the chmod comes after it).
mask=077 install
mask=""
installed_from "a masked install" reeve-linux-x86_64
[ -n "$(find "$installed" -perm 755)" ] || fail "under umask 077 the installed reeve is not mode 755"

# An existing installation keeps working on any failure: a download that does not run, an unusable directory.
cp "$root/reeve-linux-x86_64" "$work/held"
printf 'not a program\n' >"$root/reeve-linux-x86_64"
sums >"$root/SHA256SUMS"
fresh "reeve-planner 0.6.0"
run
if [ "$status" = 0 ] || ! grep -q 'does not run on this machine' "$work/out"; then fail "a binary that does not run was installed"; fi
[ "$("$installed" --version)" = "reeve-planner 0.6.0" ] || fail "a failed install broke the existing reeve"
[ -z "$(find "$work/home/.local/bin" -name '.reeve.new.*')" ] || fail "a staged file was left behind"
cp "$work/held" "$root/reeve-linux-x86_64"
cp "$work/SHA256SUMS.good" "$root/SHA256SUMS"
fresh
: >"$work/home/file"
run REEVE_INSTALL_DIR="$work/home/file/bin"
if [ "$status" = 0 ] || ! grep -qF "https://reeve.test/cli/install.sh | REEVE_INSTALL_DIR=<directory> sh" "$work/out"; then
	fail "an unusable install directory was not refused with a runnable REEVE_INSTALL_DIR command"
fi

# A download cut short runs nothing: everything is in main, called on the last line.
size=$(wc -c <"$work/install.sh")
head -c $((size - 12)) "$work/install.sh" >"$work/install.cut"
cp "$work/install.cut" "$work/install.sh"
install
[ ! -s "$work/log" ] || fail "a truncated script downloaded something"
[ ! -e "$installed" ] || fail "a truncated script installed something"

# Plain http only to loopback, checked before anything is downloaded.
render http://reeve.test
install
refused "plain http off loopback" 'https only'
[ ! -s "$work/log" ] || fail "plain http off loopback downloaded something"
render http://127.0.0.1:7474
install
installed_from "plain http to loopback" reeve-linux-x86_64
grep -q -- '^curl -q --proto =http -sS' "$work/log" || fail "a loopback download did not pin --proto =http"
if grep -q -- '--tlsv1.2' "$work/log"; then fail "a plain http download asked for TLS"; fi
for url in http://localhost:7474 'http://[::1]:7474' http://localhost http://127.0.0.1; do
	render "$url"
	install
	installed_from "plain http to $url" reeve-linux-x86_64
done
for url in http://127.0.0.1:80@evil.test http://127.0.0.1:80@evil.test:443 http://127.0.0.1@evil.test http://localhost:80/x http://127.0.0.1:; do
	render "$url"
	install
	refused "plain http to $url" 'https only'
	[ ! -s "$work/log" ] || fail "$url downloaded something"
done

[ "$fails" -eq 0 ] || {
	echo "$fails failure(s)" >&2
	exit 1
}
echo "install.sh ok"
