pub(super) const SCHEMA: &str = r#"
CREATE TABLE privacy_maintenance_state (
    state_key TEXT PRIMARY KEY NOT NULL CHECK (state_key = 'default'),
    compaction_pending INTEGER NOT NULL CHECK (compaction_pending IN (0, 1))
);

INSERT INTO privacy_maintenance_state (state_key, compaction_pending)
VALUES ('default', 0);

CREATE TRIGGER privacy_maintenance_state_singleton_delete
BEFORE DELETE ON privacy_maintenance_state
BEGIN
    SELECT RAISE(ABORT, 'privacy maintenance state is required');
END;
"#;
