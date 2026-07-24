pub(super) const VERSION: u32 = 30;

/// v30 introduces the durable history for the continuous forward-migration
/// registry. The table records only completed adjacent migrations; the
/// external receipt owns filesystem publication and crash recovery.
pub(super) const SCHEMA: &str = r#"
CREATE TABLE forward_migration_history (
    to_version INTEGER PRIMARY KEY CHECK (to_version > 29),
    from_version INTEGER NOT NULL CHECK (from_version >= 29),
    migration_name TEXT NOT NULL CHECK (
        length(migration_name) BETWEEN 1 AND 96
        AND instr(migration_name, char(0)) = 0
        AND instr(migration_name, char(10)) = 0
        AND instr(migration_name, char(13)) = 0
    ),
    migration_checksum TEXT NOT NULL CHECK (
        length(migration_checksum) = 64
        AND migration_checksum NOT GLOB '*[^0-9a-f]*'
    ),
    applied_at_seconds INTEGER NOT NULL CHECK (applied_at_seconds >= 0),
    CHECK (to_version = from_version + 1)
);
"#;
