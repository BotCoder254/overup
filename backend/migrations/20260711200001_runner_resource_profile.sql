-- Resource profile for hosted (managed) runners. Server-side presets map the
-- value to container limits; self-hosted rows stay NULL.
ALTER TABLE runners
    ADD COLUMN resource_profile TEXT
        CHECK (resource_profile IN ('small', 'standard', 'large'));
