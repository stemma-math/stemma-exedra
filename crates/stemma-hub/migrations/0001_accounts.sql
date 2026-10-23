-- Accounts, their tokens, groups, their members and invitations. Times are
-- milliseconds since the Unix epoch, in UTC.

CREATE TABLE users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    -- Administers the hub: creates groups.
    hub_admin INTEGER NOT NULL DEFAULT 0,
    -- Who they are to the forge, as the subject of OpenID Connect: random,
    -- so that it is never anyone else's, even after an account is gone.
    subject TEXT NOT NULL UNIQUE,
    -- The login of their account in the forge, once the hub has made it.
    forge_login TEXT UNIQUE,
    -- What they accept as notes: how many an hour, none for no limit, and
    -- whether from agents too.
    notes_per_hour INTEGER,
    notes_from_agents INTEGER NOT NULL DEFAULT 1,
    -- How long they keep read notifications, in days; none for always.
    keep_read_days INTEGER,
    created_at INTEGER NOT NULL,
    -- Set when the account is deleted. Its row stays, for what it wrote,
    -- with a username no one can have and no password.
    deleted_at INTEGER
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
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    last_thread INTEGER NOT NULL DEFAULT 0,
    last_task INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

-- Former names belong to the same identity until it is deleted.
CREATE TABLE user_names (
    name TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE
);
CREATE TABLE group_names (
    name TEXT PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE
);
-- Current and former names share Forgejo's account/organization namespace.
CREATE TRIGGER users_name_insert BEFORE INSERT ON users BEGIN
    SELECT RAISE(ABORT, 'UNIQUE name') WHERE EXISTS (SELECT 1 FROM user_names WHERE name = NEW.username)
        OR EXISTS (SELECT 1 FROM group_names WHERE name = NEW.username)
        OR EXISTS (SELECT 1 FROM groups WHERE slug = NEW.username);
END;
CREATE TRIGGER users_name_update BEFORE UPDATE OF username ON users BEGIN
    SELECT RAISE(ABORT, 'UNIQUE name') WHERE EXISTS (SELECT 1 FROM user_names WHERE name = NEW.username AND user_id != NEW.id)
        OR EXISTS (SELECT 1 FROM group_names WHERE name = NEW.username)
        OR EXISTS (SELECT 1 FROM groups WHERE slug = NEW.username);
END;
CREATE TRIGGER groups_name_insert BEFORE INSERT ON groups BEGIN
    SELECT RAISE(ABORT, 'UNIQUE name') WHERE EXISTS (SELECT 1 FROM group_names WHERE name = NEW.slug)
        OR EXISTS (SELECT 1 FROM user_names WHERE name = NEW.slug)
        OR EXISTS (SELECT 1 FROM users WHERE username = NEW.slug);
END;
CREATE TRIGGER groups_name_update BEFORE UPDATE OF slug ON groups BEGIN
    SELECT RAISE(ABORT, 'UNIQUE name') WHERE EXISTS (SELECT 1 FROM group_names WHERE name = NEW.slug AND group_id != NEW.id)
        OR EXISTS (SELECT 1 FROM user_names WHERE name = NEW.slug)
        OR EXISTS (SELECT 1 FROM users WHERE username = NEW.slug);
END;

-- Requests remain visible to their requester after a decision.
CREATE TABLE group_requests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users (id),
    slug TEXT NOT NULL,
    name TEXT NOT NULL,
    purpose TEXT NOT NULL,
    purpose_ids TEXT,
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'approved', 'declined')),
    created_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX group_requests_open ON group_requests (user_id, slug) WHERE status = 'open';

CREATE TABLE memberships (
    group_id INTEGER NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('admin', 'member', 'guest')),
    joined_at INTEGER NOT NULL,
    PRIMARY KEY (group_id, user_id)
);

CREATE INDEX memberships_user ON memberships (user_id);

-- A link that lets people in. A group's adds an existing account to the
-- group, with its role; the hub's, with no group, makes an account.
CREATE TABLE invitations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id INTEGER REFERENCES groups (id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    role TEXT CHECK (role IN ('admin', 'member', 'guest')),
    created_by INTEGER NOT NULL REFERENCES users (id),
    created_at INTEGER NOT NULL,
    expires_at INTEGER,
    max_uses INTEGER,
    uses INTEGER NOT NULL DEFAULT 0,
    revoked_at INTEGER,
    CHECK ((group_id IS NULL) = (role IS NULL))
);

CREATE INDEX invitations_group ON invitations (group_id);

-- The hub as an OpenID Connect provider: the codes it hands a client that
-- signs someone in, and the access tokens they are exchanged for. Secrets
-- are stored as the hex SHA-256 of what the client holds.

-- A code is used once, shortly after it is made. Once used, it is kept
-- until it expires, so that using it again revokes what it gave.
CREATE TABLE oidc_codes (
    code_hash TEXT PRIMARY KEY,
    client_id TEXT NOT NULL,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    redirect_uri TEXT NOT NULL,
    -- The scopes granted, separated by spaces.
    scope TEXT NOT NULL,
    nonce TEXT,
    -- PKCE's S256 challenge, if the client sent one.
    code_challenge TEXT,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    used_at INTEGER
);

CREATE TABLE oidc_tokens (
    token_hash TEXT PRIMARY KEY,
    code_hash TEXT NOT NULL,
    client_id TEXT NOT NULL,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    scope TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER
);

CREATE INDEX oidc_tokens_code ON oidc_tokens (code_hash);
