use std::str::FromStr;

use core_domain::ContentDigest;
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use super::allowlist::AllowlistInventory;
use crate::{
    active_store_manifest::{
        validate_store_file_name, validate_store_id_digest, ActiveStoreManifest,
    },
    migration_v27::source_schema_version,
    schema_v27, schema_v28, MetaStoreError, Result,
};

const FENCE_TABLE: &str = "metadata_predecessor_write_fence";
const FENCE_SCHEMA: &str = "resume-ir.metadata-predecessor-write-fence.v1";
const TRIGGER_PREFIX: &str = "resume_ir_predecessor_write_fence_";
const WRITE_REJECTION: &str = "resume-ir metadata predecessor is retired";
const RETIRED_EMPTY_SCHEMA_SENTINEL: u32 = 2_147_483_647;

const CREATE_FENCE_TABLE: &str = "CREATE TABLE metadata_predecessor_write_fence (
    state_key TEXT PRIMARY KEY CHECK (state_key = 'default'),
    fence_schema TEXT NOT NULL,
    source_schema_version INTEGER NOT NULL,
    target_file_name TEXT NOT NULL,
    target_schema_version INTEGER NOT NULL,
    target_store_id_digest TEXT NOT NULL,
    document_count INTEGER NOT NULL CHECK (document_count >= 0),
    authorized_root_count INTEGER NOT NULL CHECK (authorized_root_count >= 0),
    inherited_visible_epoch INTEGER NOT NULL CHECK (inherited_visible_epoch >= 0),
    canonical_digest TEXT NOT NULL
) WITHOUT ROWID";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PredecessorWriteFence {
    pub(super) source_schema_version: u32,
    pub(super) target: ActiveStoreManifest,
    pub(super) inventory: AllowlistInventory,
}

pub(super) fn install_predecessor_write_fence(
    transaction: &Transaction<'_>,
    source_version: u32,
    target: &ActiveStoreManifest,
    inventory: &AllowlistInventory,
) -> Result<()> {
    if read_predecessor_write_fence(transaction)?.is_some() {
        return Err(MetaStoreError::storage_invariant());
    }
    validate_fence_values(source_version, target, inventory)?;
    if source_version == 0 {
        transaction
            .execute_batch(&format!(
                "CREATE TABLE schema_migrations (
                    version INTEGER PRIMARY KEY,
                    applied_at_seconds INTEGER NOT NULL
                 );
                 INSERT INTO schema_migrations (version, applied_at_seconds)
                 VALUES ({RETIRED_EMPTY_SCHEMA_SENTINEL}, 0);"
            ))
            .map_err(MetaStoreError::storage)?;
    }
    transaction
        .execute_batch(CREATE_FENCE_TABLE)
        .map_err(MetaStoreError::storage)?;
    transaction
        .execute(
            "INSERT INTO metadata_predecessor_write_fence (
                state_key, fence_schema, source_schema_version,
                target_file_name, target_schema_version, target_store_id_digest,
                document_count, authorized_root_count, inherited_visible_epoch,
                canonical_digest
             ) VALUES ('default', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                FENCE_SCHEMA,
                i64::from(source_version),
                target.file_name.as_str(),
                i64::from(target.schema_version),
                target.store_id_digest.as_str(),
                u64_to_i64(inventory.document_count)?,
                u64_to_i64(inventory.authorized_root_count)?,
                u64_to_i64(inventory.inherited_visible_epoch)?,
                inventory.canonical_digest.as_str(),
            ],
        )
        .map_err(MetaStoreError::storage)?;

    for (table_index, table_name) in user_table_names(transaction)?.iter().enumerate() {
        for operation in TriggerOperation::ALL {
            let statement = trigger_statement(table_index, table_name, operation);
            transaction
                .execute_batch(&statement)
                .map_err(MetaStoreError::storage)?;
        }
    }
    let _ = validate_predecessor_write_fence(transaction)?;
    Ok(())
}

