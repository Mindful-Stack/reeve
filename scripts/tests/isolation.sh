#!/usr/bin/env bash
# scripts/tests/isolation.sh — the lint must fail on every bypass shape and pass on facade imports.
set -euo pipefail
cd "$(dirname "$0")/../.."
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/crates/reeve-x/src" "$tmp/empty"
# Assert the lint's own exit status: 1 for a violation, 2 for a missing crates/ directory.
status() { local s=0; bash scripts/check-context-isolation.sh "$1" >/dev/null 2>&1 || s=$?; echo "$s"; }
must_fail() { printf '%s\n' "$1" > "$tmp/crates/reeve-x/src/case.rs"; local s; s=$(status "$tmp"); if [ "$s" -ne 1 ]; then echo "lint exited $s, wanted 1, for: $1" >&2; exit 1; fi; }
must_pass() { printf '%s\n' "$1" > "$tmp/crates/reeve-x/src/case.rs"; bash scripts/check-context-isolation.sh "$tmp" >/dev/null; }
must_fail 'use reeve_planning::repo::PgPlanningRepo;'
must_fail 'use reeve_planning::{repo::Pg, PlanningRepo};'
must_fail $'use reeve_planning::{\n    repo::PgPlanningRepo,\n};'
must_fail 'let r = reeve_planning::repo::PgPlanningRepo::new(pool);'
must_pass 'use reeve_planning::PlanningRepo;'
must_pass 'pub use repo::make_planning_repo;'
s=$(status "$tmp/empty"); if [ "$s" -ne 2 ]; then echo "lint exited $s, wanted 2, on a directory without crates/" >&2; exit 1; fi
# Cases at an arbitrary path under crates/, each in a fresh tree.
at() { rm -rf "$tmp/crates"; mkdir -p "$tmp/crates/$(dirname "$1")"; printf '%s\n' "$2" > "$tmp/crates/$1"; }
fail_at() { at "$1" "$2"; local s; s=$(status "$tmp"); if [ "$s" -ne 1 ]; then echo "lint exited $s, wanted 1, for $1: $2" >&2; exit 1; fi; }
pass_at() { at "$1" "$2"; bash scripts/check-context-isolation.sh "$tmp" >/dev/null; }
fail_at reeve-planning/src/domain/x.rs 'fn f() -> reeve_protocol::StoryDto { todo!() }'
fail_at reeve-planning/src/domain/x.rs 'use reeve_protocol::{Key, GoalDto};'
fail_at reeve-planning/src/domain/x.rs 'fn f(s: &[StoryWithReadiness]) {}'
fail_at reeve-planning/src/domain/x.rs 'type A = V2Dto;'
fail_at reeve-planning/src/domain/x.rs '// builds a GoalDto'
pass_at reeve-planning/src/domain/x.rs 'struct DtoLike;'
pass_at reeve-planning/src/handler/dto.rs 'impl From<&Story> for reeve_protocol::StoryDto {}'
pass_at reeve-planning/src/domain/x.rs 'use reeve_protocol::{Key, Reason};'
echo "isolation lint test ok"
