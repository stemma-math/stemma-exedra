-- Working papers: each a LaTeX repository of the group's organization in
-- the forge, its authors, and its builds, which a workshop machine
-- compiles into a PDF.

CREATE TABLE papers (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    -- Its address is `p/<slug>`.
    slug TEXT NOT NULL,
    title TEXT NOT NULL,
    -- The repository of the group's organization: `paper-<slug>`.
    repository TEXT NOT NULL,
    created_by INTEGER NOT NULL REFERENCES users (id),
    -- The commit `main` was at when the hub last looked.
    head TEXT,
    -- The last build that made a PDF, whose PDF is the paper's.
    pdf_build INTEGER,
    created_at INTEGER NOT NULL,
    UNIQUE (group_id, slug)
);

-- Who writes it: its creator, and those its authors add. An author
-- approves the workshop's changes to it asked for by others.
CREATE TABLE paper_authors (
    paper_id INTEGER NOT NULL REFERENCES papers (id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    added_by INTEGER REFERENCES users (id) ON DELETE SET NULL,
    added_at INTEGER NOT NULL,
    PRIMARY KEY (paper_id, user_id)
);

-- A build's state is its job's, until the machine says how it went. Its
-- PDF, once uploaded, is `papers/<id>.pdf` in the data directory; only the
-- paper's last good one is kept.
CREATE TABLE paper_builds (
    id INTEGER PRIMARY KEY,
    paper_id INTEGER NOT NULL REFERENCES papers (id) ON DELETE CASCADE,
    commit_sha TEXT NOT NULL,
    job_id INTEGER REFERENCES jobs (id) ON DELETE SET NULL,
    succeeded INTEGER,
    error TEXT,
    log TEXT,
    -- The size of its uploaded PDF, in bytes, while it is kept.
    pdf_size INTEGER,
    created_at INTEGER NOT NULL,
    finished_at INTEGER
);

CREATE INDEX paper_builds_paper ON paper_builds (paper_id, id DESC);
CREATE INDEX paper_builds_job ON paper_builds (job_id);
