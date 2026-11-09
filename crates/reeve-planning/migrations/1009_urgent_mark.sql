-- Urgent marks (kb/ADR-0017): stored only on the marked item; lineage urgency and expiry are computed on read. The
-- item UPDATE statements never name these columns, so an edit, a move or the discovery mark cannot drop a mark.
ALTER TABLE initiatives ADD COLUMN urgent_at TIMESTAMPTZ, ADD COLUMN urgent_by TEXT, ADD COLUMN urgent_reason TEXT;
ALTER TABLE epics       ADD COLUMN urgent_at TIMESTAMPTZ, ADD COLUMN urgent_by TEXT, ADD COLUMN urgent_reason TEXT;
ALTER TABLE stories     ADD COLUMN urgent_at TIMESTAMPTZ, ADD COLUMN urgent_by TEXT, ADD COLUMN urgent_reason TEXT;
ALTER TABLE initiatives ADD CONSTRAINT initiatives_urgent_whole CHECK ((urgent_at IS NULL) = (urgent_by IS NULL) AND (urgent_reason IS NULL OR urgent_at IS NOT NULL));
ALTER TABLE epics       ADD CONSTRAINT epics_urgent_whole       CHECK ((urgent_at IS NULL) = (urgent_by IS NULL) AND (urgent_reason IS NULL OR urgent_at IS NOT NULL));
ALTER TABLE stories     ADD CONSTRAINT stories_urgent_whole     CHECK ((urgent_at IS NULL) = (urgent_by IS NULL) AND (urgent_reason IS NULL OR urgent_at IS NOT NULL));
CREATE INDEX initiatives_urgent_idx ON initiatives (key) WHERE urgent_at IS NOT NULL;
CREATE INDEX epics_urgent_idx       ON epics (key)       WHERE urgent_at IS NOT NULL;
CREATE INDEX stories_urgent_idx     ON stories (key)     WHERE urgent_at IS NOT NULL;
