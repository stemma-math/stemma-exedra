
CREATE TABLE papers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    -- Its address is `p/<slug>`.
    slug TEXT NOT NULL,
    title TEXT NOT NULL,
    title_ids TEXT,
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

CREATE TABLE paper_authors (
    paper_id INTEGER NOT NULL REFERENCES papers (id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    added_by INTEGER REFERENCES users (id) ON DELETE SET NULL,
    added_at INTEGER NOT NULL,
    PRIMARY KEY (paper_id, user_id)
);

CREATE TABLE paper_builds (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    paper_id INTEGER NOT NULL REFERENCES papers (id) ON DELETE CASCADE,
    commit_sha TEXT NOT NULL,
    succeeded INTEGER,
    error TEXT,
    log TEXT,
    -- The size of its uploaded PDF, in bytes, while it is kept.
    pdf_size INTEGER,
    created_at INTEGER NOT NULL,
    finished_at INTEGER
);

CREATE INDEX paper_builds_paper ON paper_builds (paper_id, id DESC);

