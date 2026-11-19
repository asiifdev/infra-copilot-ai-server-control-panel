-- M2-d assistant conversations, tool call records and proposals (docs/design.md 3.5, 3.6; ADR 0023).
-- Every child row references (org_id, id) of its parent, so the schema refuses cross-org links.

CREATE TABLE conversations (
    id              TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id          TEXT NOT NULL REFERENCES organizations (id),
    user_id         TEXT NOT NULL REFERENCES users (id),
    title           TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
    -- NULL = the org default provider at the time of each turn.
    provider_id     TEXT,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    archived_at     TEXT,
    -- Turn claim: at most one turn runs per conversation. A claim is taken with one atomic UPDATE
    -- and expires when its heartbeat is older than the caller's stale limit.
    turn_token      TEXT,
    turn_claimed_at TEXT,
    CHECK ((turn_token IS NULL) = (turn_claimed_at IS NULL)),
    UNIQUE (org_id, id),
    FOREIGN KEY (org_id, provider_id) REFERENCES ai_providers (org_id, id)
) STRICT;
CREATE INDEX conversations_by_user ON conversations (org_id, user_id, created_at);

CREATE TABLE messages (
    org_id          TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    seq             INTEGER NOT NULL CHECK (seq >= 1),
    role            TEXT NOT NULL CHECK (role IN ('system', 'user', 'assistant', 'tool')),
    -- Already redacted by the caller (docs/design.md 7.2).
    content         TEXT NOT NULL,
    -- JSON array of {id, name, arguments} for an assistant message that calls tools.
    tool_calls      TEXT CHECK (tool_calls IS NULL OR json_valid(tool_calls)),
    tool_call_id    TEXT CHECK (tool_call_id IS NULL OR length(tool_call_id) BETWEEN 1 AND 200),
    token_usage     TEXT CHECK (token_usage IS NULL OR json_valid(token_usage)),
    redactions      INTEGER NOT NULL DEFAULT 0 CHECK (redactions >= 0),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (org_id, conversation_id, seq),
    FOREIGN KEY (org_id, conversation_id) REFERENCES conversations (org_id, id) ON DELETE CASCADE
) STRICT;

CREATE TABLE tool_calls (
    id              TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id          TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    -- The assistant message that asked for the call.
    message_seq     INTEGER NOT NULL,
    idx             INTEGER NOT NULL CHECK (idx >= 0),
    call_id         TEXT NOT NULL CHECK (length(call_id) BETWEEN 1 AND 200),
    tool            TEXT NOT NULL CHECK (length(tool) BETWEEN 1 AND 64),
    -- NULL when the tool does not exist.
    tier            INTEGER CHECK (tier IS NULL OR tier BETWEEN 0 AND 4),
    -- Redacted and capped; never the raw arguments.
    args            TEXT NOT NULL CHECK (length(args) <= 4000),
    status          TEXT NOT NULL CHECK (length(status) BETWEEN 1 AND 64),
    duration_ms     INTEGER NOT NULL CHECK (duration_ms >= 0),
    redactions      INTEGER NOT NULL DEFAULT 0 CHECK (redactions >= 0),
    truncated       INTEGER NOT NULL DEFAULT 0 CHECK (truncated IN (0, 1)),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (org_id, conversation_id, message_seq, idx),
    FOREIGN KEY (org_id, conversation_id, message_seq)
        REFERENCES messages (org_id, conversation_id, seq) ON DELETE CASCADE
) STRICT;

-- A proposal is a record of something the assistant recommends. In M2 nothing approves or
-- executes it. It outlives its conversation's messages, so a conversation with proposals can be
-- archived but not deleted (no ON DELETE action: the foreign key refuses the delete).
CREATE TABLE proposals (
    id               TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id           TEXT NOT NULL REFERENCES organizations (id),
    conversation_id  TEXT,
    tool             TEXT NOT NULL CHECK (length(tool) BETWEEN 1 AND 64),
    tier             INTEGER NOT NULL CHECK (tier BETWEEN 1 AND 4),
    title            TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 120),
    rationale        TEXT NOT NULL CHECK (length(rationale) BETWEEN 1 AND 1000),
    suggested_action TEXT NOT NULL CHECK (length(suggested_action) BETWEEN 1 AND 500),
    impact           TEXT NOT NULL CHECK (length(impact) BETWEEN 1 AND 500),
    rule_id          TEXT CHECK (rule_id IS NULL OR length(rule_id) BETWEEN 1 AND 20),
    subject_key      TEXT CHECK (subject_key IS NULL OR length(subject_key) BETWEEN 1 AND 500),
    args_hash        TEXT NOT NULL CHECK (length(args_hash) = 64),
    status           TEXT NOT NULL DEFAULT 'pending'
                     CHECK (status IN ('pending', 'approved', 'rejected', 'executed', 'failed', 'expired')),
    created_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    expires_at       TEXT NOT NULL,
    UNIQUE (org_id, id),
    FOREIGN KEY (org_id, conversation_id) REFERENCES conversations (org_id, id)
) STRICT;
CREATE INDEX proposals_by_status ON proposals (org_id, status, created_at);
CREATE INDEX proposals_by_conversation ON proposals (org_id, conversation_id);
