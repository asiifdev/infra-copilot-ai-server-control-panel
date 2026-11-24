-- M4 (ADR 0037): the structured action of a proposal and one execution record per proposal.

-- The action is the strict JSON form of the `actions` crate; `args_hash` (set at creation) is the
-- SHA-256 of its canonical form, and the executor re-checks it before running anything. The node
-- is the target. Both are set once, while the proposal is pending, and never change afterwards.
ALTER TABLE proposals ADD COLUMN action_json TEXT
    CHECK (action_json IS NULL OR length(action_json) BETWEEN 2 AND 2000);
ALTER TABLE proposals ADD COLUMN node_id TEXT;

CREATE TRIGGER proposals_action_is_immutable
BEFORE UPDATE OF action_json, node_id, args_hash, tool, tier ON proposals
WHEN OLD.action_json IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'the action of a proposal cannot change');
END;

-- At most one execution per proposal (UNIQUE): a failed execution is never repeated by itself.
CREATE TABLE executions (
    id          TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id      TEXT NOT NULL REFERENCES organizations (id),
    proposal_id TEXT NOT NULL,
    node_id     TEXT NOT NULL,
    started_by  TEXT NOT NULL REFERENCES users (id),
    status      TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running', 'succeeded', 'failed')),
    verified    INTEGER NOT NULL DEFAULT 0 CHECK (verified IN (0, 1)),
    reason      TEXT CHECK (reason IS NULL OR length(reason) BETWEEN 1 AND 40),
    -- Already redacted and bounded by the caller; capped here as well.
    output      TEXT NOT NULL DEFAULT '' CHECK (length(output) <= 4000),
    started_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    finished_at TEXT,
    CHECK ((status = 'running') = (finished_at IS NULL)),
    UNIQUE (org_id, id),
    UNIQUE (org_id, proposal_id),
    FOREIGN KEY (org_id, proposal_id) REFERENCES proposals (org_id, id)
) STRICT;
