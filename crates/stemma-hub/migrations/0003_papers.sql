-- Working papers, their revisions and who read which; editors among the
-- agents; and jobs that are not answers in a thread.

CREATE TABLE papers (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    slug TEXT NOT NULL,
    -- The number of the current revision, from 1.
    revision INTEGER NOT NULL DEFAULT 0,
    created_by INTEGER NOT NULL REFERENCES users (id),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (group_id, slug)
);

CREATE INDEX papers_updated ON papers (group_id, updated_at DESC);

-- Revisions are never changed: a paper's history is its revisions.
CREATE TABLE paper_revisions (
    id INTEGER PRIMARY KEY,
    paper_id INTEGER NOT NULL REFERENCES papers (id) ON DELETE CASCADE,
    number INTEGER NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    author_id INTEGER NOT NULL REFERENCES users (id),
    -- The agent that wrote it for its author, if one did.
    agent TEXT,
    reason TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (paper_id, number)
);

-- The revision of a paper each person read last.
CREATE TABLE paper_reads (
    paper_id INTEGER NOT NULL REFERENCES papers (id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    revision INTEGER NOT NULL,
    read_at INTEGER NOT NULL,
    PRIMARY KEY (paper_id, user_id)
);

CREATE INDEX threads_anchor ON threads (group_id, anchor);

-- What an agent does when summoned: answer, or update a paper.
ALTER TABLE agent_profiles ADD COLUMN role TEXT NOT NULL DEFAULT 'answer'
    CHECK (role IN ('answer', 'editor'));

-- Every group gets an editor: each person's own Claude, updating papers.
INSERT INTO agent_profiles
    (group_id, handle, display_name, runner, scope, role, created_at, updated_at)
SELECT id, 'editor', 'Editor', 'claude-code', 'personal', 'editor', created_at, created_at
FROM groups
WHERE NOT EXISTS (
    SELECT 1 FROM agent_profiles p WHERE p.group_id = groups.id AND p.handle = 'editor'
);

-- Jobs now also update papers and build the library: they need not come
-- from a thread, nor be an agent's. Nothing refers to jobs, so the table is
-- made again with the new constraints.
CREATE TABLE jobs_new (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('reply', 'paper_update', 'library_build')),
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
     scope, state, worker_id, token_id, attempts, progress, error, answer, created_at, updated_at)
SELECT id, group_id, kind, thread_id, trigger_message, requester_id, agent_handle, agent, runner,
       scope, state, worker_id, token_id, attempts, progress, error, answer, created_at, updated_at
FROM jobs;

DROP TABLE jobs;
ALTER TABLE jobs_new RENAME TO jobs;

CREATE INDEX jobs_waiting ON jobs (state, id);
CREATE INDEX jobs_thread ON jobs (thread_id, id);
