-- Indexes on the link columns kb/ADR-0003 calls mandatory and 1001 left out: an initiative's epics, and a story's or a
-- decision's comment thread. `story_blocks(blocker_key)` too, so deleting or querying by a blocker never scans.
CREATE INDEX epics_initiative_idx ON epics (initiative_key);
CREATE INDEX comments_story_idx ON comments (story_key);
CREATE INDEX comments_decision_idx ON comments (decision_id);
CREATE INDEX story_blocks_blocker_idx ON story_blocks (blocker_key);
