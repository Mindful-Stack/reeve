-- Connections (kb/ADR-0029 R5, R9): an approved, exchanged request to connect an MCP client. A person, a client,
-- a scope and an end date thirty days after the approval. Every MCP token belongs to one, and a connection has at
-- most one live refresh token. Who revoked a connection is in its activity entry, not here. Owned by identity; no
-- foreign key leaves this context (kb/ADR-0010 R3) and no tenant column (kb/ADR-0016 R1). `owner` holds an email
-- address; the erasure procedure (GRA-664) must cover it.
--
-- The MCP tokens that exist are the developer command's (`reeve-server mcp-token-issue`): they live one hour and
-- have no connection, so they are deleted. From here on the command creates a connection of its own.
-- Going back: an older server refuses a database that has this migration. Restore the pre-deploy backup.
DELETE FROM tokens WHERE audience = 'mcp';

CREATE TABLE connections (
  id             BIGSERIAL PRIMARY KEY,
  owner          TEXT NOT NULL REFERENCES users(email),
  client_id      TEXT NOT NULL,
  client_name    TEXT NOT NULL,               -- as approved
  scope          TEXT NOT NULL,               -- write includes read
  approved_at    TIMESTAMPTZ NOT NULL,
  ends_at        TIMESTAMPTZ NOT NULL,
  revoked_at     TIMESTAMPTZ,
  revoked_reason TEXT,
  CONSTRAINT connections_scope_known CHECK (scope IN ('read', 'write')),
  CONSTRAINT connections_reason_known
    CHECK (revoked_reason IN ('revoked', 'refresh_token_reused', 'user_disabled', 'restored')),
  CONSTRAINT connections_end_after_approval CHECK (ends_at > approved_at),
  CONSTRAINT connections_revoked_together CHECK ((revoked_at IS NULL) = (revoked_reason IS NULL)),
  -- The target of tokens' two-column foreign key: a token's owner is its connection's owner.
  UNIQUE (id, owner)
);
CREATE INDEX connections_owner_idx ON connections (owner);

ALTER TABLE tokens ADD COLUMN connection_id BIGINT;
ALTER TABLE tokens ADD CONSTRAINT tokens_connection_fk
  FOREIGN KEY (connection_id, owner) REFERENCES connections (id, owner) ON DELETE CASCADE;
ALTER TABLE tokens ADD CONSTRAINT tokens_mcp_has_connection
  CHECK ((audience = 'mcp') = (connection_id IS NOT NULL));
-- kb/ADR-0029 R5: an MCP token expires, after it is created and at most one hour after. (IS NOT NULL is needed:
-- a CHECK that is NULL passes.)
ALTER TABLE tokens ADD CONSTRAINT tokens_mcp_expires_within_an_hour
  CHECK (audience <> 'mcp' OR (expires_at IS NOT NULL AND expires_at > created_at
                               AND expires_at <= created_at + interval '1 hour'));
CREATE INDEX tokens_connection_idx ON tokens (connection_id);

CREATE TABLE refresh_tokens (
  id            BIGSERIAL PRIMARY KEY,
  connection_id BIGINT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
  hash          TEXT NOT NULL UNIQUE,          -- hex sha256 of the secret
  created_at    TIMESTAMPTZ NOT NULL,
  used_at       TIMESTAMPTZ                    -- the newest of a connection's is when it last refreshed
);
-- Rotation (kb/ADR-0029 R5): one live refresh token per connection.
CREATE UNIQUE INDEX refresh_tokens_one_live ON refresh_tokens (connection_id) WHERE used_at IS NULL;
-- The newest `used_at` of a connection (when it last refreshed) and the cascade of a deleted connection: the
-- partial index above holds only the one live row.
CREATE INDEX refresh_tokens_connection_idx ON refresh_tokens (connection_id, used_at);
