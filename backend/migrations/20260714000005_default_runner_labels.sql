-- A runner with an empty label set can never satisfy any labeled runs-on:
-- it sits idle while jobs queue forever as no_matching_runner. GitHub
-- sidesteps the whole failure class by giving every runner an unremovable
-- default label set (self-hosted + OS + arch); registration and the hello
-- label sync now guarantee the same, and this backfills rows created
-- before that guarantee existed. The set matches the hosted auto-provision
-- default (jobs execute in containers chosen from runs-on, so the labels
-- describe the execution environment, not the host).
UPDATE runners
SET labels = ARRAY['self-hosted', 'linux', 'x64', 'ubuntu-latest']
WHERE labels = '{}' AND revoked_at IS NULL;
