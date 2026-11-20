-- M2-e organization memories and full-text search (docs/design.md 3.5, 7.4; ADR 0026).
-- Every row carries org_id and every query filters on it. Child rows reference (org_id, id) of
-- their parent, so the schema refuses cross-org links.

-- A memory is a short note the assistant may be shown. One written by the assistant starts
-- `pending` and is NEVER shown to a model or indexed until a person activates it.
CREATE TABLE memories (
    id              TEXT PRIMARY KEY CHECK (length(id) = 36),
    org_id          TEXT NOT NULL REFERENCES organizations (id),
    -- Set only for a personal preference (the person it belongs to); NULL for org-wide notes.
    user_id         TEXT REFERENCES users (id),
    kind            TEXT NOT NULL CHECK (kind IN ('org_fact', 'task', 'preference')),
    -- Already redacted by the caller; capped here as well.
    content         TEXT NOT NULL CHECK (length(content) BETWEEN 1 AND 1000),
    source          TEXT NOT NULL CHECK (source IN ('person', 'assistant')),
    status          TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'active')),
    conversation_id TEXT,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK ((kind = 'preference') = (user_id IS NOT NULL)),
    UNIQUE (org_id, id),
    FOREIGN KEY (org_id, conversation_id) REFERENCES conversations (org_id, id)
) STRICT;
CREATE INDEX memories_by_status ON memories (org_id, status, updated_at);
CREATE INDEX memories_by_conversation ON memories (org_id, conversation_id);

-- The assistant can never create an active memory: only a person's approval activates one.
CREATE TRIGGER memories_assistant_starts_pending BEFORE INSERT ON memories
WHEN new.source = 'assistant' AND new.status <> 'pending'
BEGIN
    SELECT RAISE(ABORT, 'assistant memories start pending');
END;

-- Ownership never changes.
CREATE TRIGGER memories_identity_is_fixed BEFORE UPDATE OF id, org_id, user_id, source ON memories
BEGIN
    SELECT RAISE(ABORT, 'memory identity is fixed');
END;

-- Deleting a conversation must not delete or block on the notes it produced.
CREATE TRIGGER memories_conversation_gone BEFORE DELETE ON conversations
BEGIN
    UPDATE memories SET conversation_id = NULL
    WHERE org_id = old.org_id AND conversation_id = old.id;
END;

-- Search documents: one row per indexed thing, derived by triggers from the source tables so the
-- index cannot drift. `src` identifies the source row; `owner_id` its parent (inventory,
-- comparison or the memory itself); `ref` is the stable reference shown to a model.
CREATE TABLE search_docs (
    id       INTEGER PRIMARY KEY,
    org_id   TEXT NOT NULL REFERENCES organizations (id),
    kind     TEXT NOT NULL CHECK (kind IN ('item', 'finding', 'memory')),
    src      TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    ref      TEXT NOT NULL,
    title    TEXT NOT NULL,
    body     TEXT NOT NULL,
    UNIQUE (org_id, kind, src)
) STRICT;
CREATE INDEX search_docs_owner ON search_docs (org_id, kind, owner_id);

-- External-content FTS5 table over search_docs. unicode61 with remove_diacritics 2 folds case
-- and accents ("cafe" finds "cafe" with an accent) and splits on punctuation, so "0.0.0.0:8080"
-- is found by "8080" and "/usr/bin/node" by "node".
CREATE VIRTUAL TABLE search_fts USING fts5(
    title, body,
    content = 'search_docs', content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER search_docs_ai AFTER INSERT ON search_docs BEGIN
    INSERT INTO search_fts (rowid, title, body) VALUES (new.id, new.title, new.body);
END;
CREATE TRIGGER search_docs_ad AFTER DELETE ON search_docs BEGIN
    INSERT INTO search_fts (search_fts, rowid, title, body)
    VALUES ('delete', old.id, old.title, old.body);
END;
CREATE TRIGGER search_docs_no_update BEFORE UPDATE ON search_docs BEGIN
    SELECT RAISE(ABORT, 'search_docs is derived data');
END;

-- Existing data, indexed once; later changes arrive through the triggers below.
INSERT INTO search_docs (org_id, kind, src, owner_id, ref, title, body)
SELECT org_id, 'item', inventory_id || '/' || kind || '/' || key, inventory_id,
       'item:' || kind || ':' || key, key, data
FROM inventory_items;
INSERT INTO search_docs (org_id, kind, src, owner_id, ref, title, body)
SELECT org_id, 'finding', comparison_id || '/' || seq, comparison_id,
       'finding:' || rule_id || ':' || subject_kind || ':' || subject_key,
       rule_id || ' ' || subject_key, subject_kind || ' ' || args
FROM findings;

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

-- Only ACTIVE, org-wide memories are searchable. Pending ones (not yet approved by a person) and
-- personal preferences stay out of the index.
CREATE TRIGGER memories_index AFTER INSERT ON memories
WHEN new.status = 'active' AND new.kind <> 'preference'
BEGIN
    INSERT INTO search_docs (org_id, kind, src, owner_id, ref, title, body)
    VALUES (new.org_id, 'memory', new.id, new.id, 'memory:' || new.id, new.kind, new.content);
END;
CREATE TRIGGER memories_reindex AFTER UPDATE OF status, content, kind ON memories
BEGIN
    DELETE FROM search_docs WHERE org_id = old.org_id AND kind = 'memory' AND src = old.id;
    INSERT INTO search_docs (org_id, kind, src, owner_id, ref, title, body)
    SELECT new.org_id, 'memory', new.id, new.id, 'memory:' || new.id, new.kind, new.content
    WHERE new.status = 'active' AND new.kind <> 'preference';
END;
CREATE TRIGGER memories_unindex AFTER DELETE ON memories BEGIN
    DELETE FROM search_docs WHERE org_id = old.org_id AND kind = 'memory' AND src = old.id;
END;
