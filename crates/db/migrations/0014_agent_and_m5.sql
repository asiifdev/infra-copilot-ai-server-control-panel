-- M5: pull-only remote agent tokens/jobs and local .test site registry.

CREATE TABLE agent_tokens (
    id          TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id      TEXT NOT NULL REFERENCES organizations (id),
    node_id     TEXT NOT NULL,
    token_hash  TEXT NOT NULL UNIQUE CHECK (length(token_hash) = 64),
    revoked_at  TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, id),
    FOREIGN KEY (org_id, node_id) REFERENCES nodes (org_id, id)
) STRICT;

CREATE INDEX idx_agent_tokens_org_node ON agent_tokens (org_id, node_id);

CREATE TABLE agent_jobs (
    id            TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id        TEXT NOT NULL REFERENCES organizations (id),
    node_id       TEXT NOT NULL,
    proposal_id   TEXT NOT NULL,
    execution_id  TEXT NOT NULL,
    action_json   TEXT NOT NULL,
    args_hash     TEXT NOT NULL CHECK (length(args_hash) = 64),
    status        TEXT NOT NULL DEFAULT 'queued' CHECK (status IN ('queued', 'running', 'succeeded', 'failed')),
    verified      INTEGER NOT NULL DEFAULT 0 CHECK (verified IN (0, 1)),
    reason        TEXT NOT NULL DEFAULT '' CHECK (length(reason) <= 40),
    output        TEXT NOT NULL DEFAULT '' CHECK (length(output) <= 4000),
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    claimed_at    TEXT,
    finished_at   TEXT,
    UNIQUE (org_id, id),
    UNIQUE (org_id, execution_id),
    FOREIGN KEY (org_id, node_id) REFERENCES nodes (org_id, id),
    FOREIGN KEY (org_id, proposal_id) REFERENCES proposals (org_id, id),
    FOREIGN KEY (org_id, execution_id) REFERENCES executions (org_id, id)
) STRICT;

CREATE INDEX idx_agent_jobs_node_status ON agent_jobs (org_id, node_id, status, created_at);

CREATE TABLE local_sites (
    id            TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id        TEXT NOT NULL REFERENCES organizations (id),
    domain        TEXT NOT NULL,
    upstream_port INTEGER NOT NULL CHECK (upstream_port BETWEEN 1 AND 65535),
    config_path   TEXT NOT NULL,
    status        TEXT NOT NULL CHECK (status IN ('proposed', 'active')),
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, id),
    UNIQUE (org_id, domain)
) STRICT;
