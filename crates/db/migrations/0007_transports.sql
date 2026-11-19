-- M3-b transports (docs/design.md 3.4, ADR 0025). The row never holds credentials: they are
-- sealed in `secrets` by the `vault` crate and referenced through the composite foreign key, so
-- the schema itself refuses a secret that belongs to another org.

CREATE TABLE transports (
    id          TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id      TEXT NOT NULL REFERENCES organizations (id),
    name        TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    kind        TEXT NOT NULL CHECK (kind IN ('rclone', 'rsync', 'scp', 'ssh_pipe', 'managed')),
    -- Non-secret settings only (remote kind, host, path, plain options).
    config      TEXT NOT NULL DEFAULT '{}'
                CHECK (json_valid(config) AND json_type(config) = 'object' AND length(config) <= 8192),
    -- NULL for a transport that needs no credentials.
    secret_id   TEXT,
    verified_at TEXT,
    last_check  TEXT CHECK (last_check IS NULL OR (json_valid(last_check) AND length(last_check) <= 8192)),
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, id),
    UNIQUE (org_id, name),
    FOREIGN KEY (org_id, secret_id) REFERENCES secrets (org_id, id)
) STRICT;
