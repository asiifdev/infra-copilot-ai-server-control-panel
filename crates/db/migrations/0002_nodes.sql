-- M0 nodes, agents and sealed secrets (docs/design.md 3.2).
-- Child tables reference (org_id, id) of nodes, so a row can never point at another org's node.

CREATE TABLE nodes (
    id         TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id     TEXT NOT NULL REFERENCES organizations (id),
    name       TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    kind       TEXT NOT NULL CHECK (kind IN ('vps', 'local')),
    access     TEXT NOT NULL CHECK (access IN ('agent', 'ssh', 'local')),
    status     TEXT NOT NULL DEFAULT 'pending'
               CHECK (status IN ('pending', 'online', 'offline', 'revoked')),
    labels     TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(labels)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, id),
    UNIQUE (org_id, name)
) STRICT;
CREATE INDEX nodes_org_status ON nodes (org_id, status);

CREATE TABLE agents (
    id           TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id       TEXT NOT NULL REFERENCES organizations (id),
    node_id      TEXT NOT NULL,
    token_hash   TEXT NOT NULL UNIQUE,
    version      TEXT NOT NULL,
    os           TEXT NOT NULL,
    arch         TEXT NOT NULL,
    last_seen_at TEXT,
    revoked_at   TEXT,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (org_id, node_id) REFERENCES nodes (org_id, id) ON DELETE CASCADE
) STRICT;
CREATE INDEX agents_org_node ON agents (org_id, node_id);

-- Only sealed bytes are stored. Encryption itself is not implemented yet (decision 0009, open).
CREATE TABLE node_secrets (
    id         TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id     TEXT NOT NULL REFERENCES organizations (id),
    node_id    TEXT NOT NULL,
    purpose    TEXT NOT NULL CHECK (length(purpose) BETWEEN 1 AND 100),
    ciphertext BLOB NOT NULL CHECK (length(ciphertext) > 0),
    key_id     TEXT NOT NULL CHECK (length(key_id) BETWEEN 1 AND 100),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (org_id, node_id) REFERENCES nodes (org_id, id) ON DELETE CASCADE
) STRICT;
CREATE INDEX node_secrets_org_node ON node_secrets (org_id, node_id, purpose);
