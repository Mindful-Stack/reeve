#!/usr/bin/env bash
# scripts/tests/cli-windows.sh <exe-a> <version-a> <exe-b> <version-b> — the Windows leg of cli.yml (kb/ADR-0030 R2,
# R5, R7), with two stamped builds the leg just made. install.ps1 under Windows PowerShell and PowerShell 7: an install
# that runs in the same session, an install over an existing reeve.exe ([IO.File]::Replace), one over a reeve.exe that
# is running, poisoned PowerShell defaults that must not reach a download, a checksum mismatch, a redirect, and an
# existing reeve.exe held open so the replace step fails; the user Path keeps its unexpanded %VAR% entries and its
# REG_EXPAND_SZ kind; then `reeve update` from A to B, and the %APPDATA% configuration path. It writes the runner's user
# Path (install.ps1 does) and puts it back on exit: run it only on a discarded CI runner. Git Bash.
set -euo pipefail
cd "$(dirname "$0")/../.."
die() { echo "FAIL: $*" >&2; exit 1; }
[ $# -eq 4 ] || die "usage: cli-windows.sh <exe-a> <version-a> <exe-b> <version-b>"
a=$1 va=$2 b=$3 vb=$4
for tool in python pwsh powershell.exe cygpath curl sha256sum; do command -v "$tool" >/dev/null || die "$tool not found"; done
export REEVE_UPDATE_NOTICE=0
file=reeve-windows-x86_64.exe
# Under the runner's temp directory: Git Bash's /tmp maps to a path with 8.3 short names.
work=$(mktemp -d "$(cygpath -u "${RUNNER_TEMP:?run on a GitHub runner}")/cli-windows.XXXXXX")
pid=
win() { cygpath -w "$1"; }
hash_of() { sha256sum "$1" | cut -d' ' -f1; }
[ "$(hash_of "$a")" != "$(hash_of "$b")" ] || die "builds A and B are the same bytes: the update would prove nothing"
# ps <script>: Windows PowerShell, for the registry checks.
ps() { powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "\$ErrorActionPreference = 'Stop'; $1"; }

# The user Path, saved raw with its kind (or "absent") and put back on exit.
ENVKEY="[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', \$true)"
RAW="[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames"
ps "\$k = $ENVKEY; if (\$null -eq \$k.GetValue('Path', \$null)) { 'absent' } else { [string] \$k.GetValueKind('Path'); [string] \$k.GetValue('Path', '', $RAW) }" |
	tr -d '\r' >"$work/path.orig"
restore_path() {
	local kind raw
	kind=$(sed -n 1p "$work/path.orig")
	if [ "$kind" = absent ]; then
		ps "\$k = $ENVKEY; \$k.DeleteValue('Path', \$false)"
	else
		raw=$(sed -n 2p "$work/path.orig")
		REEVE_ORIG_PATH=$raw ps "\$k = $ENVKEY; \$k.SetValue('Path', \$env:REEVE_ORIG_PATH, [Microsoft.Win32.RegistryValueKind]::$kind)"
	fi
}
cleanup() {
	[ -z "$pid" ] || kill "$pid" 2>/dev/null || true
	restore_path || echo "cli-windows: could not restore the user Path" >&2
	rm -rf "$work"
}
trap cleanup EXIT
# Seed an unexpanded entry, as Windows writes them: install.ps1 must keep it unexpanded and keep REG_EXPAND_SZ.
PROBE='%USERPROFILE%\reeve-path-probe'
REEVE_PROBE=$PROBE ps "\$k = $ENVKEY; \$raw = [string] \$k.GetValue('Path', '', $RAW); \$new = if (\$raw) { \$raw.TrimEnd(';') + ';' + \$env:REEVE_PROBE } else { \$env:REEVE_PROBE }; \$k.SetValue('Path', \$new, [Microsoft.Win32.RegistryValueKind]::ExpandString)"

# bundle <root> <exe> <version> <hash to list>: cli/ holding VERSION, the Windows binary and a SHA256SUMS naming every
# target (the others with dummy hashes).
bundle() {
	mkdir -p "$1/cli"
	cp "$2" "$1/cli/$file"
	printf '%s\n' "$3" >"$1/cli/VERSION"
	local t f
	for t in $(scripts/cli-build.sh --targets); do
		f=$(scripts/cli-build.sh --file "$t")
		if [ "$f" = "$file" ]; then printf '%s  %s\n' "$4" "$f"; else printf '%064d  %s\n' 0 "$f"; fi
	done >"$1/cli/SHA256SUMS"
}
serve() {
	[ -z "$pid" ] || { kill "$pid"; wait "$pid" 2>/dev/null || true; }
	rm -f "$work/port"
	: >"$work/log"
	python -I scripts/tests/cli-serve.py "$(win "$1")" "$(win "$work/port")" "$(win "$work/log")" 2>"$work/serve.err" &
	pid=$!
	local _
	for _ in $(seq 100); do [ -s "$work/port" ] && break; sleep 0.1; done
	[ -s "$work/port" ] || die "the stand-in did not start: $(cat "$work/serve.err")"
	url="http://127.0.0.1:$(cat "$work/port")"
	sed "s|@REEVE_URL@|$url|" deploy/cli/install.ps1 >"$1/cli/install.ps1"
}
# install_with <shell> <dir> [VAR=value...]: irm | iex in one session (POISON runs between the fetch and iex, so only
# install.ps1's own requests meet it), then the session's reeve. A failure prints install.ps1's own message.
POISON="\$null"
install_with() {
	local shell=$1 dir=$2
	shift 2
	env -u REEVE_FORCE "$@" REEVE_INSTALL_DIR="$(win "$dir")" "$shell" -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command \
		"\$ErrorActionPreference = 'Stop'; \$s = irm '$url/cli/install.ps1'; $POISON; try { iex \$s; (Get-Command reeve).Source; reeve --version } catch { [Console]::Error.WriteLine(\$_.Exception.Message); exit 1 }"
}
no_leftovers() { [ -z "$(find "$1" -maxdepth 1 -name '.reeve.new.*.exe' 2>/dev/null)" ] || die "a failed install left a staged file in $1"; }

bundle "$work/a" "$a" "$va" "$(hash_of "$a")"
bundle "$work/b" "$b" "$vb" "$(hash_of "$b")"
bundle "$work/bad" "$a" "$va" "$(printf '%064d' 1)"
for shell in powershell.exe pwsh; do
	# Installs, and runs in the same session.
	serve "$work/a"
	dir=$work/install-$shell
	out=$(install_with "$shell" "$dir" 2>&1) || die "$shell: the install failed: $out"
	[ "$(hash_of "$dir/reeve.exe")" = "$(hash_of "$a")" ] || die "$shell: the installed bytes are not the served build"
	grep -qiF "$(win "$dir")\\reeve.exe" <<<"$out" || die "$shell: the session's reeve is not the installed one: $out"
	grep -q '^reeve-planner ' <<<"$out" || die "$shell: the installed reeve did not run in the same session: $out"

	# Over an existing reeve.exe: the [IO.File]::Replace path puts B in place and leaves no backup or staged file.
	serve "$work/b"
	out=$(install_with "$shell" "$dir" 2>&1) || die "$shell: the install over an existing reeve.exe failed: $out"
	[ "$(hash_of "$dir/reeve.exe")" = "$(hash_of "$b")" ] || die "$shell: the install over an existing reeve.exe did not put B in place"
	grep -q '^reeve-planner ' <<<"$out" || die "$shell: the replaced reeve did not run in the same session: $out"
	no_leftovers "$dir"
	[ -z "$(find "$dir" -maxdepth 1 -name '.reeve.old.*.exe')" ] || die "$shell: a replace of an idle reeve.exe left its backup"

	# Over a reeve.exe that is running (`reeve auth login --no-browser` waits on its open stdin): A replaces B while B
	# runs, and B is still running when the install ends.
	serve "$work/a"
	code=0
	env -u REEVE_FORCE REEVE_INSTALL_DIR="$(win "$dir")" "$shell" -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "
		\$ErrorActionPreference = 'Stop'
		\$s = irm '$url/cli/install.ps1'
		\$psi = New-Object System.Diagnostics.ProcessStartInfo
		\$psi.FileName = '$(win "$dir/reeve.exe")'
		\$psi.Arguments = 'auth login --no-browser --url http://127.0.0.1:1'
		\$psi.UseShellExecute = \$false
		\$psi.RedirectStandardInput = \$true
		\$psi.RedirectStandardOutput = \$true
		\$psi.RedirectStandardError = \$true
		\$psi.EnvironmentVariables['REEVE_CONFIG_DIR'] = '$(win "$work/held-config-$shell")'
		\$psi.EnvironmentVariables.Remove('REEVE_TOKEN')
		\$p = [Diagnostics.Process]::Start(\$psi)
		try {
			Start-Sleep -Seconds 2
			if (\$p.HasExited) { throw \"the running reeve.exe exited before the install: \$(\$p.StandardError.ReadToEnd())\" }
			iex \$s
			if (\$p.HasExited) { throw 'the running reeve.exe exited during the install' }
		} catch {
			[Console]::Error.WriteLine(\$_.Exception.Message)
			exit 1
		} finally {
			if (-not \$p.HasExited) { \$p.Kill(); \$p.WaitForExit() }
		}" >"$work/running.out" 2>&1 || code=$?
	[ "$code" = 0 ] || die "$shell: the install over a running reeve.exe failed: $(cat "$work/running.out")"
	[ "$(hash_of "$dir/reeve.exe")" = "$(hash_of "$a")" ] || die "$shell: the install over a running reeve.exe did not put A in place"
	"$dir/reeve.exe" --version | grep -q '^reeve-planner ' || die "$shell: the reeve.exe that replaced a running one does not run"
	no_leftovers "$dir"

	# Poisoned defaults: a session default that adds an Authorization header to every cmdlet with -Headers must not
	# reach install.ps1's downloads (R5). The binary download must be in the log, so the pass is not vacuous.
	POISON="\$PSDefaultParameterValues['*:Headers'] = @{ Authorization = 'Bearer poisoned' }"
	serve "$work/a"
	install_with "$shell" "$work/poisoned-$shell" >/dev/null 2>&1 || die "$shell: the install failed with poisoned defaults"
	after=$(grep -v ' /cli/install\.ps1 ' "$work/log" || true)
	grep -q " /cli/$file " <<<"$after" || die "$shell: no binary download was logged: $after"
	if grep -q ' auth$' <<<"$after"; then die "$shell: install.ps1 sent credentials: $after"; fi
	POISON="\$null"

	# A checksum line that does not match: nothing installed, nothing staged left.
	serve "$work/bad"
	bad=$work/bad-$shell
	if install_with "$shell" "$bad" >/dev/null 2>&1; then die "$shell: installed despite a checksum mismatch"; fi
	[ ! -e "$bad/reeve.exe" ] || die "$shell: a checksum mismatch left reeve.exe"
	no_leftovers "$bad"

	# A redirect for the binary, to a path that would serve valid bytes: refused, nothing installed, target never
	# fetched. 5.1 and 7 may report it differently (a throw, or a 3xx that Get-CliFile rejects); only the outcome counts.
	root=$work/redirect-$shell
	bundle "$root" "$a" "$va" "$(hash_of "$a")"
	mkdir -p "$root/elsewhere"
	cp "$a" "$root/elsewhere/$file"
	serve "$root"
	printf '/cli/%s %s/elsewhere/%s\n' "$file" "$url" "$file" >"$root/redirects"
	redir=$work/redirect-dir-$shell
	if install_with "$shell" "$redir" >/dev/null 2>&1; then die "$shell: followed a redirect"; fi
	[ ! -e "$redir/reeve.exe" ] || die "$shell: a redirect left reeve.exe"
	grep -qF "GET /cli/$file " "$work/log" || die "$shell: the binary was never requested: $(cat "$work/log")"
	if grep -qF ' /elsewhere/' "$work/log"; then die "$shell: fetched the redirect's target"; fi

	# The replace step fails (the existing reeve.exe held open with no sharing): the install fails, and the existing
	# installation is unchanged and still runs once released (R5). REEVE_FORCE=1 because the held file cannot run for
	# the foreign-binary check either.
	serve "$work/b"
	held=$work/held-$shell
	mkdir -p "$held"
	cp "$a" "$held/reeve.exe"
	code=0
	env REEVE_INSTALL_DIR="$(win "$held")" REEVE_FORCE=1 "$shell" -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command \
		"\$s = irm '$url/cli/install.ps1'; \$h = [IO.File]::Open('$(win "$held/reeve.exe")', 'Open', 'Read', 'None'); \$ok = \$true; try { iex \$s } catch { \$ok = \$false } finally { \$h.Close() }; if (\$ok) { exit 0 } else { exit 1 }" \
		>"$work/held.out" 2>&1 || code=$?
	[ "$code" != 0 ] || die "$shell: the install succeeded although reeve.exe was held: $(cat "$work/held.out")"
	[ "$(hash_of "$held/reeve.exe")" = "$(hash_of "$a")" ] || die "$shell: a failed replace changed the existing reeve.exe"
	"$held/reeve.exe" --version | grep -q '^reeve-planner ' || die "$shell: the existing reeve.exe no longer runs"
	no_leftovers "$held"
done

# The user Path after every install above: the seeded %USERPROFILE% entry still unexpanded, the kind still
# REG_EXPAND_SZ, each install directory (installed into three times) listed once, and the broadcast variable gone.
REEVE_PROBE=$PROBE REEVE_DIRS="$(win "$work/install-powershell.exe");$(win "$work/install-pwsh")" ps "
	\$k = $ENVKEY
	\$raw = [string] \$k.GetValue('Path', '', $RAW)
	\$fail = @()
	if (\$k.GetValueKind('Path') -ne [Microsoft.Win32.RegistryValueKind]::ExpandString) { \$fail += \"the user Path is now \$(\$k.GetValueKind('Path')), not ExpandString\" }
	if (-not (\$raw -split ';' -contains \$env:REEVE_PROBE)) { \$fail += \"the unexpanded \$env:REEVE_PROBE entry is gone: \$raw\" }
	foreach (\$d in \$env:REEVE_DIRS -split ';') {
		\$n = @(\$raw -split ';' | Where-Object { \$_.TrimEnd('\\') -eq \$d.TrimEnd('\\') }).Count
		if (\$n -ne 1) { \$fail += \"\$d is listed \$n times in the user Path, want 1: \$raw\" }
	}
	if (\$null -ne \$k.GetValue('REEVE_PATH_BROADCAST', \$null)) { \$fail += 'the REEVE_PATH_BROADCAST user variable was left behind' }
	if (\$fail) { \$fail | ForEach-Object { [Console]::Error.WriteLine(\$_) }; exit 1 }" || die "install.ps1 damaged the user Path"

# reeve update from the installed build A to B, with a poisoned REEVE_TOKEN and a proxy that answers nothing: B's
# bytes, this target's file, no credentials, no proxy for a loopback download.
exe=$work/install-pwsh/reeve.exe
[ "$(hash_of "$exe")" = "$(hash_of "$a")" ] || die "the update's starting point is not build A"
serve "$work/b"
env -u NO_PROXY -u no_proxy REEVE_URL="$url" REEVE_TOKEN=poisoned-env REEVE_CONFIG_DIR="$(win "$work/update-config")" \
	HTTP_PROXY=http://127.0.0.1:9 http_proxy=http://127.0.0.1:9 ALL_PROXY=http://127.0.0.1:9 all_proxy=http://127.0.0.1:9 \
	"$exe" update >"$work/update.out" 2>&1 || die "reeve update failed: $(cat "$work/update.out")"
[ "$(hash_of "$exe")" = "$(hash_of "$b")" ] || die "reeve update did not install the build the server offers"
"$exe" --version | grep -q '^reeve-planner ' || die "the updated reeve.exe does not run"
grep -qF "GET /cli/$file " "$work/log" || die "reeve update did not ask for $file: $(cat "$work/log")"
if grep -v " /cli/$file " "$work/log" | grep -q ' /cli/reeve-'; then die "reeve update asked for another target's file: $(cat "$work/log")"; fi
if grep -q ' auth$' "$work/log"; then die "reeve update sent credentials: $(cat "$work/log")"; fi

# The configuration lives in %APPDATA%\reeve on Windows, whatever HOME and XDG_CONFIG_HOME say (or not).
appdata=$work/appdata
mkdir -p "$appdata"
env -u HOME -u XDG_CONFIG_HOME -u REEVE_CONFIG_DIR APPDATA="$(win "$appdata")" \
	"$exe" auth set-token --url https://reeve.example.com --token t >"$work/settoken.out" 2>&1 ||
	die "reeve auth set-token failed: $(cat "$work/settoken.out")"
grep -qF 'https://reeve.example.com' "$appdata/reeve/config.toml" 2>/dev/null ||
	die "reeve auth set-token did not write %APPDATA%\\reeve\\config.toml: $(cat "$work/settoken.out")"
echo "cli-windows ok"
