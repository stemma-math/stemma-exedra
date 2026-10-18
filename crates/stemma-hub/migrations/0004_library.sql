-- Libraries: each group's Lean project, a bare git repository in the data
-- directory, its builds, and what the inspection of each found; and the
-- workshop's tasks on it.

CREATE TABLE libraries (
    group_id INTEGER PRIMARY KEY REFERENCES groups (id) ON DELETE CASCADE,
    -- The commit `main` was at when the hub last looked.
    head TEXT,
    created_at INTEGER NOT NULL
);

-- A build's state is its job's, until the worker says how it went.
CREATE TABLE library_builds (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    commit_sha TEXT NOT NULL,
    job_id INTEGER REFERENCES jobs (id) ON DELETE SET NULL,
    -- Set when the worker reports: whether it built and was inspected.
    succeeded INTEGER,
    error TEXT,
    log TEXT,
    -- Lean's version, as it ran.
    lean TEXT,
    created_at INTEGER NOT NULL,
    finished_at INTEGER
);

CREATE INDEX library_builds_group ON library_builds (group_id, id DESC);
CREATE INDEX library_builds_job ON library_builds (job_id);

-- The declarations a successful build's inspection reported. Only the
-- latest successful builds keep theirs.
CREATE TABLE library_declarations (
    build_id INTEGER NOT NULL REFERENCES library_builds (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    module TEXT NOT NULL,
    kind TEXT NOT NULL,
    type TEXT NOT NULL,
    doc TEXT,
    start_line INTEGER,
    start_column INTEGER,
    end_line INTEGER,
    end_column INTEGER,
    sorry INTEGER NOT NULL,
    data_sorry INTEGER NOT NULL,
    -- JSON arrays of names.
    axioms TEXT NOT NULL,
    uses TEXT NOT NULL,
    PRIMARY KEY (build_id, name)
);

CREATE INDEX library_declarations_module ON library_declarations (build_id, module);

-- Who uses what, to answer "used by".
CREATE TABLE library_uses (
    build_id INTEGER NOT NULL REFERENCES library_builds (id) ON DELETE CASCADE,
    used TEXT NOT NULL,
    user TEXT NOT NULL,
    PRIMARY KEY (build_id, used, user)
);

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
