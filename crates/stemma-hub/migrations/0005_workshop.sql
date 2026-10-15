-- no-transaction
-- The workshop: tasks, the jobs that run them, the group's night window,
-- the members who lend their workers at night, and tokens that may push
-- to the library.
--
-- The jobs and the agents' profiles are made again with new constraints.
-- Builds refer to jobs, so foreign keys are off meanwhile: dropping the old
-- table must not unlink them. That needs the migration outside of sqlx's
-- transaction, so it makes its own.

PRAGMA foreign_keys = OFF;

BEGIN;

-- The night window, in minutes after midnight in the group's time zone,
-- and the number of the last task, so numbers are never reused.
ALTER TABLE groups ADD COLUMN last_task INTEGER NOT NULL DEFAULT 0;
ALTER TABLE groups ADD COLUMN night_from INTEGER NOT NULL DEFAULT 60;
ALTER TABLE groups ADD COLUMN night_to INTEGER NOT NULL DEFAULT 420;
ALTER TABLE groups ADD COLUMN time_zone TEXT NOT NULL DEFAULT 'UTC';

-- Whether a member lends their own worker to the group's workshop during
-- its night window.
ALTER TABLE memberships ADD COLUMN lends_worker INTEGER NOT NULL DEFAULT 0;

-- A task's token pushes to the library.
ALTER TABLE api_tokens ADD COLUMN can_push INTEGER NOT NULL DEFAULT 0;

-- Whether a worker said it works on tasks.
ALTER TABLE workers ADD COLUMN workshop INTEGER NOT NULL DEFAULT 0;

-- Agents can be the workshop.
CREATE TABLE agent_profiles_new (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    handle TEXT NOT NULL,
    display_name TEXT NOT NULL,
    runner TEXT NOT NULL,
    model TEXT,
    endpoint TEXT,
    instructions TEXT,
    scope TEXT NOT NULL CHECK (scope IN ('personal', 'group')),
    role TEXT NOT NULL DEFAULT 'answer' CHECK (role IN ('answer', 'editor', 'workshop')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (group_id, handle)
);

INSERT INTO agent_profiles_new
    (id, group_id, handle, display_name, runner, model, endpoint, instructions, scope, role,
     created_at, updated_at)
SELECT id, group_id, handle, display_name, runner, model, endpoint, instructions, scope, role,
       created_at, updated_at
FROM agent_profiles;

DROP TABLE agent_profiles;
ALTER TABLE agent_profiles_new RENAME TO agent_profiles;

-- Every group gets the workshop, on its own workers, with OpenCode.
INSERT INTO agent_profiles
    (group_id, handle, display_name, runner, scope, role, created_at, updated_at)
SELECT id, 'workshop', 'Workshop', 'opencode', 'group', 'workshop', created_at, created_at
FROM groups
WHERE NOT EXISTS (
    SELECT 1 FROM agent_profiles p WHERE p.group_id = groups.id AND p.handle = 'workshop'
);

-- Jobs now also run tasks.
CREATE TABLE jobs_new (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('reply', 'paper_update', 'library_build', 'task_run')),
    -- The thread and message that asked for it, if one did.
    thread_id INTEGER REFERENCES threads (id) ON DELETE CASCADE,
    trigger_message INTEGER,
    requester_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The agent's profile as it was when the job was created, as JSON, so
    -- that later edits do not change a job on its way. None for builds.
    agent_handle TEXT,
    agent TEXT,
    -- The runner, or `lean` for builds.
    runner TEXT NOT NULL,
    scope TEXT NOT NULL CHECK (scope IN ('personal', 'group')),
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'done', 'failed', 'cancelled')),
    worker_id INTEGER REFERENCES workers (id) ON DELETE SET NULL,
    -- The job token of the current run.
    token_id INTEGER REFERENCES api_tokens (id) ON DELETE SET NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    progress TEXT,
    error TEXT,
    -- The number of the message with the answer, or with the news.
    answer INTEGER,
    -- For a paper's update: the paper, and the revision it started from.
    paper_slug TEXT,
    base_revision INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

INSERT INTO jobs_new
    (id, group_id, kind, thread_id, trigger_message, requester_id, agent_handle, agent, runner,
     scope, state, worker_id, token_id, attempts, progress, error, answer, paper_slug,
     base_revision, created_at, updated_at)
SELECT id, group_id, kind, thread_id, trigger_message, requester_id, agent_handle, agent, runner,
       scope, state, worker_id, token_id, attempts, progress, error, answer, paper_slug,
       base_revision, created_at, updated_at
FROM jobs;

DROP TABLE jobs;
ALTER TABLE jobs_new RENAME TO jobs;

CREATE INDEX jobs_waiting ON jobs (state, id);
CREATE INDEX jobs_thread ON jobs (thread_id, id);

-- The workshop's tasks. A task is open until a worker runs it, then done,
-- failed or blocked; or cancelled.
CREATE TABLE tasks (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    number INTEGER NOT NULL,
    title TEXT NOT NULL,
    instructions TEXT NOT NULL,
    -- The message that asked for it.
    thread_id INTEGER NOT NULL REFERENCES threads (id) ON DELETE CASCADE,
    origin_message INTEGER NOT NULL,
    requester_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The agent that handed it over for its requester, if one did.
    agent TEXT,
    state TEXT NOT NULL
        CHECK (state IN ('open', 'running', 'done', 'failed', 'blocked', 'cancelled')),
    schedule TEXT NOT NULL CHECK (schedule IN ('now', 'night')),
    -- The job that runs it, now or last.
    job_id INTEGER REFERENCES jobs (id) ON DELETE SET NULL,
    runner TEXT,
    model TEXT,
    reason TEXT,
    report TEXT,
    log TEXT,
    -- A JSON array of the commits it pushed.
    commits TEXT NOT NULL DEFAULT '[]',
    -- The number of the message, in its thread, that reports on it.
    report_message INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    started_at INTEGER,
    finished_at INTEGER,
    UNIQUE (group_id, number)
);

CREATE INDEX tasks_group ON tasks (group_id, number DESC);
CREATE INDEX tasks_job ON tasks (job_id);

COMMIT;

PRAGMA foreign_keys = ON;
