#!/usr/bin/env bash
# scripts/ci-needs-rust.sh — reads a pull request's changed paths, one per line, and prints `true` when ci.yml's Rust
# jobs (lint, test) must run, `false` when they may skip. It lists what the Rust jobs cannot see rather than what
# they need, so a path nobody has classified runs them, and so does an empty list: a failed listing never skips them.
#
# Cannot see: web/** except the two files the Rust tests read (crates/reeve/src/tui/words.rs includes
# web/src/lib/board.ts, crates/reeve/tests/glyphs_snapshot.rs reads web/src/lib/glyphs.json), docs/**, lore/** and
# top-level markdown. The web client is still checked by ci.yml's web and smoke jobs, which always run.
set -euo pipefail
rust=false
seen=0
while IFS= read -r path; do
	[ -n "$path" ] || continue
	seen=1
	case "$path" in
	web/src/lib/board.ts | web/src/lib/glyphs.json) rust=true ;;
	web/* | docs/* | lore/*) ;;
	*/*) rust=true ;;
	*.md) ;;
	*) rust=true ;;
	esac
done
[ "$seen" -eq 1 ] || rust=true
echo "$rust"
