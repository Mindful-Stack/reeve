-- MCP clients that registered themselves (kb/ADR-0029 R3; RFC 7591): an app that publishes no metadata document
-- asks for a client id and gets one of this server's making. Nothing here is a secret, and nothing here is a
-- person's. Owned by identity; no foreign key leaves this context (kb/ADR-0010 R3) and none points here: a
-- connection keeps the client id, the name and the host it was approved with, so deleting a registration ends no
-- connection. R3's cap and expiry are the code's (`registrations.rs`): rows are deleted when a registration is
-- made.
-- Going back: an older server refuses a database that has this migration. Restore the pre-deploy backup.
CREATE TABLE registered_clients (
  client_id     TEXT PRIMARY KEY,
  client_name   TEXT NOT NULL,               -- self-asserted; shown as what the app calls itself
  redirect_uris TEXT[] NOT NULL,             -- each was in [mcp] redirect_uris when it was registered
  created_at    TIMESTAMPTZ NOT NULL,
  last_used_at  TIMESTAMPTZ,                 -- NULL until a person approves a connection for it
  -- Never a URL: a metadata-document client id can never be a key here.
  CONSTRAINT registered_clients_id_shape CHECK (client_id ~ '^dyn_[0-9a-f]{48}$'),
  CONSTRAINT registered_clients_redirect_count CHECK (cardinality(redirect_uris) BETWEEN 1 AND 5)
);
-- The oldest never-used row, which is the one dropped to make room.
CREATE INDEX registered_clients_unused_idx ON registered_clients (created_at) WHERE last_used_at IS NULL;

-- The app's identity as people are shown it, kept with the connection: for a client that registered itself the
-- client id (`dyn_…`) names no host, and its registration can be dropped while the connection lives. NULL (every
-- connection made before this migration, and the developer command's) means: derive it from the client id.
ALTER TABLE connections ADD COLUMN client_host TEXT;
