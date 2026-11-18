-- Prompt 16f: Per-user notification read and snooze state for attention items.

CREATE TABLE attention_state (
    user_id         TEXT NOT NULL,
    item_id         TEXT NOT NULL,
    read_since      INTEGER,
    read_severity   INTEGER,
    read_at         INTEGER,
    snooze_until    INTEGER,
    snooze_severity INTEGER,
    updated_at      INTEGER NOT NULL,
    PRIMARY KEY (user_id, item_id)
) STRICT;
CREATE INDEX attention_state_user ON attention_state (user_id);
