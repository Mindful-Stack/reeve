-- crates/reeve-planning/migrations/1002_comment_idempotency.sql
-- Comment creation takes a caller-supplied idempotency key like every other create (kb/ADR-0005 R3). Nullable: imported
-- comments carry an external_ref instead, and comments written before this migration have neither.
ALTER TABLE comments ADD COLUMN idempotency_key TEXT;
CREATE UNIQUE INDEX comments_idempotency_key_idx ON comments (idempotency_key) WHERE idempotency_key IS NOT NULL;
