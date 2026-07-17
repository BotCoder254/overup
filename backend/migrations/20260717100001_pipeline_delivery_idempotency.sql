-- Webhook-triggered pipelines record the delivery that created them, so a
-- retried delivery (crash or commit failure between side effects and the
-- completion transaction) can never create the same pipeline twice.
-- NULL for manual dispatch/rerun pipelines.
ALTER TABLE pipelines ADD COLUMN webhook_delivery_id TEXT;

-- One pipeline per (delivery, workflow). Partial: manual pipelines are
-- unconstrained. workflow_id is already workspace-scoped, so one delivery
-- fanning out to multiple workspaces still creates one pipeline per
-- workspace without conflicting.
CREATE UNIQUE INDEX pipelines_delivery_workflow_uq
    ON pipelines (webhook_delivery_id, workflow_id)
    WHERE webhook_delivery_id IS NOT NULL;
