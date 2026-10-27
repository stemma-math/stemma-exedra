-- The forum: threads, their messages and the files attached to them.

CREATE TABLE threads (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    number INTEGER NOT NULL,
    title TEXT NOT NULL,
    title_ids TEXT,
    kind TEXT NOT NULL CHECK (kind IN ('discussion', 'chat', 'request')),
    -- The agent that answers every message of a chat.
    resident_agent TEXT,
    resident_runner TEXT,
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
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads (id) ON DELETE CASCADE,
    number INTEGER NOT NULL,
    -- Workshop messages have their own author, represented by orchestrator_model.
    author_id INTEGER REFERENCES users (id),
    -- The agent that wrote it for its author, if one did.
    agent TEXT,
    workshop INTEGER NOT NULL DEFAULT 0,
    orchestrator_model TEXT,
    agent_runner TEXT,
    body TEXT NOT NULL,
    body_ids TEXT,
    reply_to INTEGER,
    created_at INTEGER NOT NULL,
    edited_at INTEGER,
    deleted_at INTEGER,
    UNIQUE (thread_id, number)
);

CREATE TABLE attachments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    -- Unset between the upload and the message that carries it.
    message_id INTEGER REFERENCES messages (id) ON DELETE CASCADE,
    process_work INTEGER,
    uploader_id INTEGER NOT NULL REFERENCES users (id),
    name TEXT NOT NULL,
    content_type TEXT NOT NULL,
    size INTEGER NOT NULL,
    -- The file's name under the data directory's `attachments/`.
    storage_key TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
);

CREATE INDEX attachments_message ON attachments (message_id);

-- The forum's search: an index of thread titles and of messages, kept by
-- triggers as they change.

CREATE VIRTUAL TABLE threads_fts USING fts5 (
    title,
    content = 'threads',
    content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER threads_fts_insert AFTER INSERT ON threads BEGIN
    INSERT INTO threads_fts (rowid, title) VALUES (new.id, new.title);
END;

CREATE TRIGGER threads_fts_delete AFTER DELETE ON threads BEGIN
    INSERT INTO threads_fts (threads_fts, rowid, title) VALUES ('delete', old.id, old.title);
END;

CREATE TRIGGER threads_fts_update AFTER UPDATE OF title ON threads BEGIN
    INSERT INTO threads_fts (threads_fts, rowid, title) VALUES ('delete', old.id, old.title);
    INSERT INTO threads_fts (rowid, title) VALUES (new.id, new.title);
END;

CREATE VIRTUAL TABLE messages_fts USING fts5 (
    body,
    content = 'messages',
    content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER messages_fts_insert AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts (rowid, body) VALUES (new.id, new.body);
END;

CREATE TRIGGER messages_fts_delete AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts (messages_fts, rowid, body) VALUES ('delete', old.id, old.body);
END;

-- A deleted message keeps its row, with no text.
CREATE TRIGGER messages_fts_update AFTER UPDATE OF body ON messages BEGIN
    INSERT INTO messages_fts (messages_fts, rowid, body) VALUES ('delete', old.id, old.body);
    INSERT INTO messages_fts (rowid, body) VALUES (new.id, new.body);
END;
