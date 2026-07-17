-- Toolchains a user has installed (pulled + warmed) from the UI, so the set
-- to prewarm is no longer env-only (RUNNER_PREPULL_IMAGES). The runner Docker
-- daemon is a single deployment-global resource, so this table is global (no
-- workspace_id) — an install is actioned/audited under the calling workspace's
-- content.write, but the pulled image is shared. `toolchain_key` is the catalog
-- alias (e.g. 'rust'); `image` is its resolved -latest reference. `error` holds
-- a static category on a failed pull, never raw daemon text.
CREATE TABLE installed_toolchain_images (
    toolchain_key TEXT PRIMARY KEY,
    image         TEXT NOT NULL,
    status        TEXT NOT NULL DEFAULT 'pending'
                      CHECK (status IN ('pending', 'installed', 'failed')),
    error         TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
