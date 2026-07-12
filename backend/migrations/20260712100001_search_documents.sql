-- Global Search: a denormalized full-text index over every searchable
-- workspace object, maintained asynchronously by services/search_indexer.rs
-- — NEVER written in a request path. PostgreSQL remains the transactional
-- source of truth; these rows are a rebuildable projection (dropping the
-- table loses nothing that a reconcile pass cannot restore).
--
-- Metadata only: secret VALUES (ciphertext, DEKs, nonces) and log content
-- never land here. Each row carries the RBAC permission a caller must hold
-- before it may match — the query layer filters on it BEFORE ranking, so
-- unauthorized objects never influence scores or leak through counts.
CREATE TABLE search_documents (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id      UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    entity_type       TEXT NOT NULL CHECK (entity_type IN
        ('repository', 'workflow', 'pipeline', 'runner', 'artifact',
         'environment', 'secret', 'activity')),
    entity_id         UUID NOT NULL,
    permission        TEXT NOT NULL DEFAULT 'content.read' CHECK (permission IN
        ('content.read', 'secrets.read', 'audit.read')),
    title             TEXT NOT NULL,               -- weight A
    subtitle          TEXT NOT NULL DEFAULT '',    -- weight B
    body              TEXT NOT NULL DEFAULT '',    -- weight C
    -- Display-only chips for the UI (status, kind, repo full_name, ...).
    meta              JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- Recency for ranking tie-breaks + cheap change detection against the
    -- source row (upserts are no-ops when this hasn't moved).
    source_updated_at TIMESTAMPTZ NOT NULL,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- The 'simple' config is deliberate: titles are identifiers (repo
    -- full names, branch refs, SHAs, UPPER_SNAKE secret names), not prose.
    -- No stemming or stopword surprises, lowercasing gives case-insensitive
    -- matching, and a hex SHA stays one prefix-matchable lexeme.
    search            tsvector GENERATED ALWAYS AS (
        setweight(to_tsvector('simple', left(title, 512)), 'A') ||
        setweight(to_tsvector('simple', left(subtitle, 1024)), 'B') ||
        setweight(to_tsvector('simple', left(body, 4096)), 'C')
    ) STORED,
    CONSTRAINT search_documents_entity_key UNIQUE (workspace_id, entity_type, entity_id)
);

-- Reconcile passes, per-category counts, and volume-cap trims all scan by
-- (workspace, type, recency). The GIN index follows in the next migration
-- (CONCURRENTLY, so it needs its own no-transaction file).
CREATE INDEX search_documents_ws_type_updated_idx
    ON search_documents (workspace_id, entity_type, source_updated_at DESC, id DESC);

-- Incremental-indexing watermarks, one row per workspace. audit_logs is
-- append-only, so activity documents advance by (created_at, id) cursor
-- instead of re-scanning the ledger.
CREATE TABLE search_index_state (
    workspace_id    UUID PRIMARY KEY REFERENCES workspaces (id) ON DELETE CASCADE,
    audit_cursor_at TIMESTAMPTZ,
    audit_cursor_id UUID,
    reconciled_at   TIMESTAMPTZ
);
