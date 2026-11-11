-- crates/reeve-planning/migrations/1011_attachments.sql
-- Attachments (kb/ADR-0022, kb/ADR-0023): images owned by at most one initiative, epic or story; an attachment owned by
-- none is an upload not yet saved with any text, visible only to its uploader. The bytes are 1 MiB chunks.
CREATE TABLE attachments (
    id uuid PRIMARY KEY,
    initiative_key TEXT REFERENCES initiatives(key),
    epic_key TEXT REFERENCES epics(key),
    story_key TEXT REFERENCES stories(key),
    -- An email; no foreign key to identity (kb/ADR-0010 R3).
    uploaded_by TEXT NOT NULL,
    uploaded_at TIMESTAMPTZ NOT NULL,
    filename TEXT NOT NULL,
    content_type TEXT NOT NULL,
    byte_size BIGINT NOT NULL,
    sha256 BYTEA NOT NULL,
    -- Lives as long as its row: a key whose upload was deleted or swept uploads anew.
    idempotency_key TEXT,
    CHECK (num_nonnulls(initiative_key, epic_key, story_key) <= 1),
    CHECK (content_type IN ('image/png', 'image/jpeg', 'image/webp')),
    CHECK (octet_length(sha256) = 32),
    CHECK (byte_size >= 0)
);
CREATE UNIQUE INDEX attachments_idempotency_key_idx ON attachments (idempotency_key) WHERE idempotency_key IS NOT NULL;
CREATE INDEX attachments_initiative_idx ON attachments (initiative_key) WHERE initiative_key IS NOT NULL;
CREATE INDEX attachments_epic_idx ON attachments (epic_key) WHERE epic_key IS NOT NULL;
CREATE INDEX attachments_story_idx ON attachments (story_key) WHERE story_key IS NOT NULL;
CREATE INDEX attachments_unclaimed_idx ON attachments (uploaded_at)
    WHERE initiative_key IS NULL AND epic_key IS NULL AND story_key IS NULL;

CREATE TABLE attachment_chunks (
    attachment_id uuid NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
    n INT NOT NULL,
    bytes BYTEA NOT NULL CHECK (octet_length(bytes) <= 1048576),
    CHECK (n >= 0),
    PRIMARY KEY (attachment_id, n)
);
-- Image bytes are already compressed: stored out of line without a compression attempt (PostgreSQL, "TOAST").
ALTER TABLE attachment_chunks ALTER COLUMN bytes SET STORAGE EXTERNAL;
