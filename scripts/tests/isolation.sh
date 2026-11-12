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
must_fail 'use reeve_planning::{repo::Pg, StoryRepo};'
must_fail $'use reeve_planning::{\n    repo::PgPlanningRepo,\n};'
must_fail 'let r = reeve_planning::repo::PgPlanningRepo::new(pool);'
must_pass 'use reeve_planning::StoryRepo;'
must_pass 'pub use repo::make_planning_ports;'
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
fail_at reeve-planning/src/handler/prs.rs 'self.ports.stories.set_delivery(tx, s, 1, now)'
fail_at reeve-planning/src/handler/stories.rs 'self.ports.stories.set_delivery(tx, s, 1, now)'
fail_at reeve-planning/src/handler/moves.rs 'self.ports.sprints.sprint_in_state(tx, st)'
fail_at reeve-planning/src/handler/moves.rs 'self.ports.reads.active_goals_of_stories(tx, ks)'
fail_at reeve-planning/src/handler/moves.rs 'self.ports.urgency.urgent_marks_in(tx)'
fail_at reeve-planning/src/handler/moves.rs 'self.ports.reads.urgency_lineage_in(tx, es, is)'
fail_at reeve-planning/src/handler/moves.rs 'self.ports.reads.draft_goals_of_stories(tx, ks)'
fail_at reeve-planning/src/handler/moves.rs 'self.ports.reads.decision_lineage_in(tx, ks)'
fail_at reeve-planning/src/handler/moves.rs 'self.decision_queue_in(tx, &s, now)'
fail_at reeve-planning/src/handler/moves.rs 'self.decision_queue(&s, now)'
fail_at reeve-planning/src/handler/moves.rs $'self\n    .decision_context(id, now)'
fail_at reeve-planning/src/handler/moves.rs $'self\n    .urgency_in(&mut tx, c.now)'
fail_at reeve-planning/src/handler/moves.rs 'self.ports.promises.lock_for_promise(tx, item, k)'
fail_at reeve-planning/src/handler/moves.rs $'self\n    .ports\n    .promises\n    .lock_for_promise(tx, item, k)'
fail_at reeve-planning/src/handler/moves.rs 'self.ports.reads.promise_facts_in(tx, ks, es)'
fail_at reeve-planning/src/handler/moves.rs 'self.promise_clock_in(&mut tx, c.now)'
fail_at reeve-planning/src/handler/moves.rs 'let f = Self::promise_clock_in;'
fail_at reeve-planning/src/handler/moves.rs 'self.ports.sprints.active_and_draft_in(tx)'
fail_at reeve-planning/src/handler/moves.rs 'let f = Self::active_and_draft_in;'
fail_at reeve-planning/src/handler/moves.rs 'self.ports.reads.promised_between_in(tx, after, upto)'
fail_at reeve-planning/src/handler/moves.rs 'self.set_promise_in(tx, c, item, k, req)'
fail_at reeve-planning/src/handler/moves.rs 'let f = Self::set_promise_in;'
fail_at reeve-planning/src/handler/moves.rs 'let f = Self::clear_promise_in;'
fail_at reeve-planning/src/handler/moves.rs 'let f = Self::set_promise;'
fail_at reeve-planning/src/handler/moves.rs 'Self::clear_promise_in(self, tx, c, item, k)'
pass_at reeve-planning/src/handler/moves.rs 'self.reset_promise_index(tx)'
pass_at reeve-planning/src/handler/moves.rs 'self.compromise_clock_in(tx)'
fail_at reeve-planning/src/handler/prs.rs 'self.ports.decisions.get_decision(1)'
fail_at reeve-planning/src/handler/moves.rs $'self\n    .ports\n    .sprints\n    .sprint_in_state(tx, st)'
fail_at reeve-planning/src/handler/prs.rs $'self\n    .ports\n    .decisions\n    .get_decision(1)'
fail_at reeve-planning/src/handler/stories.rs $'self.ports.stories\n    .set_delivery(tx, s, 1, now)'
pass_at reeve-planning/src/handler/moves.rs 'self.ports.stories.set_delivery(tx, s, 1, now)'
fail_at reeve-planning/src/handler/stories.rs 'story.move_to(m, true, t, a)'
fail_at reeve-planning/src/handler/prs.rs $'story\n    .move_to(m, true, t, a)'
fail_at reeve-planning/src/handler/stories.rs 'Story::move_to(&mut s, m, true, t, a)'
pass_at reeve-planning/src/handler/moves.rs 'story.move_to(m, true, t, a)'
fail_at reeve-planning/src/handler/goals.rs 'self.ports.sprints.move_goals(tx, from, to, ids)'
fail_at reeve-planning/src/handler/stories.rs $'self.ports.sprints\n    .move_goals(tx, from, to, ids)'
fail_at reeve-planning/src/handler/sub/x.rs 'let f = Self::move_goals;'
fail_at reeve-planning/src/handler/goals.rs 'SprintRepo::move_goals(&*self.ports.sprints, tx, from, to, ids)'
pass_at reeve-planning/src/handler/sprints.rs 'self.ports.sprints.move_goals(tx, from, to, ids)'
pass_at reeve-planning/src/handler/goals.rs 'self.remove_goals_x(tx)'
pass_at reeve-planning/src/handler/goals.rs '// only open_sprint_leaving calls move_goals'
pass_at reeve-planning/src/handler/stories.rs '// only move_story calls story.move_to(m, true, t, a)'
pass_at reeve-planning/src/handler/stories.rs 'self.remove_to(x)'
pass_at reeve-planning/src/handler/prs.rs $'self.ports.pull_requests.upsert_pull_request(tx, p)\nself.ports.stories.get_story(k)'
fail_at reeve-planning/src/handler/prs.rs 'self.ports.stories.touch_story(tx, k, 1, now)'
fail_at reeve-planning/src/handler/prs.rs $'self.ports\n    .stories\n    .update_story(tx, s, None, 1, now)'
fail_at reeve-planning/src/handler/prs.rs 'self.move_story(c, k, 1, to, None, None)'
fail_at reeve-planning/src/handler/stories.rs 'StoryRepo::set_delivery(&*self.ports.stories, tx, s, 1, now)'
fail_at reeve-planning/src/handler/sub/x.rs 'self.ports.stories.set_delivery(tx, s, 1, now)'
pass_at reeve-planning/src/handler/stories.rs '/// `self.ports.stories.set_delivery(` is the only mover'
pass_at reeve-planning/src/handler/prs.rs '// never self.ports.decisions here'
pass_at reeve-planning/src/handler/prs.rs 'self.ports.stories.get_story(k)'
pass_at reeve-planning/src/handler/prs.rs $'use crate::ports::{PullRequestRepo, StoryRepo};\nself.ports.stories.get_story(k)'
# A string literal holding `//` must not hide the rest of its line from the call-site rules.
fail_at reeve-planning/src/handler/stories.rs 'let u = "https://x"; self.ports.stories.set_delivery(tx, s, 1, now)'
pass_at reeve-planning/src/handler/stories.rs 'let u = "a \\"quoted\\" self.ports.stories.set_delivery(tx) str";'
# prs.rs reaches the ports only as `ports.pull_requests.<method>` or `ports.stories.get_story`: no alias, no UFCS.
fail_at reeve-planning/src/handler/prs.rs $'let st = &self.ports.stories;\nst.update_story(tx, s, None, 1, now)'
fail_at reeve-planning/src/handler/prs.rs $'let p = &self.ports;\np.decisions.get_decision(1)'
fail_at reeve-planning/src/handler/prs.rs 'StoryRepo::touch_story(&*self.ports.stories, tx, k, 1, now)'
fail_at reeve-planning/src/handler/prs.rs 'let r = &self.ports.pull_requests;'
# A method value is a call waiting to happen: naming set_delivery or move_to outside moves.rs is enough.
fail_at reeve-planning/src/handler/stories.rs $'let f = StoryRepo::set_delivery;\nf(&*self.ports.stories, tx, s, 1, now)'
fail_at reeve-planning/src/handler/stories.rs $'let f = Story::move_to;\nf(&mut s, m, true, t, a)'
pass_at reeve-planning/src/handler/stories.rs 'self.set_delivery_x(t)'
# With a handler/mod.rs, renaming moves.rs or prs.rs away must not silently switch their rules off.
rm -rf "$tmp/crates"; mkdir -p "$tmp/crates/reeve-planning/src/handler"
printf '%s\n' 'mod prs;' > "$tmp/crates/reeve-planning/src/handler/mod.rs"; printf '%s\n' '' > "$tmp/crates/reeve-planning/src/handler/prs.rs"
s=$(status "$tmp"); if [ "$s" -ne 1 ]; then echo "lint exited $s, wanted 1, for a handler/mod.rs without moves.rs" >&2; exit 1; fi
rm "$tmp/crates/reeve-planning/src/handler/prs.rs"; printf '%s\n' '' > "$tmp/crates/reeve-planning/src/handler/moves.rs"
s=$(status "$tmp"); if [ "$s" -ne 1 ]; then echo "lint exited $s, wanted 1, for a handler/mod.rs without prs.rs" >&2; exit 1; fi
printf '%s\n' '' > "$tmp/crates/reeve-planning/src/handler/prs.rs"; bash scripts/check-context-isolation.sh "$tmp" >/dev/null
# A handler file the lint cannot list must stop it (exit 2), not pass it unread. Root reads any directory, so skip there.
if [ "$(id -u)" -ne 0 ]; then
  at reeve-planning/src/handler/sub/x.rs 'self.ports.stories.set_delivery(tx, s, 1, now)'
  chmod 000 "$tmp/crates/reeve-planning/src/handler/sub"
  s=$(status "$tmp"); chmod 755 "$tmp/crates/reeve-planning/src/handler/sub"
  if [ "$s" -ne 2 ]; then echo "lint exited $s, wanted 2, for an unreadable handler directory" >&2; exit 1; fi
fi
# A handler directory with no .rs files is clean.
pass_at reeve-planning/src/handler/README.md 'notes'
# A failing `find` must stop the lint (exit 2), not skip the handler scan and report ok.
at reeve-planning/src/handler/stories.rs 'self.ports.stories.set_delivery(tx, s, 1, now)'
mkdir -p "$tmp/shim"; printf '#!/bin/sh\nexit 1\n' > "$tmp/shim/find"; chmod +x "$tmp/shim/find"
s=0; PATH="$tmp/shim:$PATH" bash scripts/check-context-isolation.sh "$tmp" >/dev/null 2>&1 || s=$?
if [ "$s" -ne 2 ]; then echo "lint exited $s, wanted 2, when find fails" >&2; exit 1; fi
echo "isolation lint test ok"
