#!/usr/bin/env bash
# A crate may use another context crate only through its facade (`reeve_planning::Thing`), never a
# submodule path (`reeve_planning::repo::PgPlanningRepo`), in `use` lines, brace groups or inline paths.
set -euo pipefail
cd "${1:-$(dirname "$0")/..}"
[ -d crates ] || { echo "no crates/ directory under $PWD" >&2; exit 2; }
crates='reeve_(protocol|planning|identity|activity|server)'
g() { grep "$@" || [ $? -eq 1 ]; }   # swallow only "no match" (1); a grep error (2) aborts under set -e
violations=$( { g -rnE "${crates}::[a-z_]+::" crates --include='*.rs'; } ; { g -rnEz "${crates}::\{[^}]*\b[a-z_]+::" crates --include='*.rs' -l; } )
if [ -n "$violations" ]; then
  echo "context isolation violated: import through the crate facade only" >&2
  echo "$violations" >&2
  exit 1
fi
planning=crates/reeve-planning/src
if [ -d "$planning/domain" ]; then
  dto=$(g -rnE '\b[A-Za-z0-9_]*Dto\b|\bStoryWithReadiness\b' "$planning/domain" --include='*.rs')
  if [ -n "$dto" ]; then
    echo "planning's domain names a wire DTO: build DTOs in handler/dto.rs" >&2
    echo "$dto" >&2
    exit 1
  fi
fi
echo "context isolation ok"
