-- The forum's search: an index of thread titles and of messages, kept by
-- triggers as they change; and how a chat's resident agent answers.

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

INSERT INTO threads_fts (threads_fts) VALUES ('rebuild');
INSERT INTO messages_fts (messages_fts) VALUES ('rebuild');

-- A chat's resident agent answers with the model and the effort chosen
-- when it started, when the writer's agent runs with the same runner.
ALTER TABLE threads ADD COLUMN resident_runner TEXT;
ALTER TABLE threads ADD COLUMN resident_model TEXT;
ALTER TABLE threads ADD COLUMN resident_effort TEXT
    CHECK (resident_effort IN ('low', 'medium', 'high'));
