-- kb/ADR-0014: sprints hold goals; goals link to any item by key; stories carry no sprint field.
-- A sprint's window is [opened_at, closed_at); starts_on/ends_on are for display and promise shortcuts only.
CREATE TABLE sprints (
  id BIGSERIAL PRIMARY KEY,
  state TEXT NOT NULL CHECK (state IN ('draft', 'active', 'closed')),
  starts_on DATE NOT NULL,
  ends_on DATE NOT NULL CHECK (ends_on > starts_on),
  opened_at TIMESTAMPTZ,
  closed_at TIMESTAMPTZ,
  version BIGINT NOT NULL DEFAULT 1,
  created_at TIMESTAMPTZ NOT NULL,
  CHECK ((state = 'draft') = (opened_at IS NULL)),
  CHECK ((state = 'closed') = (closed_at IS NOT NULL)),
  CHECK (closed_at IS NULL OR closed_at >= opened_at)
);
CREATE UNIQUE INDEX sprints_one_draft_idx ON sprints ((true)) WHERE state = 'draft';
CREATE UNIQUE INDEX sprints_one_active_idx ON sprints ((true)) WHERE state = 'active';

CREATE TABLE goals (
  id BIGSERIAL PRIMARY KEY,
  sprint_id BIGINT NOT NULL REFERENCES sprints (id),
  outcome TEXT NOT NULL CHECK (btrim(outcome) <> ''),
  check_text TEXT NOT NULL CHECK (btrim(check_text) <> ''),
  owner TEXT NOT NULL CHECK (btrim(owner) <> ''),
  notes TEXT NOT NULL DEFAULT '',
  confidence TEXT CHECK (confidence IN ('on_track', 'at_risk', 'off_track')),
  confidence_note TEXT,
  confidence_at TIMESTAMPTZ,
  -- A draft confidence from an agent or automation (kb/ADR-0014 R5); recording a confidence clears it.
  suggested_confidence TEXT CHECK (suggested_confidence IN ('on_track', 'at_risk', 'off_track')),
  suggested_note TEXT,
  suggested_at TIMESTAMPTZ,
  suggested_by TEXT,
  verdict TEXT CHECK (verdict IN ('met', 'partly_met', 'missed')),
  verdict_note TEXT,
  version BIGINT NOT NULL DEFAULT 1,
  idempotency_key TEXT NOT NULL UNIQUE,
  created_at TIMESTAMPTZ NOT NULL,
  CHECK ((confidence IS NULL) = (confidence_at IS NULL)),
  CHECK ((suggested_confidence IS NULL) = (suggested_at IS NULL) AND (suggested_confidence IS NULL) = (suggested_by IS NULL)),
  CHECK (verdict_note IS NULL OR verdict IS NOT NULL)
);
CREATE INDEX goals_sprint_idx ON goals (sprint_id);

-- item_key names an initiative, epic or story (one key sequence, kb/ADR-0002 R5), as required_decisions.on_key does.
CREATE TABLE goal_links (
  goal_id BIGINT NOT NULL REFERENCES goals (id) ON DELETE CASCADE,
  item_key TEXT NOT NULL,
  linked_at TIMESTAMPTZ NOT NULL,
  linked_by TEXT NOT NULL,
  PRIMARY KEY (goal_id, item_key)
);
CREATE INDEX goal_links_item_idx ON goal_links (item_key);

-- When a story last entered an active state from todo or done (needed for "in flight at close", kb/ADR-0014 R2);
-- NULL for stories that never did since this column.
ALTER TABLE stories ADD COLUMN started_at TIMESTAMPTZ;
CREATE INDEX stories_done_at_idx ON stories (done_at);
