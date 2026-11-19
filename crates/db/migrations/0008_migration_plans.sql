-- M3-a migration plans and their steps (docs/design.md 3.4, 6).
-- Plans reference nodes by (org_id, id), so a plan can never point at another org's node, and
-- steps reference their plan the same way. The spec and the step documents are opaque JSON text
-- here: the model and its rules live in the `plan` crate, which `db` does not depend on.
-- The transport a plan uses stays inside the spec JSON; there is deliberately no foreign key to a
-- transports table (another track creates it).

CREATE TABLE migration_plans (
    id             TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id         TEXT NOT NULL REFERENCES organizations (id),
    name           TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 200),
    source_node_id TEXT NOT NULL,
    target_node_id TEXT NOT NULL,
    status         TEXT NOT NULL DEFAULT 'draft'
                   CHECK (status IN ('draft', 'reviewed', 'approved', 'running', 'paused', 'done', 'aborted')),
    spec           TEXT NOT NULL CHECK (json_valid(spec) AND length(spec) <= 262144),
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, id),
    CHECK (source_node_id <> target_node_id),
    -- No cascade: a node that a plan still points at cannot be deleted by accident.
    FOREIGN KEY (org_id, source_node_id) REFERENCES nodes (org_id, id),
    FOREIGN KEY (org_id, target_node_id) REFERENCES nodes (org_id, id)
) STRICT;
CREATE INDEX migration_plans_org_created ON migration_plans (org_id, created_at DESC);

CREATE TABLE migration_steps (
    org_id     TEXT NOT NULL,
    plan_id    TEXT NOT NULL,
    seq        INTEGER NOT NULL CHECK (seq >= 0),
    kind       TEXT NOT NULL
               CHECK (kind IN ('inventory', 'backup', 'transfer', 'restore', 'verify', 'delta',
                               'cutover', 'post_verify', 'decommission_source')),
    spec       TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(spec) AND length(spec) <= 262144),
    status     TEXT NOT NULL DEFAULT 'pending'
               CHECK (status IN ('pending', 'proposed', 'approved', 'running', 'succeeded', 'failed',
                                 'skipped', 'rolled_back')),
    depends_on TEXT NOT NULL DEFAULT '[]'
               CHECK (CASE WHEN json_valid(depends_on) THEN json_type(depends_on) = 'array' ELSE 0 END),
    PRIMARY KEY (org_id, plan_id, seq),
    FOREIGN KEY (org_id, plan_id) REFERENCES migration_plans (org_id, id) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

-- Defense in depth for the tier-4 step: shutting the old server down may not even be proposed
-- until every other step of the plan has succeeded (an optional delta may be skipped). The `plan`
-- crate checks the gates (open critical findings); this keeps the order true even for a caller
-- that bypasses it.
CREATE TRIGGER migration_steps_decommission_insert
BEFORE INSERT ON migration_steps
WHEN NEW.kind = 'decommission_source' AND NEW.status <> 'pending'
BEGIN
    SELECT RAISE(ABORT, 'decommission_source must start pending');
END;

CREATE TRIGGER migration_steps_decommission_status
BEFORE UPDATE OF status ON migration_steps
WHEN NEW.kind = 'decommission_source'
     AND NEW.status IN ('proposed', 'approved', 'running', 'succeeded')
     AND EXISTS (
         SELECT 1 FROM migration_steps other
         WHERE other.org_id = NEW.org_id AND other.plan_id = NEW.plan_id AND other.seq <> NEW.seq
           AND NOT (other.status = 'succeeded' OR (other.kind = 'delta' AND other.status = 'skipped'))
     )
BEGIN
    SELECT RAISE(ABORT, 'decommission_source needs every other step to have succeeded');
END;
