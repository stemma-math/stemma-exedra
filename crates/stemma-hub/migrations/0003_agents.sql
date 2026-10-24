
CREATE TABLE api_tokens (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    prefix TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER,
    revoked_at INTEGER,
    kind TEXT NOT NULL DEFAULT 'personal' CHECK (kind IN ('personal', 'agent', 'job', 'build')),
    group_id INTEGER REFERENCES groups (id) ON DELETE CASCADE,
    build_work INTEGER,
    lease_attempt INTEGER,
    thread_id INTEGER REFERENCES threads (id) ON DELETE CASCADE,
    agent TEXT,
    can_post INTEGER NOT NULL DEFAULT 1,
    attachments TEXT NOT NULL DEFAULT 'mine' CHECK (attachments IN ('none', 'mine', 'group')),
    expires_at INTEGER
);

CREATE INDEX api_tokens_user ON api_tokens (user_id);

CREATE TABLE jobs (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    kind TEXT NOT NULL
        CHECK (kind = 'reply'),
    -- The thread and message that asked for it, if one did.
    thread_id INTEGER REFERENCES threads (id) ON DELETE CASCADE,
    trigger_message INTEGER,
    requester_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    agent_handle TEXT,
    runner TEXT NOT NULL,
    state TEXT NOT NULL
        CHECK (state IN ('queued', 'running', 'held', 'done', 'failed', 'cancelled')),
    worker_id INTEGER REFERENCES api_tokens (id) ON DELETE SET NULL,
    -- The job token of the current run.
    token_id INTEGER REFERENCES api_tokens (id) ON DELETE SET NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    progress TEXT,
    error TEXT,
    -- The number of the message with the answer.
    answer INTEGER,
    -- An answer that seems to hold a secret, until its requester publishes
    -- or discards it.
    held_answer TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX jobs_waiting ON jobs (state, id);
CREATE INDEX jobs_thread ON jobs (thread_id, id);


-- Shared by builds and future workshop processes. One lease per piece of work.
CREATE TABLE work_leases (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    repository TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('library', 'paper')),
    state TEXT NOT NULL DEFAULT 'queued' CHECK (state IN ('queued', 'running', 'passed', 'failed', 'cancelled')),
    holder INTEGER REFERENCES api_tokens (id) ON DELETE SET NULL,
    connection INTEGER,
    attempts INTEGER NOT NULL DEFAULT 0,
    heartbeat INTEGER,
    token_id INTEGER REFERENCES api_tokens (id) ON DELETE SET NULL,
    machine TEXT,
    error TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE (group_id, repository, commit_sha)
);
