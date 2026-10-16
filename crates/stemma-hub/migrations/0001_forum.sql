-- Accounts, groups and the forum. Times are milliseconds since the Unix
-- epoch, in UTC.

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

CREATE TABLE api_tokens (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    prefix TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER,
    revoked_at INTEGER
);

CREATE INDEX api_tokens_user ON api_tokens (user_id);

CREATE TABLE groups (
    id INTEGER PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    -- The number of the last thread created, so numbers are never reused.
    last_thread INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE TABLE memberships (
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('admin', 'member')),
    joined_at INTEGER NOT NULL,
    PRIMARY KEY (group_id, user_id)
);

CREATE INDEX memberships_user ON memberships (user_id);

CREATE TABLE invitations (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    role TEXT NOT NULL CHECK (role IN ('admin', 'member')),
    created_by INTEGER NOT NULL REFERENCES users (id),
    created_at INTEGER NOT NULL,
    expires_at INTEGER,
    max_uses INTEGER,
    uses INTEGER NOT NULL DEFAULT 0,
    revoked_at INTEGER
);

CREATE INDEX invitations_group ON invitations (group_id);

CREATE TABLE threads (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    number INTEGER NOT NULL,
    title TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('discussion', 'chat')),
    resident_agent TEXT,
    anchor TEXT,
    created_by INTEGER NOT NULL REFERENCES users (id),
    created_at INTEGER NOT NULL,
    last_activity_at INTEGER NOT NULL,
    -- The number of the last message posted, so numbers are never reused.
    last_message INTEGER NOT NULL DEFAULT 0,
    UNIQUE (group_id, number)
);

CREATE INDEX threads_activity ON threads (group_id, last_activity_at DESC);

CREATE TABLE messages (
    id INTEGER PRIMARY KEY,
    thread_id INTEGER NOT NULL REFERENCES threads (id) ON DELETE CASCADE,
    number INTEGER NOT NULL,
    author_id INTEGER NOT NULL REFERENCES users (id),
    -- The agent that wrote it for its author, if one did.
    agent TEXT,
    body TEXT NOT NULL,
    reply_to INTEGER,
    created_at INTEGER NOT NULL,
    edited_at INTEGER,
    deleted_at INTEGER,
    UNIQUE (thread_id, number)
);

CREATE TABLE attachments (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    -- Unset between the upload and the message that carries it.
    message_id INTEGER REFERENCES messages (id) ON DELETE CASCADE,
    uploader_id INTEGER NOT NULL REFERENCES users (id),
    name TEXT NOT NULL,
    content_type TEXT NOT NULL,
    size INTEGER NOT NULL,
    -- The file's name under the data directory's `attachments/`.
    storage_key TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
);

CREATE INDEX attachments_message ON attachments (message_id);
