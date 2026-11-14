-- Where a token works (kb/ADR-0029 R5, R6): 'api' (the API, the event stream, attachments, the session cookie) or
-- 'mcp' (the MCP endpoint only). Every token that exists becomes an API token. There is no default: an insert that
-- forgets the audience fails instead of becoming an API token by omission. An MCP token is always interactive
-- (kb/ADR-0029 R5); the audience is not a third token kind (kb/ADR-0015 R7). Owned by identity; no foreign key leaves
-- this context (kb/ADR-0010 R3) and no tenant column (kb/ADR-0016 R1).
-- Going back: an older server refuses a database that has this migration. Restore the pre-deploy backup.
ALTER TABLE tokens ADD COLUMN audience TEXT CHECK (audience IN ('api', 'mcp'));
UPDATE tokens SET audience = 'api';
ALTER TABLE tokens ALTER COLUMN audience SET NOT NULL;
ALTER TABLE tokens ADD CONSTRAINT tokens_mcp_is_interactive CHECK (audience <> 'mcp' OR kind = 'interactive');
