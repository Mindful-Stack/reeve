-- Brief templates: a named Markdown text per level (initiative or epic) from which a new brief can be prefilled.
-- Prefill only: the table names no item and no item names a row, so there is no foreign key in either direction.
-- name_key is the name trimmed and lowercased by the server (never by lower() here), and makes a name unique per
-- level. At most one row per level is the default: the partial unique index holds it. version is what an edit
-- names; the default mark changes none. No column says who wrote a row or when: the activity entries do, and no
-- column holds an email.
CREATE TABLE brief_templates (
  id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  level      TEXT NOT NULL CHECK (level IN ('initiative', 'epic')),
  name       TEXT NOT NULL,
  name_key   TEXT NOT NULL,
  body       TEXT NOT NULL,
  is_default BOOLEAN NOT NULL DEFAULT false,
  version    BIGINT NOT NULL DEFAULT 1,
  UNIQUE (level, name_key)
);
CREATE UNIQUE INDEX brief_templates_default_idx ON brief_templates (level) WHERE is_default;
