
CREATE TABLE api_tokens (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    prefix TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER,
    revoked_at INTEGER,
    kind TEXT NOT NULL DEFAULT 'personal' CHECK (kind IN ('personal', 'agent', 'job', 'build', 'process')),
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
    repository TEXT,
    commit_sha TEXT,
    kind TEXT NOT NULL CHECK (kind IN ('library', 'paper', 'process')),
    state TEXT NOT NULL DEFAULT 'queued' CHECK (state IN ('queued', 'running', 'passed', 'failed', 'cancelled')),
    holder INTEGER REFERENCES api_tokens (id) ON DELETE SET NULL,
    connection INTEGER,
    attempts INTEGER NOT NULL DEFAULT 0,
    heartbeat INTEGER,
    token_id INTEGER REFERENCES api_tokens (id) ON DELETE SET NULL,
    machine TEXT,
    error TEXT,
    created_at INTEGER NOT NULL,
    review_build INTEGER NOT NULL DEFAULT 0,
    UNIQUE (group_id, repository, commit_sha)
);


-- A request is a normal numbered thread. Its committed recipe is immutable.
CREATE TABLE workshop_requests (
    thread_id INTEGER PRIMARY KEY REFERENCES threads (id) ON DELETE CASCADE,
    recipe TEXT NOT NULL,
    ask TEXT NOT NULL,
    orchestrated INTEGER NOT NULL DEFAULT 0,
    wake INTEGER NOT NULL DEFAULT 0,
    thinking INTEGER NOT NULL DEFAULT 0,
    resume_month TEXT,
    waiting TEXT,
    expires_at INTEGER,
    state TEXT NOT NULL DEFAULT 'waiting' CHECK (state IN ('waiting','running','done','failed','cancelled','expired')),
    recurring INTEGER NOT NULL,
    paused INTEGER NOT NULL DEFAULT 0,
    next_at INTEGER,
    created_at INTEGER NOT NULL
);
CREATE TABLE workshop_processes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES workshop_requests (thread_id) ON DELETE CASCADE,
    work_id INTEGER NOT NULL UNIQUE REFERENCES work_leases (id) ON DELETE CASCADE,
    slot INTEGER NOT NULL,
    recipe TEXT,
    ask TEXT,
    rebase_review INTEGER REFERENCES workshop_reviews (id) ON DELETE SET NULL,
    repositories TEXT,
    machine_id INTEGER REFERENCES api_tokens (id),
    expires_at INTEGER NOT NULL,
    deadline INTEGER,
    expiry_woken INTEGER NOT NULL DEFAULT 0,
    owner_id INTEGER REFERENCES users (id),
    started_at INTEGER,
    finished_at INTEGER,
    branches TEXT NOT NULL DEFAULT '[]',
    pushed INTEGER NOT NULL DEFAULT 0,
    usage TEXT NOT NULL DEFAULT '{}',
    summary TEXT,
    -- A retry is a new run, with its own branch, owner, token and lease.
    retry_of INTEGER UNIQUE REFERENCES workshop_processes (id) ON DELETE SET NULL
);
CREATE INDEX workshop_slots ON workshop_processes (thread_id, slot);
-- A decision records only delivery/lease state. Authority remains on the machine.
CREATE TABLE workshop_offers (
    process_id INTEGER NOT NULL REFERENCES workshop_processes (id) ON DELETE CASCADE,
    machine_id INTEGER NOT NULL REFERENCES api_tokens (id) ON DELETE CASCADE,
    connection INTEGER NOT NULL,
    decision TEXT NOT NULL DEFAULT 'offered',
    PRIMARY KEY (process_id, machine_id)
);
CREATE INDEX workshop_due ON workshop_requests (next_at) WHERE state IN ('waiting','running');

CREATE TRIGGER workshop_process_deleted AFTER DELETE ON workshop_processes BEGIN
    DELETE FROM work_leases WHERE id=old.work_id;
END;

-- Keys never leave the hub. Only the masked metadata is exposed by its API.
CREATE TABLE workshop_models (
    group_id INTEGER PRIMARY KEY REFERENCES groups (id) ON DELETE CASCADE,
    provider TEXT NOT NULL DEFAULT 'anthropic',
    model TEXT NOT NULL DEFAULT '',
    encrypted_key BLOB,
    last_four TEXT,
    set_by INTEGER REFERENCES users (id) ON DELETE SET NULL,
    set_at INTEGER,
    monthly_cap INTEGER NOT NULL DEFAULT 1000000,
    step_cap INTEGER NOT NULL DEFAULT 20
);
CREATE TABLE workshop_usage (
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    month TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cost_usd REAL,
    PRIMARY KEY (group_id, month)
);
CREATE TABLE workshop_reviews (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    process_id INTEGER NOT NULL REFERENCES workshop_processes (id) ON DELETE CASCADE,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    repository TEXT NOT NULL,
    pull_number INTEGER NOT NULL,
    url TEXT NOT NULL,
    branch TEXT NOT NULL,
    head TEXT NOT NULL,
    build_work INTEGER REFERENCES work_leases (id) ON DELETE SET NULL,
    state TEXT NOT NULL DEFAULT 'queued',
    message TEXT,
    rebases INTEGER NOT NULL DEFAULT 0,
    rebase_process INTEGER REFERENCES workshop_processes (id) ON DELETE SET NULL,
    resume_month TEXT,
    UNIQUE (group_id, repository, pull_number)
);
