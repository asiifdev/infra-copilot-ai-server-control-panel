-- M1-b (Sprint 3): monitor targets, incidents, notification channels (with vault-sealed
-- secrets), alert rules with quiet hours, pending alerts, and Telegram chat links.

CREATE TABLE monitor_targets (
    id              TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id          TEXT NOT NULL REFERENCES organizations (id),
    name            TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    kind            TEXT NOT NULL CHECK (kind IN ('http', 'tcp', 'tls')),
    target          TEXT NOT NULL CHECK (length(target) BETWEEN 1 AND 500),
    interval_sec    INTEGER NOT NULL DEFAULT 60 CHECK (interval_sec BETWEEN 10 AND 86400),
    timeout_sec     INTEGER NOT NULL DEFAULT 10 CHECK (timeout_sec BETWEEN 1 AND 60),
    allow_private   INTEGER NOT NULL DEFAULT 0 CHECK (allow_private IN (0, 1)),
    is_active       INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    current_status  TEXT NOT NULL DEFAULT 'unknown'
                    CHECK (current_status IN ('unknown', 'up', 'down', 'degraded')),
    last_checked_at TEXT,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, id),
    UNIQUE (org_id, name)
) STRICT;
CREATE INDEX monitor_targets_by_org ON monitor_targets (org_id, is_active);

CREATE TABLE incidents (
    id              TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id          TEXT NOT NULL REFERENCES organizations (id),
    monitor_id      TEXT NOT NULL,
    started_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    resolved_at     TEXT,
    cause           TEXT NOT NULL CHECK (length(cause) BETWEEN 1 AND 500),
    severity        TEXT NOT NULL CHECK (severity IN ('info', 'warning', 'critical')),
    acknowledged_by TEXT REFERENCES users (id),
    acknowledged_at TEXT,
    UNIQUE (org_id, id),
    FOREIGN KEY (org_id, monitor_id) REFERENCES monitor_targets (org_id, id) ON DELETE CASCADE
) STRICT;
CREATE INDEX incidents_by_org ON incidents (org_id, started_at DESC);

CREATE TABLE notification_channels (
    id          TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id      TEXT NOT NULL REFERENCES organizations (id),
    kind        TEXT NOT NULL CHECK (kind IN ('telegram', 'webhook')),
    name        TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    config_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(config_json)),
    secret_id   TEXT,
    is_active   INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, id),
    UNIQUE (org_id, name),
    FOREIGN KEY (org_id, secret_id) REFERENCES secrets (org_id, id)
) STRICT;

CREATE TABLE alert_rules (
    id           TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id       TEXT NOT NULL REFERENCES organizations (id),
    name         TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    channel_id   TEXT NOT NULL,
    min_severity TEXT NOT NULL DEFAULT 'warning' CHECK (min_severity IN ('info', 'warning', 'critical')),
    quiet_start  TEXT CHECK (quiet_start IS NULL OR length(quiet_start) = 5),
    quiet_end    TEXT CHECK (quiet_end IS NULL OR length(quiet_end) = 5),
    is_active    INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, id),
    UNIQUE (org_id, name),
    FOREIGN KEY (org_id, channel_id) REFERENCES notification_channels (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE TABLE pending_alerts (
    id         TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id     TEXT NOT NULL REFERENCES organizations (id),
    channel_id TEXT NOT NULL,
    title      TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
    body       TEXT NOT NULL CHECK (length(body) BETWEEN 1 AND 4000),
    severity   TEXT NOT NULL CHECK (severity IN ('info', 'warning', 'critical')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    flushed_at TEXT,
    UNIQUE (org_id, id),
    FOREIGN KEY (org_id, channel_id) REFERENCES notification_channels (org_id, id) ON DELETE CASCADE
) STRICT;
CREATE INDEX pending_alerts_unflushed ON pending_alerts (org_id, flushed_at, created_at);

CREATE TABLE telegram_links (
    id             TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id         TEXT NOT NULL REFERENCES organizations (id),
    user_id        TEXT NOT NULL REFERENCES users (id),
    chat_id        TEXT,
    link_code_hash TEXT NOT NULL CHECK (length(link_code_hash) = 64),
    language       TEXT NOT NULL DEFAULT 'en' CHECK (language IN ('en', 'id')),
    status         TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'linked', 'revoked')),
    expires_at     TEXT NOT NULL,
    verified_at    TEXT,
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, id),
    UNIQUE (org_id, chat_id),
    UNIQUE (org_id, link_code_hash)
) STRICT;
