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
