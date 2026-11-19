-- M2-b inventories, comparisons and findings (docs/design.md 3.3).
-- Every child row references (org_id, id) of its parent, so a row can never point across orgs.

CREATE TABLE inventories (
    id                TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id            TEXT NOT NULL REFERENCES organizations (id),
    node_id           TEXT NOT NULL,
    taken_at          TEXT NOT NULL CHECK (length(taken_at) BETWEEN 1 AND 40),
    collector_version TEXT NOT NULL CHECK (length(collector_version) BETWEEN 1 AND 100),
    status            TEXT NOT NULL CHECK (status IN ('complete', 'partial')),
    os                TEXT NOT NULL CHECK (length(os) BETWEEN 1 AND 100),
    arch              TEXT NOT NULL CHECK (length(arch) BETWEEN 1 AND 100),
    warnings          TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(warnings)),
    created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, id),
    FOREIGN KEY (org_id, node_id) REFERENCES nodes (org_id, id) ON DELETE CASCADE
) STRICT;
CREATE INDEX inventories_org_node_taken ON inventories (org_id, node_id, taken_at DESC);

-- Items hold names, ports and flags only; collectors never store secret values (decision 0015).
CREATE TABLE inventory_items (
    org_id       TEXT NOT NULL,
    inventory_id TEXT NOT NULL,
    kind         TEXT NOT NULL CHECK (kind IN ('listener', 'process', 'cron', 'env_ref')),
    key          TEXT NOT NULL CHECK (length(key) BETWEEN 1 AND 500),
    data         TEXT NOT NULL CHECK (json_valid(data)),
    seen_via     TEXT NOT NULL CHECK (seen_via IN ('proc', 'systemd', 'docker', 'config')),
    PRIMARY KEY (org_id, inventory_id, kind, key),
    FOREIGN KEY (org_id, inventory_id) REFERENCES inventories (org_id, id) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

CREATE TABLE comparisons (
    id                  TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id              TEXT NOT NULL REFERENCES organizations (id),
    source_inventory_id TEXT NOT NULL,
    target_inventory_id TEXT NOT NULL,
    created_at          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, id),
    FOREIGN KEY (org_id, source_inventory_id) REFERENCES inventories (org_id, id) ON DELETE CASCADE,
    FOREIGN KEY (org_id, target_inventory_id) REFERENCES inventories (org_id, id) ON DELETE CASCADE
) STRICT;
CREATE INDEX comparisons_org_created ON comparisons (org_id, created_at DESC);

CREATE TABLE findings (
    org_id        TEXT NOT NULL,
    comparison_id TEXT NOT NULL,
    seq           INTEGER NOT NULL CHECK (seq >= 0),
    rule_id       TEXT NOT NULL CHECK (length(rule_id) BETWEEN 1 AND 50),
    severity      TEXT NOT NULL CHECK (severity IN ('info', 'important', 'critical')),
    subject_kind  TEXT NOT NULL CHECK (subject_kind IN ('listener', 'process', 'cron', 'env_ref')),
    subject_key   TEXT NOT NULL CHECK (length(subject_key) BETWEEN 1 AND 500),
    evidence      TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(evidence)),
    args          TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(args)),
    status        TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new', 'accepted', 'fixed')),
    PRIMARY KEY (org_id, comparison_id, seq),
    FOREIGN KEY (org_id, comparison_id) REFERENCES comparisons (org_id, id) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
