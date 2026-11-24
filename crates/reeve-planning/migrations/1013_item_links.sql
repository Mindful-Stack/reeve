-- Item links (kb/ADR-0031): a named, informational connection between two items of any level, an edge owned by
-- neither item (R4). Keys are plain text with no foreign key, as goal_links.item_key is: nothing deletes an item, and
-- the handler checks that both ends exist. No column says who added a row or when: the activity entries do (R7), as
-- for story_blocks. The primary key makes an add idempotent without an idempotency-key column.
-- relates_to has no direction: it is one row with the lower key first. "Lower" is byte order, held by the last check
-- with an explicit collation so it never depends on the database's locale.
CREATE TABLE item_links (
  from_key  TEXT NOT NULL,
  to_key    TEXT NOT NULL,
  link_type TEXT NOT NULL CHECK (link_type IN ('follows', 'duplicates', 'relates_to')),
  PRIMARY KEY (from_key, to_key, link_type),
  CHECK (from_key <> to_key),
  CHECK (link_type <> 'relates_to' OR from_key < to_key COLLATE "C")
);
CREATE INDEX item_links_to_idx ON item_links (to_key);
