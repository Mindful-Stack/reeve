CREATE TABLE users (
  email        TEXT PRIMARY KEY,
  display_name TEXT NOT NULL DEFAULT '',
  disabled     BOOLEAN NOT NULL DEFAULT FALSE,
  created_at   TIMESTAMPTZ NOT NULL
);
CREATE TABLE tokens (
  id          BIGSERIAL PRIMARY KEY,
  owner       TEXT NOT NULL REFERENCES users(email),
  kind        TEXT NOT NULL CHECK (kind IN ('interactive','automation')),
  hash        TEXT NOT NULL UNIQUE,          -- hex sha256 of the secret
  label       TEXT NOT NULL DEFAULT '',
  created_at  TIMESTAMPTZ NOT NULL,
  expires_at  TIMESTAMPTZ,
  revoked_at  TIMESTAMPTZ
);
CREATE INDEX tokens_owner_idx ON tokens (owner);
