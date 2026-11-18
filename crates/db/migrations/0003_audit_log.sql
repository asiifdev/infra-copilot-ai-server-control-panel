-- Append-only, hash-chained audit trail (docs/design.md 3.6). The chain is computed in Rust.
-- node_id has no foreign key on purpose: history must outlive the node.

CREATE TABLE audit_log (
    id         TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id     TEXT NOT NULL REFERENCES organizations (id),
    seq        INTEGER NOT NULL CHECK (seq >= 1),
    actor_kind TEXT NOT NULL CHECK (actor_kind IN ('user', 'agent', 'api_token', 'system')),
    actor_id   TEXT,
    action     TEXT NOT NULL CHECK (length(action) BETWEEN 1 AND 200),
    node_id    TEXT,
    subject    TEXT NOT NULL CHECK (json_valid(subject)),
    result     TEXT NOT NULL CHECK (result IN ('success', 'failure', 'denied')),
    created_at TEXT NOT NULL,
    prev_hash  TEXT NOT NULL CHECK (length(prev_hash) = 64),
    hash       TEXT NOT NULL CHECK (length(hash) = 64),
    UNIQUE (org_id, seq),
    UNIQUE (org_id, prev_hash)
) STRICT;
CREATE INDEX audit_log_org_node ON audit_log (org_id, node_id);

CREATE TRIGGER audit_log_no_update BEFORE UPDATE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only');
END;

CREATE TRIGGER audit_log_no_delete BEFORE DELETE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only');
END;

-- Enforces the chain shape in SQL as well. A BEFORE INSERT trigger fires before SQLite resolves a
-- conflict, so INSERT OR REPLACE cannot silently overwrite an existing row.
CREATE TRIGGER audit_log_chain_shape BEFORE INSERT ON audit_log
WHEN NEW.seq IS NOT COALESCE((SELECT max(seq) FROM audit_log WHERE org_id = NEW.org_id), 0) + 1
  OR (NEW.seq > 1 AND NEW.prev_hash IS NOT
      (SELECT hash FROM audit_log WHERE org_id = NEW.org_id AND seq = NEW.seq - 1))
BEGIN
    SELECT RAISE(ABORT, 'audit_log entries must extend the chain');
END;
