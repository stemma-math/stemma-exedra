-- Recipes are folders with a script, and a process delivers outputs of its
-- choosing; the git orchestrator prepares each pull request's merge, and a
-- person approves it.

-- Requests made with the earlier recipes end here: those recipes no longer run.
UPDATE workshop_requests
SET recipe = json_set(recipe, '$.recipe', json('null')),
    state = CASE WHEN state IN ('waiting', 'running') THEN 'cancelled' ELSE state END,
    next_at = NULL,
    wake = 0,
    waiting = NULL;
UPDATE workshop_processes
SET recipe = json_set(recipe, '$.recipe', json('null'))
WHERE recipe IS NOT NULL;
UPDATE work_leases
SET state = 'cancelled'
WHERE kind = 'process' AND state IN ('queued', 'running');
UPDATE workshop_processes
SET finished_at = coalesce(finished_at, CAST(strftime('%s', 'now') AS INTEGER) * 1000);
UPDATE api_tokens
SET revoked_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000
WHERE kind = 'process' AND revoked_at IS NULL;
DELETE FROM workshop_offers;
-- Reviews by the group's model are gone; `rebase_review` and `repositories`
-- of processes, and `workshop_reviews`, are no longer used.
DELETE FROM workshop_reviews;

-- A process token's tools beyond reading and posting in its request.
ALTER TABLE api_tokens ADD COLUMN permissions TEXT NOT NULL DEFAULT '[]';

-- The branch a process may push in any of the group's repositories, if its
-- recipe admits pull requests.
ALTER TABLE workshop_processes ADD COLUMN branch TEXT;

-- Each pull request a process opened, and its merge into main: prepared by
-- the git orchestrator, approved by a person.
CREATE TABLE workshop_merges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    process_id INTEGER NOT NULL REFERENCES workshop_processes (id) ON DELETE CASCADE,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    repository TEXT NOT NULL,
    pull_number INTEGER NOT NULL,
    url TEXT NOT NULL,
    branch TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'preparing'
        CHECK (state IN ('preparing', 'ready', 'waiting', 'merged', 'closed')),
    -- The prepared merge: the main and head it joins, and its tree.
    main TEXT,
    head TEXT,
    tree TEXT,
    conflicts TEXT NOT NULL DEFAULT '[]',
    explanation TEXT,
    message TEXT,
    retry_at INTEGER,
    resume_month TEXT,
    approved_by INTEGER REFERENCES users (id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (group_id, repository, pull_number)
);
CREATE INDEX workshop_merges_open ON workshop_merges (group_id, repository, state);

-- What a process delivered, as it said, before the hub checked it.
ALTER TABLE workshop_processes ADD COLUMN outputs TEXT NOT NULL DEFAULT '[]';
