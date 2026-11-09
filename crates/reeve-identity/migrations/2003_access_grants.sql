-- Sign-in access beyond the configured floor (kb/ADR-0015 R2). Owned by identity; no foreign key leaves this context
-- (kb/ADR-0010 R3) and no tenant column (kb/ADR-0016 R1). Values are stored normalised
-- (no whitespace, ASCII-lowercased, non-empty) by the handlers; the CHECKs enforce it so a hand edit fails loudly instead of
-- leaving a row that can be loaded but never deleted.
CREATE TABLE allowed_entries (
  kind     TEXT NOT NULL CHECK (kind IN ('domain','email')),
  value    TEXT NOT NULL CHECK (value <> '' AND value !~ '\s' AND value = translate(value, 'ABCDEFGHIJKLMNOPQRSTUVWXYZ', 'abcdefghijklmnopqrstuvwxyz')),
  added_by TEXT NOT NULL,
  added_at TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (kind, value)
);
CREATE TABLE admin_grants (
  email    TEXT PRIMARY KEY CHECK (email <> '' AND email !~ '\s' AND email = translate(email, 'ABCDEFGHIJKLMNOPQRSTUVWXYZ', 'abcdefghijklmnopqrstuvwxyz')),
  added_by TEXT NOT NULL,
  added_at TIMESTAMPTZ NOT NULL
);
