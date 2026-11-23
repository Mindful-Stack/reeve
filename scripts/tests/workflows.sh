#!/usr/bin/env bash
# scripts/tests/workflows.sh — the release and CLI workflows' shell, and the CLI version's lock. The shell: release.yml
# and cli.yml set `defaults: run: shell: bash` at workflow level, which GitHub runs as `bash --noprofile --norc -eo
# pipefail {0}` (the implicit default is `bash -e {0}`, without pipefail, where a failure before a pipe is lost), and no
# job or step overrides it.
set -euo pipefail
cd "$(dirname "$0")/../.."
die() { echo "FAIL: $*" >&2; exit 1; }
if command -v shellcheck >/dev/null 2>&1; then shellcheck scripts/tests/workflows.sh; elif [ -n "${CI:-}" ]; then die "shellcheck required in CI"; fi

# The default, at workflow level, and no other shell: anywhere in the file.
has_default() { # the three lines, top level, consecutive
	awk 'prev2 == "defaults:" && prev1 == "  run:" && $0 == "    shell: bash" { found = 1 } { prev2 = prev1; prev1 = $0 } END { exit !found }' "$1"
}
for f in .github/workflows/release.yml .github/workflows/cli.yml; do
	if grep -q '| *tee' "$f" && ! has_default "$f"; then die "$f pipes into tee without defaults: run: shell: bash (the pipe's status is lost)"; fi
	has_default "$f" || die "$f lacks the workflow-level defaults: run: shell: bash"
	others=$(grep -n '^ *shell:' "$f" | grep -v '^[0-9]*:    shell: bash$' || true)
	[ -z "$others" ] || die "$f overrides the shell: $others"
done

# cli.yml runs on every pull request that can change the CLI: each input scripts/cli-version.sh names (a directory, or
# a file) is matched by one of its pull_request.paths patterns (bash globbing: `*` and `**` both cross `/`, which is
# looser than GitHub's `*` only for patterns that contain `/` after it; none here does).
mapfile -t inputs < <(awk '/^inputs=\($/ { on = 1; next } on && /^\)$/ { exit } on { gsub(/^[ \t]+|[ \t]+$/, ""); print }' scripts/cli-version.sh)
[ "${#inputs[@]}" -gt 0 ] || die "found no inputs=( … ) list in scripts/cli-version.sh"
mapfile -t paths < <(awk '
	/^  pull_request:$/ { pr = 1; next }
	pr && /^    paths:$/ { on = 1; next }
	on && /^      - / { sub(/^      - /, ""); gsub(/"/, ""); print; next }
	on || (pr && /^  [^ ]/) { exit }' .github/workflows/cli.yml)
[ "${#paths[@]}" -gt 0 ] || die "found no pull_request.paths in .github/workflows/cli.yml"
for i in "${inputs[@]}"; do
	hit=
	for p in "${paths[@]}"; do
		# shellcheck disable=SC2053 # $p is the glob
		if [[ "$i" == $p || "$i/x" == $p ]]; then hit=1; break; fi
	done
	[ -n "$hit" ] || die "the CLI input $i is not in .github/workflows/cli.yml's pull_request.paths: a change to it would run no CLI build before main"
done
# One build-and-publish run per CLI version at a time (kb/ADR-0030 R1): release.yml's job that calls cli.yml holds a
# concurrency group named for the CLI version and never cancels the run holding it, and it hands cli.yml that version;
# cli.yml takes no job-level group of its own, which would wait on the caller's group forever.
job_block() { # <file> <job>: the job's lines, up to the next job
	awk -v job="  $2:" '$0 == job { on = 1; print; next } on && /^  [^ #]/ { exit } on { print }' "$1"
}
cli_job=$(job_block .github/workflows/release.yml cli)
[ -n "$cli_job" ] || die "release.yml has no cli job"
grep -qx '    uses: ./.github/workflows/cli.yml' <<<"$cli_job" || die "release.yml's cli job does not call cli.yml"
grep -qx '    needs: cli-version' <<<"$cli_job" || die "release.yml's cli job does not wait for cli-version"
grep -qx '    concurrency:' <<<"$cli_job" || die "release.yml's cli job holds no lock around cli.yml"
# shellcheck disable=SC2016 # the expression is GitHub's, matched literally
grep -qxF '      group: reeve-cli-${{ needs.cli-version.outputs.version }}' <<<"$cli_job" ||
	die "release.yml's cli lock is not named for the CLI version"
grep -qx '      cancel-in-progress: false' <<<"$cli_job" || die "release.yml's cli lock may cancel a run in progress"
# shellcheck disable=SC2016 # the expression is GitHub's, matched literally
grep -qxF '      locked-version: ${{ needs.cli-version.outputs.version }}' <<<"$cli_job" ||
	die "release.yml's cli job does not hand cli.yml the version it locked"
# shellcheck disable=SC2016 # the shell text is cli.yml's, matched literally
grep -qF '[ "$EVENT" != push ] || [ -n "$LOCKED" ] ||' .github/workflows/cli.yml ||
	die "cli.yml does not refuse a push run without locked-version (it would build and publish unlocked)"
if grep -q '^    concurrency:' .github/workflows/cli.yml; then
	die "cli.yml has a job-level concurrency group: inside release.yml's lock, one of the same name never starts"
fi
echo "workflows test ok"
