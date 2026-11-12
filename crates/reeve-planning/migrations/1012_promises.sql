-- Promises (kb/ADR-0025): a date by which a story or an epic will be done, set by a person, optionally to a named
-- recipient. Stored on the promised row only (R1); state, due period and sprint membership are derived on read (R4,
-- R5, R8). Like the urgent columns, no item UPDATE names these, so an edit or a move never drops a promise and a
-- promise never bumps the version (R2).
ALTER TABLE stories ADD COLUMN promised_on DATE, ADD COLUMN promised_by TEXT, ADD COLUMN promised_at TIMESTAMPTZ, ADD COLUMN promised_to TEXT;
ALTER TABLE epics   ADD COLUMN promised_on DATE, ADD COLUMN promised_by TEXT, ADD COLUMN promised_at TIMESTAMPTZ, ADD COLUMN promised_to TEXT;
ALTER TABLE stories ADD CONSTRAINT stories_promise_whole CHECK (
  (promised_on IS NULL) = (promised_by IS NULL) AND (promised_on IS NULL) = (promised_at IS NULL)
  AND (promised_to IS NULL OR promised_on IS NOT NULL)
  AND (promised_to IS NULL OR char_length(promised_to) <= 200)
  AND (promised_to IS NULL OR btrim(promised_to) <> '') AND (promised_by IS NULL OR btrim(promised_by) <> ''));
ALTER TABLE epics   ADD CONSTRAINT epics_promise_whole   CHECK (
  (promised_on IS NULL) = (promised_by IS NULL) AND (promised_on IS NULL) = (promised_at IS NULL)
  AND (promised_to IS NULL OR promised_on IS NOT NULL)
  AND (promised_to IS NULL OR char_length(promised_to) <= 200)
  AND (promised_to IS NULL OR btrim(promised_to) <> '') AND (promised_by IS NULL OR btrim(promised_by) <> ''));
CREATE INDEX stories_promised_idx ON stories (promised_on) WHERE promised_on IS NOT NULL;
CREATE INDEX epics_promised_idx   ON epics (promised_on)   WHERE promised_on IS NOT NULL;
