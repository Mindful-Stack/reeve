#!/usr/bin/env bash
# A crate may use another context crate only through its facade (`reeve_planning::Thing`), never a
# submodule path (`reeve_planning::repo::PgPlanningRepo`), in `use` lines, brace groups or inline paths.
#
# The planning rules below are a tripwire over source text, not a proof. They read each handler file with
# its string literals blanked and its `//` comments removed, so they catch the direct spellings of a forbidden
# call (a method call, UFCS, a method value, a ports alias) but not, among others:
#   - a call routed through another handler method, e.g. prs.rs calling `self.create_story` or any method that
#     itself reaches a forbidden port;
#   - macro-generated code, raw strings, char literals holding `"`, string literals spanning lines, and a `//`
#     inside a `/* */` block comment (it hides the rest of that line).
# The behaviour itself is pinned by tests: tests/handlers.rs
# recording_a_pull_request_links_updates_and_never_moves_the_story, tests/handlers.rs
# promises_change_no_readiness_delivery_progress_or_rank, tests/rules.rs (move_story's rules) and
# tests/sprints.rs moves_never_depend_on_goals.
# The domain DTO rule judges comments too, deliberately: planning's domain docs must not name a wire DTO either.
set -euo pipefail
cd "${1:-$(dirname "$0")/..}"
[ -d crates ] || { echo "no crates/ directory under $PWD" >&2; exit 2; }
crates='reeve_(protocol|planning|identity|activity|server)'
# Exit codes: 0 clean, 1 a violation, 2 the lint itself could not run (it never reports ok on a partial read).
g() { local s=0; grep "$@" || s=$?; [ "$s" -le 1 ] || exit 2; }   # swallow only "no match" (1); a grep error is 2
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
  # Judge each file with its string literals blanked (a "https://..." must not hide the rest of its line), then
  # without `//` comments and with its whitespace removed (rustfmt splits call chains over lines).
  flat() { sed -E 's/"([^"\\]|\\.)*"/""/g; s#//.*##' "$1" | tr -d '[:space:]'; }
  lb='(^|[^A-Za-z0-9_])'   # left boundary: a path or call, not the tail of a longer name
  bad=""
  moves="$planning/handler/moves.rs"
  p="$planning/handler/prs.rs"
  # The rules below name these two files; renaming either away must not switch its rules off silently.
  if [ -f "$planning/handler/mod.rs" ]; then
    for f in "$moves" "$p"; do
      if [ ! -f "$f" ]; then bad+="$f: missing (the call-site rules below judge it by name)"$'\n'; fi
    done
  fi
  # A command substitution (not `< <(...)`) so a failing find stops the lint instead of skipping the scan.
  files=$(find "$planning/handler" -name '*.rs' | sort) || { echo "cannot list $planning/handler" >&2; exit 2; }
  while IFS= read -r f; do
    [ -n "$f" ] || continue   # no .rs files: "<<<" still feeds one empty line
    # kb/ADR-0002 R4: only move_story (handler/moves.rs) moves a story, however the call is spelled
    # (a call, UFCS, or a method value bound to a name and called later).
    if [ "$f" != "$moves" ]; then
      x=$(flat "$f" | g -oE "${lb}set_delivery\b")
      if [ -n "$x" ]; then bad+="$f: set_delivery outside handler/moves.rs"$'\n'; fi
      # The delivery rule (Story::move_to) runs only inside move_story's transaction.
      x=$(flat "$f" | g -oE "${lb}move_to\b")
      if [ -n "$x" ]; then bad+="$f: move_to outside handler/moves.rs"$'\n'; fi
    fi
  done <<< "$files"
  # kb/ADR-0014 R4, kb/ADR-0017 R5, kb/ADR-0025 R7: moving a story never reads sprints, goals, urgency or the decision
  # queue's windows, and never reads or writes promises.
  if [ -f "$moves" ]; then
    x=$(flat "$moves" | g -oE "${lb}ports\.sprints|active_goals_of_stories|draft_goals_of_stories|decision_lineage_in|${lb}decision_queue(_in)?\(|${lb}decision_context\(|${lb}ports\.urgency|urgency_lineage_in|${lb}urgency_in\(|${lb}ports\.promises|promise_facts_in|promised_between_in|${lb}promise_clock_in\b|active_and_draft_in|${lb}(set|clear)_promise(_in)?\b")
    if [ -n "$x" ]; then bad+="$moves: reads sprints, goals, urgency or the decision queue, or reads or writes promises"$'\n'; fi
  fi
  # The webhook path reaches pull requests and story lookup only, and never moves a story: every mention of
  # `ports` in prs.rs must be exactly `ports.pull_requests.<method>` or `ports.stories.get_story`, so an alias
  # (`let st = &self.ports.stories;`) or UFCS (`StoryRepo::touch_story(&*self.ports.stories, ..)`) fails.
  if [ -f "$p" ]; then
    # `use` declarations (`use crate::ports::StoryRepo;`) name the module, not the field, so they are dropped first.
    x=$(flat <(sed -E '/^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?use[[:space:]]/d' "$p") \
      | g -oE "${lb}ports\b(\.[A-Za-z0-9_]+(\.[A-Za-z0-9_]+)?)?" | sed -E 's/^[^p]*//' \
      | g -vE '^ports\.pull_requests\.[A-Za-z0-9_]+$|^ports\.stories\.get_story$')
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
