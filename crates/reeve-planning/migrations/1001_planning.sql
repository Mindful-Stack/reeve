-- crates/reeve-planning/migrations/1001_planning.sql
CREATE TABLE key_sequence (prefix TEXT PRIMARY KEY, last BIGINT NOT NULL DEFAULT 0);

CREATE TABLE initiatives (
  key TEXT PRIMARY KEY, title TEXT NOT NULL, brief TEXT NOT NULL DEFAULT '',
  horizon TEXT NOT NULL, target_month TEXT, owner TEXT,
  discovery_at TIMESTAMPTZ, discovery_by TEXT, external_ref TEXT UNIQUE,
  version BIGINT NOT NULL DEFAULT 1, idempotency_key TEXT UNIQUE,
  created_at TIMESTAMPTZ NOT NULL, updated_at TIMESTAMPTZ NOT NULL
);
CREATE TABLE epics (
  key TEXT PRIMARY KEY, title TEXT NOT NULL,
  initiative_key TEXT REFERENCES initiatives(key), brief TEXT NOT NULL DEFAULT '',
  discovery_at TIMESTAMPTZ, discovery_by TEXT, external_ref TEXT UNIQUE,
  version BIGINT NOT NULL DEFAULT 1, idempotency_key TEXT UNIQUE,
  created_at TIMESTAMPTZ NOT NULL, updated_at TIMESTAMPTZ NOT NULL
);
CREATE TABLE stories (
  key TEXT PRIMARY KEY, title TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('story','bug','chore')),
  epic_key TEXT REFERENCES epics(key), description TEXT NOT NULL DEFAULT '',
  criteria JSONB NOT NULL DEFAULT '[]'::jsonb, owner TEXT, labels TEXT[] NOT NULL DEFAULT '{}',
  rank INTEGER NOT NULL DEFAULT 0,
  delivery TEXT NOT NULL DEFAULT 'todo' CHECK (delivery IN ('todo','in_progress','in_review','testing','done')),
  resolution TEXT CHECK (resolution IN ('completed','dropped')),
  external_ref TEXT UNIQUE,
  version BIGINT NOT NULL DEFAULT 1, idempotency_key TEXT UNIQUE,
  created_at TIMESTAMPTZ NOT NULL, updated_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX stories_epic_idx ON stories (epic_key);
CREATE INDEX stories_delivery_idx ON stories (delivery, rank);
CREATE TABLE story_blocks (story_key TEXT NOT NULL REFERENCES stories(key), blocker_key TEXT NOT NULL REFERENCES stories(key), PRIMARY KEY (story_key, blocker_key), CHECK (story_key <> blocker_key));

CREATE TABLE required_decisions (
  id BIGSERIAL PRIMARY KEY,
  on_type TEXT NOT NULL CHECK (on_type IN ('initiative','epic','story')), on_key TEXT NOT NULL,
  title TEXT NOT NULL, decider TEXT NOT NULL, due DATE,
  record_repo TEXT, record_path TEXT, record_id TEXT, pr_url TEXT,
  check_result JSONB NOT NULL DEFAULT '{"kind":"unchecked"}'::jsonb, checked_at TIMESTAMPTZ,
  closed_at TIMESTAMPTZ, closed_by TEXT, closed_reason TEXT,
  version BIGINT NOT NULL DEFAULT 1,
  idempotency_key TEXT UNIQUE, created_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX required_decisions_on_idx ON required_decisions (on_key);

CREATE TABLE comments (
  id BIGSERIAL PRIMARY KEY,
  story_key TEXT REFERENCES stories(key), decision_id BIGINT REFERENCES required_decisions(id),
  author TEXT NOT NULL, body TEXT NOT NULL, at TIMESTAMPTZ NOT NULL, external_ref TEXT UNIQUE,
  CHECK ((story_key IS NULL) <> (decision_id IS NULL))
);
