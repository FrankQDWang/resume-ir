pub(super) const VERSION: u32 = 37;

/// Receipt snapshots migrated from an earlier schema have not yet attested the
/// current checkpoint invariant, even when their contents happen to match it.
pub(super) const LEGACY_OR_UNATTESTED: i64 = 1;

/// The receipt snapshot satisfies the v2 checkpoint invariant at the moment
/// this attestation is written. It does not fence later ownership changes.
pub(super) const SNAPSHOT_INVARIANT_V2: i64 = 2;

/// Add one fail-closed checkpoint attestation to the authoritative receipt.
///
/// SQLite requires a non-null default when adding this column. The default is
/// deliberately the legacy value; the sole current receipt writer must opt in
/// to v2 explicitly.
pub(super) const SCHEMA: &str = r#"
ALTER TABLE source_root_deletion
ADD COLUMN checkpoint_protocol_version INTEGER NOT NULL DEFAULT 1 CHECK (
    checkpoint_protocol_version IN (1, 2)
);
"#;
