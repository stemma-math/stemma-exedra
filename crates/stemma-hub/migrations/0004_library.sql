
CREATE TABLE libraries (
    group_id INTEGER PRIMARY KEY REFERENCES groups (id) ON DELETE CASCADE,
    -- The commit `main` was at when the hub last looked.
    head TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE library_builds (
    work_id INTEGER UNIQUE REFERENCES work_leases (id) ON DELETE CASCADE,
    accepted INTEGER NOT NULL DEFAULT 1,
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    commit_sha TEXT NOT NULL,
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

