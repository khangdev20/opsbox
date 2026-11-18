-- Prompt 16k: optional human-facing name on panel accounts.
-- Empty string is not stored (application trims to NULL). Length is enforced in
-- application code (characters, not bytes); the CHECK is a coarse byte cap.

ALTER TABLE users ADD COLUMN display_name TEXT CHECK (
    display_name IS NULL OR (length(display_name) >= 1 AND length(display_name) <= 64)
);
