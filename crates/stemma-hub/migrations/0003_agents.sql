-- Agents: API tokens, people's agents, the machines they run on, the
-- workshop machines of groups, the jobs, and what each run consumed.

-- A personal token acts as its owner, with everything they can do. A job
-- token acts for a job's requester, in the job's group only, while the job
-- lasts; it posts, if at all, only in its thread and as its agent; it
-- pushes, if at all, only to one branch of one repository, its task's; and
-- it opens a few
-- threads, if its job is the workshop's.
-- Launcher tokens expire with an agent session and cannot mint tokens or browser sessions.
CREATE TABLE api_tokens (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    prefix TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER,
    revoked_at INTEGER,
    kind TEXT NOT NULL DEFAULT 'personal' CHECK (kind IN ('personal', 'launcher', 'job')),
    group_id INTEGER REFERENCES groups (id) ON DELETE CASCADE,
    thread_id INTEGER REFERENCES threads (id) ON DELETE CASCADE,
    agent TEXT,
    can_post INTEGER NOT NULL DEFAULT 1,
    -- The one branch a job's token pushes to: `refs/heads/workshop/w7`, of
    -- the one repository of its group named here: `library`.
    push_ref TEXT,
    push_repository TEXT,
    -- How many threads a job's token may still open.
    threads_left INTEGER NOT NULL DEFAULT 0,
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

-- A person's machines, where their agents run; called workers in the
-- protocol.
CREATE TABLE workers (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    owner_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    prefix TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_seen_at INTEGER,
    -- What it announced when it last connected: a JSON array of runners,
    -- whether it has Lean and LaTeX, and how it isolates the workshop's
    -- jobs (`docker`, `podman` or `host`; none if it cannot).
    runners TEXT NOT NULL DEFAULT '[]',
    lean INTEGER NOT NULL DEFAULT 0,
    latex INTEGER NOT NULL DEFAULT 0,
    isolation TEXT,
    version TEXT,
    -- A machine its owner's jobs go to first.
    preferred INTEGER NOT NULL DEFAULT 0,
    revoked_at INTEGER
);

CREATE INDEX workers_owner ON workers (owner_id);

-- The machines that work for a group's workshop: its admins add their own.
CREATE TABLE workshop_machines (
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    worker_id INTEGER NOT NULL REFERENCES workers (id) ON DELETE CASCADE,
    added_by INTEGER NOT NULL REFERENCES users (id),
    added_at INTEGER NOT NULL,
    PRIMARY KEY (group_id, worker_id)
);

CREATE INDEX workshop_machines_worker ON workshop_machines (worker_id);

-- The agents a machine offers the workshop, which its owner keeps: in
-- order of preference, each with how much it may do in a day (UTC).
CREATE TABLE workshop_agents (
    id INTEGER PRIMARY KEY,
    worker_id INTEGER NOT NULL REFERENCES workers (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    runner TEXT NOT NULL,
    model TEXT,
    args TEXT NOT NULL DEFAULT '[]',
    daily_jobs INTEGER,
    daily_minutes INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX workshop_agents_worker ON workshop_agents (worker_id, position);

CREATE TABLE jobs (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    -- An answer, on the requester's machines; or the workshop's: a build of
    -- the library or of a paper, or a task's run, on the group's workshop
    -- machines.
    kind TEXT NOT NULL
        CHECK (kind IN ('reply', 'library_build', 'paper_build', 'task_run')),
    -- The thread and message that asked for it, if one did.
    thread_id INTEGER REFERENCES threads (id) ON DELETE CASCADE,
    trigger_message INTEGER,
    requester_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- The agent as it was when the job was created, as JSON, so that later
    -- edits do not change a job on its way. None for builds.
    agent_handle TEXT,
    agent TEXT,
    -- The runner of an answer; `lean` or `latex` for builds; `workshop` for tasks,
    -- whose runner is the workshop agent a machine runs it with.
    runner TEXT NOT NULL,
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

-- Each time a job ran on a machine: how long, and what its runner said it
-- consumed. The workshop's routing and its panel read them.
CREATE TABLE job_runs (
    id INTEGER PRIMARY KEY,
    job_id INTEGER NOT NULL REFERENCES jobs (id) ON DELETE CASCADE,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    worker_id INTEGER REFERENCES workers (id) ON DELETE SET NULL,
    -- The machine's name then, to show after it is gone.
    machine TEXT NOT NULL,
    -- The workshop agent it ran with, for a task.
    workshop_agent_id INTEGER REFERENCES workshop_agents (id) ON DELETE SET NULL,
    runner TEXT NOT NULL,
    model TEXT,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    -- How it ended: done, failed, cancelled, or requeued when its machine
    -- went away.
    outcome TEXT,
    input_tokens INTEGER,
    output_tokens INTEGER,
    cache_read_tokens INTEGER,
    cache_write_tokens INTEGER,
    cost_usd REAL
);

CREATE INDEX job_runs_job ON job_runs (job_id);
CREATE INDEX job_runs_group ON job_runs (group_id, started_at);
CREATE INDEX job_runs_agent ON job_runs (workshop_agent_id, started_at);
