CREATE TABLE activity (
  id          BIGSERIAL PRIMARY KEY,
  at          TIMESTAMPTZ NOT NULL,
  actor       TEXT NOT NULL,            -- user email; agent principal name in milestone two
  via         TEXT NOT NULL CHECK (via IN ('interactive','automation','system')),
  entity_type TEXT NOT NULL,            -- initiative | epic | story | decision | comment | token
  entity_key  TEXT NOT NULL,            -- GRA-412, or the decision id as text
  action      TEXT NOT NULL,            -- created | edited | moved | override | discovery_marked | ...
  payload     JSONB NOT NULL DEFAULT '{}'::jsonb
);
CREATE INDEX activity_entity_idx ON activity (entity_type, entity_key, id);
CREATE INDEX activity_action_idx ON activity (action, at);
