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
echo "context isolation ok"
