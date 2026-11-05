-- Comments on initiatives and epics too (GRA-70): discussion about them had nowhere to go but the brief. A comment is on
-- exactly one thing: a story, a required decision, an initiative or an epic.
ALTER TABLE comments
  ADD COLUMN initiative_key TEXT REFERENCES initiatives(key),
  ADD COLUMN epic_key TEXT REFERENCES epics(key);
ALTER TABLE comments DROP CONSTRAINT comments_check;
ALTER TABLE comments ADD CONSTRAINT comments_one_target
  CHECK (num_nonnulls(story_key, decision_id, initiative_key, epic_key) = 1);
CREATE INDEX comments_initiative_idx ON comments (initiative_key);
CREATE INDEX comments_epic_idx ON comments (epic_key);
