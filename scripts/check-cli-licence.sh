#!/usr/bin/env bash
# The `reeve` CLI and `reeve-protocol` are MIT while the server side is AGPL-3.0-only (kb/ADR-0032), so neither may
# link an AGPL crate: this fails when a run-time or build dependency of either one, ours or third-party, carries an
# AGPL licence. Dev-dependencies are allowed (the CLI's tests start a real server), so `cargo tree` runs with
# `-e normal,build` only, for every target platform.
set -euo pipefail
bad=0
for pkg in reeve reeve-protocol; do
  if ! tree=$(cargo tree --locked -p "$pkg" -e normal,build --target all --prefix none --format '{p} {l}'); then
    echo "cargo tree failed for $pkg" >&2
    exit 1
  fi
  if hits=$(grep -i 'AGPL' <<<"$tree" | sort -u) && [ -n "$hits" ]; then
    echo "$pkg (MIT) depends on AGPL code at run or build time (kb/ADR-0032 R3):" >&2
    echo "$hits" >&2
    bad=1
  fi
done
exit "$bad"
