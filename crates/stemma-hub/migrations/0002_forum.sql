-- The forum: threads, their messages and the files attached to them.

CREATE TABLE threads (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    number INTEGER NOT NULL,
    title TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('discussion', 'chat')),
    -- The agent that answers every message of a chat.
    resident_agent TEXT,
    -- What the thread is about: a declaration's address, `lean:Foo.bar`.
    anchor TEXT,
    created_by INTEGER NOT NULL REFERENCES users (id),
    created_at INTEGER NOT NULL,
    last_activity_at INTEGER NOT NULL,
    -- The number of the last message posted, so numbers are never reused.
    last_message INTEGER NOT NULL DEFAULT 0,
    UNIQUE (group_id, number)
);

CREATE INDEX threads_activity ON threads (group_id, last_activity_at DESC);
CREATE INDEX threads_anchor ON threads (group_id, anchor);

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
