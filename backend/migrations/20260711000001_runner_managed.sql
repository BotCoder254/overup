-- Hosted runners: provisioned by the control plane as Docker containers on
-- the server host. `managed` marks them; `container_id` records the Docker
-- container so revoke/janitor can deprovision it.
ALTER TABLE runners
    ADD COLUMN managed BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN container_id TEXT;
