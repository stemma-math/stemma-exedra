-- What people are told: mentions, their agents' answers, notes, and what
-- happens to their groups. While unread, one row gathers the same kind in
-- the same thread.

CREATE TABLE notifications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (
        kind IN (
            'mention', 'answer', 'held', 'failed', 'note',
            'role', 'removed', 'approved', 'declined', 'requested'
        )
    ),
    group_id INTEGER REFERENCES groups (id) ON DELETE CASCADE,
    -- Unset once the thread is deleted: the notification says it is gone.
    thread_id INTEGER REFERENCES threads (id) ON DELETE SET NULL,
    -- The latest message it is about.
    message INTEGER,
    count INTEGER NOT NULL DEFAULT 1,
    actor_id INTEGER REFERENCES users (id),
    agent TEXT,
    -- A note's stored text, or what it is about: a role, a group's name.
    text TEXT,
    -- The token that sent a note, to limit how many it sends.
    token_id INTEGER REFERENCES api_tokens (id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    read_at INTEGER
);

CREATE INDEX notifications_user ON notifications (user_id, id DESC);
CREATE INDEX notifications_unread ON notifications (user_id, thread_id, kind) WHERE read_at IS NULL;
CREATE INDEX notifications_sender ON notifications (actor_id, created_at) WHERE kind = 'note';
