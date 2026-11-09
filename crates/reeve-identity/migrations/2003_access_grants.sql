-- Sign-in access beyond the configured floor (kb/ADR-0015 R2). Owned by identity; no foreign key leaves this context
-- (kb/ADR-0010 R3) and no tenant column (kb/ADR-0016 R1). Values are stored normalised by the handlers.
CREATE TABLE allowed_entries (
  kind     TEXT NOT NULL CHECK (kind IN ('domain','email')),
  value    TEXT NOT NULL,
  added_by TEXT NOT NULL,
  added_at TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (kind, value)
);
CREATE TABLE admin_grants (
  email    TEXT PRIMARY KEY,
  added_by TEXT NOT NULL,
  added_at TIMESTAMPTZ NOT NULL
);
