-- Hosted-runner provisioning happens in a background task (the image pull
-- can take minutes); this column is how a failed attempt is surfaced to the
-- polling wizard. Values are static categories only (image_pull_failed,
-- container_create_failed, container_start_failed, provision_timeout) —
-- never upstream Docker text. NULL = not failed (provisioning or healthy).
ALTER TABLE runners ADD COLUMN provision_error TEXT;
