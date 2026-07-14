-- Manual dispatch inputs (workflow_dispatch-style): the validated, effective
-- input map snapshotted at pipeline creation so reruns reproduce the exact
-- run. Inputs are non-secret by definition; values are capped at dispatch
-- (1 KB each, 16 KB total, <= 25 keys). NULL for push/legacy pipelines.
ALTER TABLE pipelines
    ADD COLUMN trigger_inputs JSONB;
