pub(super) const VERSION: u32 = 38;

pub(super) const MAX_ROOT_REVOCATION_EPOCH: i64 = 9_007_199_254_740_991;

/// Add a bounded root revocation generation and bind every scan snapshot to
/// the generation captured by its existing writer.
///
/// SQLite requires a non-null default when adding a column. Both defaults are
/// legacy backfill values; current writers must always provide the captured
/// epoch explicitly.
pub(super) const SCHEMA: &str = r#"
ALTER TABLE source_root
ADD COLUMN revocation_epoch INTEGER NOT NULL DEFAULT 0 CHECK (
    typeof(revocation_epoch) = 'integer'
    AND revocation_epoch BETWEEN 0 AND 9007199254740991
);

ALTER TABLE scan_snapshot
ADD COLUMN root_revocation_epoch INTEGER NOT NULL DEFAULT 0 CHECK (
    typeof(root_revocation_epoch) = 'integer'
    AND root_revocation_epoch BETWEEN 0 AND 9007199254740991
);

CREATE TRIGGER scan_snapshot_root_revocation_epoch_immutable
BEFORE UPDATE OF root_revocation_epoch ON scan_snapshot
WHEN NEW.root_revocation_epoch <> OLD.root_revocation_epoch
BEGIN
    SELECT RAISE(ABORT, 'immutable scan root revocation epoch');
END;
"#;
