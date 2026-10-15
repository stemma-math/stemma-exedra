-- The editor brings the papers up to date once a night.

-- When the group's papers were last looked at, at night.
ALTER TABLE groups ADD COLUMN papers_updated_at INTEGER;

-- Whether an editor's job is a paper's nightly update, which nobody asked
-- for in a message.
ALTER TABLE jobs ADD COLUMN nightly INTEGER NOT NULL DEFAULT 0;