pub(super) fn read_predecessor_write_fence(
    connection: &Connection,
) -> Result<Option<PredecessorWriteFence>> {
    let object_type = connection
        .query_row(
            "SELECT type FROM sqlite_schema WHERE name = ?1",
            [FENCE_TABLE],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(MetaStoreError::storage)?;
    match object_type.as_deref() {
        None => return Ok(None),
        Some("table") => {}
        Some(_) => return Err(MetaStoreError::storage_invariant()),
    }
    validate_predecessor_write_fence(connection).map(Some)
}

fn validate_predecessor_write_fence(connection: &Connection) -> Result<PredecessorWriteFence> {
    let row = connection
        .query_row(
            "SELECT fence_schema, source_schema_version, target_file_name,
                    target_schema_version, target_store_id_digest, document_count,
                    authorized_root_count, inherited_visible_epoch, canonical_digest
             FROM metadata_predecessor_write_fence WHERE state_key = 'default'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, String>(8)?,
                ))
            },
        )
        .map_err(MetaStoreError::storage)?;
    if row.0 != FENCE_SCHEMA {
        return Err(MetaStoreError::storage_invariant());
    }
    let fence = PredecessorWriteFence {
        source_schema_version: u32::try_from(row.1)
            .map_err(|_| MetaStoreError::storage_invariant())?,
        target: ActiveStoreManifest {
            file_name: row.2,
            schema_version: u32::try_from(row.3)
                .map_err(|_| MetaStoreError::storage_invariant())?,
            store_id_digest: row.4,
        },
        inventory: AllowlistInventory {
            document_count: i64_to_u64(row.5)?,
            authorized_root_count: i64_to_u64(row.6)?,
            inherited_visible_epoch: i64_to_u64(row.7)?,
            canonical_digest: ContentDigest::from_str(&row.8)
                .map_err(|_| MetaStoreError::storage_invariant())?,
        },
    };
    validate_fence_values(fence.source_schema_version, &fence.target, &fence.inventory)?;
    let observed_source_version = source_schema_version(connection)?;
    let expected_observed_version = if fence.source_schema_version == 0 {
        RETIRED_EMPTY_SCHEMA_SENTINEL
    } else {
        fence.source_schema_version
    };
    if observed_source_version != expected_observed_version {
        return Err(MetaStoreError::storage_invariant());
    }
    validate_trigger_coverage(connection)?;
    Ok(fence)
}

fn validate_fence_values(
    source_version: u32,
    target: &ActiveStoreManifest,
    _inventory: &AllowlistInventory,
) -> Result<()> {
    if !matches!(source_version, 0 | 26 | schema_v27::VERSION)
        || target.schema_version != schema_v28::VERSION
        || !target.file_name.starts_with("metadata-v28-")
    {
        return Err(MetaStoreError::storage_invariant());
    }
    validate_store_file_name(&target.file_name)?;
    validate_store_id_digest(&target.store_id_digest)
}

fn validate_trigger_coverage(connection: &Connection) -> Result<()> {
    let tables = user_table_names(connection)?;
    let expected_count = tables
        .len()
        .checked_mul(TriggerOperation::ALL.len())
        .ok_or_else(MetaStoreError::storage_invariant)?;
    let observed_count = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema
             WHERE type = 'trigger' AND name GLOB ?1",
            [format!("{TRIGGER_PREFIX}*")],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    if usize::try_from(observed_count).ok() != Some(expected_count) {
        return Err(MetaStoreError::storage_invariant());
    }
    for (table_index, table_name) in tables.iter().enumerate() {
        for operation in TriggerOperation::ALL {
            let trigger_name = trigger_name(table_index, operation);
            let expected_sql = trigger_sql(&trigger_name, table_name, operation);
            let observed = connection
                .query_row(
                    "SELECT tbl_name, sql FROM sqlite_schema
                     WHERE type = 'trigger' AND name = ?1",
                    [&trigger_name],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(MetaStoreError::storage)?;
            if observed.as_ref() != Some(&(table_name.clone(), expected_sql)) {
                return Err(MetaStoreError::storage_invariant());
            }
        }
    }
    Ok(())
}

fn user_table_names(connection: &Connection) -> Result<Vec<String>> {
    let mut statement = connection
        .prepare(
            "SELECT name FROM sqlite_schema
             WHERE type = 'table' AND name NOT GLOB 'sqlite_*'
             ORDER BY name",
        )
        .map_err(MetaStoreError::storage)?;
    let mut rows = statement.query([]).map_err(MetaStoreError::storage)?;
    let mut names = Vec::new();
    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        names.push(row.get::<_, String>(0).map_err(MetaStoreError::storage)?);
    }
    Ok(names)
}

#[derive(Clone, Copy)]
enum TriggerOperation {
    Insert,
    Update,
    Delete,
}

impl TriggerOperation {
    const ALL: [Self; 3] = [Self::Insert, Self::Update, Self::Delete];

    fn sql(self) -> &'static str {
        match self {
            Self::Insert => "INSERT",
            Self::Update => "UPDATE",
            Self::Delete => "DELETE",
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Self::Insert => "insert",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
}

fn trigger_statement(table_index: usize, table_name: &str, operation: TriggerOperation) -> String {
    let name = trigger_name(table_index, operation);
    format!("{};", trigger_sql(&name, table_name, operation))
}

fn trigger_name(table_index: usize, operation: TriggerOperation) -> String {
    format!("{TRIGGER_PREFIX}{table_index}_{}", operation.suffix())
}

fn trigger_sql(name: &str, table_name: &str, operation: TriggerOperation) -> String {
    format!(
        "CREATE TRIGGER \"{}\" BEFORE {} ON \"{}\" BEGIN SELECT RAISE(ABORT, '{}'); END",
        quote_identifier(name),
        operation.sql(),
        quote_identifier(table_name),
        WRITE_REJECTION,
    )
}

fn quote_identifier(value: &str) -> String {
    value.replace('"', "\"\"")
}

fn u64_to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| MetaStoreError::storage_invariant())
}

fn i64_to_u64(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| MetaStoreError::storage_invariant())
}
