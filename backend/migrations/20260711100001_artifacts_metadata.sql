-- Richer artifact metadata. `kind` is classified SERVER-side from the name
-- at insert time (services/artifact_kind.rs — keep the CASE below in sync);
-- the manifest fields come from the runner (validated + capped) and stay
-- NULL for non-archives and for uploads from older runners.
ALTER TABLE artifacts
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'other'
        CHECK (kind IN ('package','report','docs','archive','binary','image','log','other')),
    ADD COLUMN uncompressed_bytes BIGINT,
    ADD COLUMN file_count INTEGER,
    ADD COLUMN entries JSONB;

-- Best-effort backfill for pre-existing rows.
UPDATE artifacts SET kind = CASE
    WHEN name ~* '\.(crate|whl|gem|jar|deb|rpm|nupkg|apk)$'  THEN 'package'
    WHEN name ~* '\.(zip|tar|tgz|tar\.gz|tar\.bz2|7z)$'      THEN 'archive'
    WHEN name ~* '\.(png|jpe?g|gif|svg|webp|ico)$'           THEN 'image'
    WHEN name ~* '\.(html?|pdf|md)$'                         THEN 'docs'
    WHEN name ~* '\.(xml|sarif|lcov|junit)$'                 THEN 'report'
    WHEN name ~* '\.(log|txt)$'                              THEN 'log'
    WHEN name ~* '\.(exe|dll|so|dylib|wasm|bin)$'            THEN 'binary'
    ELSE 'other'
END;

-- Per-kind retention policies, GitHub-style. kind = 'default' is the
-- workspace-wide default overriding the global ARTIFACT_RETENTION_DAYS env;
-- per-kind rows override 'default'. Resolution at upload time:
-- kind row -> 'default' row -> env. expires_at stays immutable per artifact.
CREATE TABLE artifact_retention_policies (
    workspace_id   UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    kind           TEXT NOT NULL
        CHECK (kind IN ('default','package','report','docs','archive','binary','image','log','other')),
    retention_days INTEGER NOT NULL CHECK (retention_days BETWEEN 1 AND 400),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, kind)
);
