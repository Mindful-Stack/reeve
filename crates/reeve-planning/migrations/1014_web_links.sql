-- Web links (kb/ADR-0031 R5): a titled URL on one item of any level, in a list of its own. item_key is plain text
-- with no foreign key, as goal_links.item_key is: nothing deletes an item, and the handler checks that the item exists.
-- No column says who added a row or when: the activity entries do (R7), and no column holds an email. The identity
-- orders an item's web links by when they were added; the unique index makes adding the same URL to the same item
-- idempotent. url holds the URL parser's serialisation, so "the same URL" is plain equality.
CREATE TABLE web_links (
  id       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  item_key TEXT NOT NULL,
  title    TEXT NOT NULL,
  url      TEXT NOT NULL,
  UNIQUE (item_key, url)
);
