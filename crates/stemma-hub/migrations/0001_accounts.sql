-- Accounts, their tokens, groups, their members and invitations. Times are
-- milliseconds since the Unix epoch, in UTC.

CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    -- Administers the hub: creates groups.
    hub_admin INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

-- Secrets are stored as the hex SHA-256 of what the client holds.
CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE INDEX sessions_user ON sessions (user_id);

CREATE TABLE groups (
    id INTEGER PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    -- The numbers of the last thread and task, so numbers are never reused.
    last_thread INTEGER NOT NULL DEFAULT 0,
    last_task INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

-- An admin manages the group; a member also asks the workshop for work; a
-- guest reads and converses.
CREATE TABLE memberships (
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('admin', 'member', 'guest')),
    joined_at INTEGER NOT NULL,
    PRIMARY KEY (group_id, user_id)
);

CREATE INDEX memberships_user ON memberships (user_id);

-- An invitation grants its role to whoever accepts it.
CREATE TABLE invitations (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    role TEXT NOT NULL CHECK (role IN ('admin', 'member', 'guest')),
    created_by INTEGER NOT NULL REFERENCES users (id),
    created_at INTEGER NOT NULL,
    expires_at INTEGER,
    max_uses INTEGER,
    uses INTEGER NOT NULL DEFAULT 0,
    revoked_at INTEGER
);

CREATE INDEX invitations_group ON invitations (group_id);
