-- M4 (Sprint 1): approvals (with typed confirmation for tier 4), dry-run cache on proposals,
-- pre-execution backups, and execution rollbacks.

CREATE TABLE approvals (
    id                 TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id             TEXT NOT NULL REFERENCES organizations (id),
    proposal_id        TEXT NOT NULL,
    user_id            TEXT NOT NULL REFERENCES users (id),
    decision           TEXT NOT NULL CHECK (decision IN ('approved', 'rejected')),
    typed_confirmation INTEGER NOT NULL DEFAULT 0 CHECK (typed_confirmation IN (0, 1)),
    comment            TEXT CHECK (comment IS NULL OR length(comment) <= 500),
    created_at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, id),
    UNIQUE (org_id, proposal_id, user_id),
    FOREIGN KEY (org_id, proposal_id) REFERENCES proposals (org_id, id)
) STRICT;
CREATE INDEX approvals_by_proposal ON approvals (org_id, proposal_id);

-- Updating dry_run_json / dry_run_at does not touch action_json, node_id, args_hash, tool, or
-- tier, so it never fires `proposals_action_is_immutable`.
ALTER TABLE proposals ADD COLUMN dry_run_json TEXT
    CHECK (dry_run_json IS NULL OR (length(dry_run_json) <= 8000 AND json_valid(dry_run_json)));
ALTER TABLE proposals ADD COLUMN dry_run_at TEXT;

CREATE TABLE execution_backups (
    id           TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id       TEXT NOT NULL REFERENCES organizations (id),
    execution_id TEXT NOT NULL,
    storage_ref  TEXT NOT NULL CHECK (length(storage_ref) BETWEEN 1 AND 500),
    sha256       TEXT NOT NULL CHECK (length(sha256) = 64),
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, id),
    UNIQUE (org_id, execution_id),
    FOREIGN KEY (org_id, execution_id) REFERENCES executions (org_id, id)
) STRICT;

CREATE TABLE execution_rollbacks (
    id           TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id       TEXT NOT NULL REFERENCES organizations (id),
    execution_id TEXT NOT NULL,
    started_by   TEXT NOT NULL REFERENCES users (id),
    status       TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running', 'succeeded', 'failed')),
    verified     INTEGER NOT NULL DEFAULT 0 CHECK (verified IN (0, 1)),
    reason       TEXT CHECK (reason IS NULL OR length(reason) BETWEEN 1 AND 40),
    output       TEXT NOT NULL DEFAULT '' CHECK (length(output) <= 4000),
    started_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    finished_at  TEXT,
    CHECK ((status = 'running') = (finished_at IS NULL)),
    UNIQUE (org_id, id),
    UNIQUE (org_id, execution_id),
    FOREIGN KEY (org_id, execution_id) REFERENCES executions (org_id, id)
) STRICT;
