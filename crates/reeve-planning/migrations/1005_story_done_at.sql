-- When a story last entered done. NULL on a story that is not done, and on a story imported already done: it finished
-- before the planner tracked when, so it falls outside every done window. Native done stories take updated_at, the
-- closest fact recorded before this column existed.
ALTER TABLE stories ADD COLUMN done_at TIMESTAMPTZ;
UPDATE stories SET done_at = updated_at WHERE delivery = 'done' AND external_ref IS NULL;
