-- Agents: token scopes, agent profiles, workers and the jobs they run.

-- A personal token acts as its owner, with everything they can do. A job
-- token acts for a job's requester, in the job's group only, while the job
-- lasts; it posts, if at all, only in its thread and as its agent.
ALTER TABLE api_tokens ADD COLUMN kind TEXT NOT NULL DEFAULT 'personal'
    CHECK (kind IN ('personal', 'job'));
ALTER TABLE api_tokens ADD COLUMN group_id INTEGER REFERENCES groups (id) ON DELETE CASCADE;
ALTER TABLE api_tokens ADD COLUMN thread_id INTEGER REFERENCES threads (id) ON DELETE CASCADE;
ALTER TABLE api_tokens ADD COLUMN agent TEXT;
ALTER TABLE api_tokens ADD COLUMN can_post INTEGER NOT NULL DEFAULT 1;
ALTER TABLE api_tokens ADD COLUMN expires_at INTEGER;

CREATE TABLE agent_profiles (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    handle TEXT NOT NULL,
    display_name TEXT NOT NULL,
    runner TEXT NOT NULL,
    model TEXT,
    endpoint TEXT,
    instructions TEXT,
    scope TEXT NOT NULL CHECK (scope IN ('personal', 'group')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (group_id, handle)
);

-- Every group starts with each person's own Claude and Codex.
INSERT INTO agent_profiles
    (group_id, handle, display_name, runner, scope, created_at, updated_at)
SELECT id, 'claude', 'Claude', 'claude-code', 'personal', created_at, created_at FROM groups
UNION ALL
SELECT id, 'codex', 'Codex', 'codex', 'personal', created_at, created_at FROM groups;

-- A worker is a group's (group_id) or a member's (owner_id), never both.
CREATE TABLE workers (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    group_id INTEGER REFERENCES groups (id) ON DELETE CASCADE,
    owner_id INTEGER REFERENCES users (id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    prefix TEXT NOT NULL,
    created_by INTEGER NOT NULL REFERENCES users (id),
    created_at INTEGER NOT NULL,
    last_seen_at INTEGER,
    -- What it announced when it last connected: a JSON array of runners.
    runners TEXT NOT NULL DEFAULT '[]',
    lean INTEGER NOT NULL DEFAULT 0,
    version TEXT,
    revoked_at INTEGER,
    CHECK ((group_id IS NULL) != (owner_id IS NULL))
);

CREATE INDEX workers_group ON workers (group_id);
CREATE INDEX workers_owner ON workers (owner_id);

CREATE TABLE jobs (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    thread_id INTEGER NOT NULL REFERENCES threads (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('reply')),
    -- The message it answers, by number.
    trigger_message INTEGER NOT NULL,
    requester_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The agent's profile as it was when the job was created, as JSON, so
    -- that later edits do not change a job on its way.
    agent_handle TEXT NOT NULL,
    agent TEXT NOT NULL,
    runner TEXT NOT NULL,
    scope TEXT NOT NULL CHECK (scope IN ('personal', 'group')),
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'done', 'failed', 'cancelled')),
    worker_id INTEGER REFERENCES workers (id) ON DELETE SET NULL,
    -- The job token of the current run.
    token_id INTEGER REFERENCES api_tokens (id) ON DELETE SET NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    progress TEXT,
    error TEXT,
    -- The number of the message with the answer.
    answer INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX jobs_waiting ON jobs (state, id);
CREATE INDEX jobs_thread ON jobs (thread_id, id);
