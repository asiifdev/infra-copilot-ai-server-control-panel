-- M0 identity and tenancy (docs/design.md 3.1, adapted for SQLite; see docs/decisions/0009).
-- Ids are UUID text, times are ISO-8601 UTC text, json is TEXT guarded by json_valid().

CREATE TABLE organizations (
    id         TEXT PRIMARY KEY CHECK (length(id) = 36),
    name       TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    slug       TEXT NOT NULL UNIQUE
               CHECK (length(slug) BETWEEN 1 AND 63 AND slug NOT GLOB '*[^a-z0-9-]*'),
    plan       TEXT NOT NULL DEFAULT 'free' CHECK (length(plan) BETWEEN 1 AND 50),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE users (
    id            TEXT PRIMARY KEY CHECK (length(id) = 36),
    email         TEXT NOT NULL UNIQUE CHECK (email = lower(email) AND length(email) BETWEEN 3 AND 320),
    password_hash TEXT,
    name          TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE memberships (
    id         TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id     TEXT NOT NULL REFERENCES organizations (id),
    user_id    TEXT NOT NULL REFERENCES users (id),
    role       TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'operator', 'viewer')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, user_id)
) STRICT;
CREATE INDEX memberships_user ON memberships (user_id);

CREATE TABLE api_tokens (
    id         TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id     TEXT NOT NULL REFERENCES organizations (id),
    user_id    TEXT NOT NULL REFERENCES users (id),
    name       TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    token_hash TEXT NOT NULL UNIQUE,
    scopes     TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(scopes)),
    expires_at TEXT CHECK (expires_at IS NULL OR datetime(expires_at) IS NOT NULL),
    revoked_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- A token only lives while its owner is a member of the org: removing the member deletes it.
    FOREIGN KEY (org_id, user_id) REFERENCES memberships (org_id, user_id) ON DELETE CASCADE
) STRICT;
CREATE INDEX api_tokens_org_user ON api_tokens (org_id, user_id);
