-- Pull requests linked to stories: a cache of GitHub state, owned by planning, keyed by (repo, number).
CREATE TABLE story_prs (
  story_key TEXT NOT NULL REFERENCES stories(key),
  repo TEXT NOT NULL,
  number BIGINT NOT NULL,
  url TEXT NOT NULL,
  title TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('draft','open','merged','closed')),
  updated_at TIMESTAMPTZ NOT NULL,
  PRIMARY KEY (repo, number)
);
CREATE INDEX story_prs_story_idx ON story_prs (story_key);
