-- M2-c org-level sealed secrets and AI providers (docs/design.md 3.1 and 3.5).
-- Only sealed bytes are stored; encryption happens in the `vault` crate (ADR 0021).

CREATE TABLE secrets (
    id         TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id     TEXT NOT NULL REFERENCES organizations (id),
    purpose    TEXT NOT NULL CHECK (length(purpose) BETWEEN 1 AND 100),
    ciphertext BLOB NOT NULL CHECK (length(ciphertext) > 0),
    key_id     TEXT NOT NULL CHECK (length(key_id) BETWEEN 1 AND 100),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, id)
) STRICT;

CREATE TABLE ai_providers (
    id            TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id        TEXT NOT NULL REFERENCES organizations (id),
    name          TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    base_url      TEXT NOT NULL CHECK (length(base_url) BETWEEN 1 AND 500),
    default_model TEXT NOT NULL CHECK (length(default_model) BETWEEN 1 AND 200),
    -- NULL for a provider that needs no key (a local model).
    secret_id     TEXT,
    params        TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(params)),
    is_default    INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, id),
    UNIQUE (org_id, name),
    FOREIGN KEY (org_id, secret_id) REFERENCES secrets (org_id, id)
) STRICT;
-- At most one default provider per org.
CREATE UNIQUE INDEX ai_providers_one_default ON ai_providers (org_id) WHERE is_default = 1;
