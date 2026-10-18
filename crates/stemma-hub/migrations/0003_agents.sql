-- Agents: API tokens, people's and groups' agents, the workers and machines
-- they run on, and their jobs.

-- A personal token acts as its owner, with everything they can do. A job
-- token acts for a job's requester, in the job's group only, while the job
-- lasts; it posts, if at all, only in its thread and as its agent, and
-- pushes to the library only for a task.
CREATE TABLE api_tokens (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    prefix TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER,
    revoked_at INTEGER,
    kind TEXT NOT NULL DEFAULT 'personal' CHECK (kind IN ('personal', 'job')),
    group_id INTEGER REFERENCES groups (id) ON DELETE CASCADE,
    thread_id INTEGER REFERENCES threads (id) ON DELETE CASCADE,
    agent TEXT,
    can_post INTEGER NOT NULL DEFAULT 1,
    can_push INTEGER NOT NULL DEFAULT 0,
    expires_at INTEGER
);

CREATE INDEX api_tokens_user ON api_tokens (user_id);

-- A person's own agents, beyond those everyone has (`@claude`, `@codex`,
-- `@opencode`), and what they changed of those. They answer for the person
-- in every group they are in, and run on the person's machines.
CREATE TABLE personal_agents (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    handle TEXT NOT NULL,
    display_name TEXT NOT NULL,
    runner TEXT NOT NULL,
    model TEXT,
    -- A JSON array of more arguments for the runner.
    args TEXT NOT NULL DEFAULT '[]',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (user_id, handle)
);

CREATE INDEX personal_agents_handle ON personal_agents (handle);

-- A group's own agents, which run on its workers: what each does when
-- summoned, answer or hand the message to the workshop as a task.
CREATE TABLE group_agents (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    handle TEXT NOT NULL,
    display_name TEXT NOT NULL,
    runner TEXT NOT NULL,
    model TEXT,
    args TEXT NOT NULL DEFAULT '[]',
    instructions TEXT,
    role TEXT NOT NULL DEFAULT 'answer' CHECK (role IN ('answer', 'workshop')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (group_id, handle)
);

-- A group's worker (group_id), or a person's machine (owner_id), never
-- both.
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
    -- What it announced when it last connected: a JSON array of runners,
    -- whether it has Lean and whether it works on the workshop's tasks.
    runners TEXT NOT NULL DEFAULT '[]',
    lean INTEGER NOT NULL DEFAULT 0,
    workshop INTEGER NOT NULL DEFAULT 0,
    version TEXT,
    -- A machine its owner's jobs go to first.
    preferred INTEGER NOT NULL DEFAULT 0,
    revoked_at INTEGER,
    CHECK ((group_id IS NULL) != (owner_id IS NULL))
);

CREATE INDEX workers_group ON workers (group_id);
CREATE INDEX workers_owner ON workers (owner_id);

CREATE TABLE jobs (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('reply', 'library_build', 'task_run')),
    -- The thread and message that asked for it, if one did.
    thread_id INTEGER REFERENCES threads (id) ON DELETE CASCADE,
    trigger_message INTEGER,
    requester_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The agent as it was when the job was created, as JSON, so that later
    -- edits do not change a job on its way. None for builds.
    agent_handle TEXT,
    agent TEXT,
    -- The runner, or `lean` for builds.
    runner TEXT NOT NULL,
    -- Whose agent runs it: the requester's own, on their machines, or the
    -- group's, on its workers.
    scope TEXT NOT NULL CHECK (scope IN ('personal', 'group')),
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'done', 'failed', 'cancelled')),
    worker_id INTEGER REFERENCES workers (id) ON DELETE SET NULL,
    -- The job token of the current run.
    token_id INTEGER REFERENCES api_tokens (id) ON DELETE SET NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    progress TEXT,
    error TEXT,
    -- The number of the message with the answer, or with the report.
    answer INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX jobs_waiting ON jobs (state, id);
CREATE INDEX jobs_thread ON jobs (thread_id, id);
