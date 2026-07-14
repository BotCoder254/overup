-- Profile settings: users may override the display name / email mirrored
-- from GitHub. Once a field is customized, upsert_by_github stops refreshing
-- it on login (avatar always mirrors GitHub — it is the source of truth).
ALTER TABLE users
    ADD COLUMN display_name_customized BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN email_customized BOOLEAN NOT NULL DEFAULT FALSE;
