-- Per-object storage-backend markers. Presigned URLs, HeadObject and
-- DeleteObject are host-specific, so once MinIO (primary) and R2 (fallback)
-- coexist every stored object must remember which store holds it. Existing
-- rows were all written when R2 was the only store, hence the defaults; the
-- nullable columns read as 'r2' when NULL (legacy rows).

ALTER TABLE artifacts
    ADD COLUMN storage_backend TEXT NOT NULL DEFAULT 'r2'
        CONSTRAINT artifacts_storage_backend_check
        CHECK (storage_backend IN ('minio', 'r2'));

-- Set alongside logs_archived_at going forward; NULL = legacy archive in R2.
ALTER TABLE pipeline_jobs
    ADD COLUMN logs_archive_backend TEXT
        CONSTRAINT pipeline_jobs_logs_archive_backend_check
        CHECK (logs_archive_backend IN ('minio', 'r2'));

-- Set alongside logo_key going forward; NULL = legacy logo in R2.
ALTER TABLE workspaces
    ADD COLUMN logo_storage_backend TEXT
        CONSTRAINT workspaces_logo_storage_backend_check
        CHECK (logo_storage_backend IN ('minio', 'r2'));
