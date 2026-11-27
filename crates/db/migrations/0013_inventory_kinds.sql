-- M1 parity/collector expansion (Sprint 5): allow vhost, cert, database, dns, and check
-- item/subject kinds in `inventory_items` and `findings`, preserving FTS5 search triggers.

DROP TRIGGER IF EXISTS inventory_items_index;
DROP TRIGGER IF EXISTS inventory_items_unindex;
ALTER TABLE inventory_items RENAME TO inventory_items_old;

CREATE TABLE inventory_items (
    org_id       TEXT NOT NULL,
    inventory_id TEXT NOT NULL,
    kind         TEXT NOT NULL CHECK (kind IN (
        'listener', 'process', 'cron', 'env_ref', 'vhost', 'cert', 'database', 'dns', 'check'
    )),
    key          TEXT NOT NULL CHECK (length(key) BETWEEN 1 AND 500),
    data         TEXT NOT NULL CHECK (json_valid(data)),
    seen_via     TEXT NOT NULL CHECK (seen_via IN ('proc', 'systemd', 'docker', 'config')),
    PRIMARY KEY (org_id, inventory_id, kind, key),
    FOREIGN KEY (org_id, inventory_id) REFERENCES inventories (org_id, id) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

INSERT INTO inventory_items (org_id, inventory_id, kind, key, data, seen_via)
SELECT org_id, inventory_id, kind, key, data, seen_via FROM inventory_items_old;
DROP TABLE inventory_items_old;

CREATE TRIGGER inventory_items_index AFTER INSERT ON inventory_items BEGIN
    INSERT INTO search_docs (org_id, kind, src, owner_id, ref, title, body)
    VALUES (new.org_id, 'item', new.inventory_id || '/' || new.kind || '/' || new.key,
            new.inventory_id, 'item:' || new.kind || ':' || new.key, new.key, new.data);
END;
CREATE TRIGGER inventory_items_unindex AFTER DELETE ON inventory_items BEGIN
    DELETE FROM search_docs
    WHERE org_id = old.org_id AND kind = 'item'
      AND src = old.inventory_id || '/' || old.kind || '/' || old.key;
END;

DROP TRIGGER IF EXISTS findings_index;
DROP TRIGGER IF EXISTS findings_unindex;
ALTER TABLE findings RENAME TO findings_old;

CREATE TABLE findings (
    org_id        TEXT NOT NULL,
    comparison_id TEXT NOT NULL,
    seq           INTEGER NOT NULL CHECK (seq >= 0),
    rule_id       TEXT NOT NULL CHECK (length(rule_id) BETWEEN 1 AND 50),
    severity      TEXT NOT NULL CHECK (severity IN ('info', 'important', 'critical')),
    subject_kind  TEXT NOT NULL CHECK (subject_kind IN (
        'listener', 'process', 'cron', 'env_ref', 'vhost', 'cert', 'database', 'dns', 'check'
    )),
    subject_key   TEXT NOT NULL CHECK (length(subject_key) BETWEEN 1 AND 500),
    evidence      TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(evidence)),
    args          TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(args)),
    status        TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new', 'accepted', 'fixed')),
    PRIMARY KEY (org_id, comparison_id, seq),
    FOREIGN KEY (org_id, comparison_id) REFERENCES comparisons (org_id, id) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

INSERT INTO findings (org_id, comparison_id, seq, rule_id, severity, subject_kind, subject_key, evidence, args, status)
SELECT org_id, comparison_id, seq, rule_id, severity, subject_kind, subject_key, evidence, args, status FROM findings_old;
DROP TABLE findings_old;

CREATE TRIGGER findings_index AFTER INSERT ON findings BEGIN
    INSERT INTO search_docs (org_id, kind, src, owner_id, ref, title, body)
    VALUES (new.org_id, 'finding', new.comparison_id || '/' || new.seq, new.comparison_id,
            'finding:' || new.rule_id || ':' || new.subject_kind || ':' || new.subject_key,
            new.rule_id || ' ' || new.subject_key, new.subject_kind || ' ' || new.args);
END;
CREATE TRIGGER findings_unindex AFTER DELETE ON findings BEGIN
    DELETE FROM search_docs
    WHERE org_id = old.org_id AND kind = 'finding' AND src = old.comparison_id || '/' || old.seq;
END;
