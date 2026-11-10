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
if [ -d "$planning/handler" ]; then
  # Judge each file without `//` comments and with its whitespace removed (rustfmt splits call chains over lines).
  flat() { sed 's#//.*##' "$1" | tr -d '[:space:]'; }
  lb='(^|[^A-Za-z0-9_])'   # left boundary: a path or call, not the tail of a longer name
  bad=""
  moves="$planning/handler/moves.rs"
  while IFS= read -r f; do
    # kb/ADR-0002 R4: only move_story (handler/moves.rs) moves a story, however the call is spelled.
    if [ "$f" != "$moves" ]; then
      x=$(flat "$f" | g -oE "${lb}set_delivery\(")
      if [ -n "$x" ]; then bad+="$f: set_delivery outside handler/moves.rs"$'\n'; fi
    fi
  done < <(find "$planning/handler" -name '*.rs' | sort)
  # kb/ADR-0014 R4, kb/ADR-0017 R5: moving a story never reads sprints, goals or urgency.
  if [ -f "$moves" ]; then
    x=$(flat "$moves" | g -oE "${lb}ports\.sprints|active_goals_of_stories|${lb}ports\.urgency|urgency_lineage_in|${lb}urgency_in\(")
    if [ -n "$x" ]; then bad+="$moves: reads sprints, goals or urgency"$'\n'; fi
  fi
  p="$planning/handler/prs.rs"
  # The webhook path reaches pull requests and story lookup (`get_story`) only, and never moves a story.
  if [ -f "$p" ]; then
    ports=$(flat "$p" | g -oE "${lb}ports\.[a-z_]+(\.[a-z_]+)?" | sed -E 's/^[^p]*//')
    x=$(printf '%s\n' "$ports" | g -vE '^$|^ports\.pull_requests(\.[a-z_]+)?$|^ports\.stories(\.get_story)?$')
    while IFS= read -r l; do
      if [ -n "$l" ]; then bad+="$p: uses $l"$'\n'; fi
    done <<< "$x"
    x=$(printf '%s\n' "$ports" | g -E '^ports\.stories\.' | g -vE '^ports\.stories\.get_story$')
    while IFS= read -r l; do
      if [ -n "$l" ]; then bad+="$p: uses $l"$'\n'; fi
    done <<< "$x"
    x=$(flat "$p" | g -oE "${lb}move_story")
    if [ -n "$x" ]; then bad+="$p: calls move_story"$'\n'; fi
  fi
  if [ -n "$bad" ]; then
    echo "planning call-site rule violated" >&2
    printf '%s' "$bad" >&2
    exit 1
  fi
fi
echo "context isolation ok"
