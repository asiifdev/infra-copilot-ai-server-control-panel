-- 2-approver governance, multi-role AI provider models, open-core licenses/usage counters,
-- M5 local_sites PHP/TLS metadata, and DNS snapshots.

ALTER TABLE organizations ADD COLUMN require_two_approvers INTEGER NOT NULL DEFAULT 0
    CHECK (require_two_approvers IN (0, 1));

CREATE TABLE ai_provider_models (
    id          TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id      TEXT NOT NULL REFERENCES organizations (id),
    provider_id TEXT NOT NULL,
    model       TEXT NOT NULL CHECK (length(model) BETWEEN 1 AND 200),
    role        TEXT NOT NULL CHECK (role IN ('chat', 'planner', 'summarizer', 'embedding')),
    params_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(params_json)),
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, id),
    FOREIGN KEY (org_id, provider_id) REFERENCES ai_providers (org_id, id) ON DELETE CASCADE
) STRICT;
CREATE INDEX idx_ai_provider_models_org_provider ON ai_provider_models (org_id, provider_id);

CREATE TABLE licenses (
    id          TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id      TEXT NOT NULL REFERENCES organizations (id),
    key_id      TEXT NOT NULL CHECK (length(key_id) BETWEEN 1 AND 100),
    tier        TEXT NOT NULL CHECK (length(tier) BETWEEN 1 AND 50),
    limits_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(limits_json)),
    valid_from  TEXT NOT NULL CHECK (datetime(valid_from) IS NOT NULL),
    valid_to    TEXT CHECK (valid_to IS NULL OR datetime(valid_to) IS NOT NULL),
    instance_id TEXT CHECK (instance_id IS NULL OR length(instance_id) BETWEEN 1 AND 128),
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, id)
) STRICT;
CREATE INDEX idx_licenses_org ON licenses (org_id, created_at);

CREATE TABLE usage_counters (
    id         TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id     TEXT NOT NULL REFERENCES organizations (id),
    metric     TEXT NOT NULL CHECK (length(metric) BETWEEN 1 AND 100),
    period     TEXT NOT NULL CHECK (length(period) BETWEEN 1 AND 40),
    value      INTEGER NOT NULL DEFAULT 0 CHECK (value >= 0),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, id),
    UNIQUE (org_id, metric, period)
) STRICT;

ALTER TABLE local_sites ADD COLUMN php_version TEXT
    CHECK (php_version IS NULL OR php_version IN ('8.1', '8.2', '8.3', '8.4'));
ALTER TABLE local_sites ADD COLUMN tls_enabled INTEGER NOT NULL DEFAULT 0
    CHECK (tls_enabled IN (0, 1));

CREATE TABLE dns_snapshots (
    id           TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id       TEXT NOT NULL REFERENCES organizations (id),
    zone_id      TEXT NOT NULL CHECK (length(zone_id) BETWEEN 1 AND 128),
    zone_name    TEXT NOT NULL CHECK (length(zone_name) BETWEEN 1 AND 255),
    records_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(records_json)),
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, id)
) STRICT;
CREATE INDEX idx_dns_snapshots_org_zone ON dns_snapshots (org_id, zone_id, created_at);
