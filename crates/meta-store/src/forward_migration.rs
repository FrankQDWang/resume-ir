use rusqlite::{params, Connection, Transaction};
use sha2::{Digest, Sha256};

use crate::{schema_v29, schema_v30, MetaStoreError, Result};

const V29_TO_V30_NAME: &str = "metadata-forward-migration-history";

struct MigrationStep {
    from: u32,
    to: u32,
    name: &'static str,
    schema: &'static str,
    apply: fn(&Transaction<'_>) -> Result<()>,
    validate: fn(&Connection) -> Result<()>,
}

impl MigrationStep {
    fn checksum(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(b"resume-ir.metadata-forward-migration.v1");
        digest.update(self.from.to_be_bytes());
        digest.update(self.to.to_be_bytes());
        digest.update(self.name.as_bytes());
        digest.update(self.schema.as_bytes());
        format!("{:x}", digest.finalize())
    }
}

pub(super) fn apply_chain(connection: &mut Connection, from: u32, to: u32) -> Result<()> {
    if from > to {
        return Err(MetaStoreError::unsupported_store_schema());
    }
    let mut current = from;
    while current < to {
        let step = registry()
            .into_iter()
            .find(|step| step.from == current)
            .ok_or_else(MetaStoreError::unsupported_store_schema)?;
        if step.to != current + 1 || step.to > to {
            return Err(MetaStoreError::storage_invariant());
        }
        apply_step(connection, &step)?;
        current = step.to;
    }
    validate_chain(connection, schema_v29::VERSION, to)
}

pub(super) fn validate_chain(connection: &Connection, from: u32, to: u32) -> Result<()> {
    if from > to {
        return Err(MetaStoreError::unsupported_store_schema());
    }
    let mut current = from;
    while current < to {
        let step = registry()
            .into_iter()
            .find(|step| step.from == current)
            .ok_or_else(MetaStoreError::unsupported_store_schema)?;
        validate_history(connection, &step)?;
        (step.validate)(connection)?;
        current = step.to;
    }
    if current != to {
        return Err(MetaStoreError::storage_invariant());
    }
    let history_count = connection
        .query_row(
            "SELECT COUNT(*) FROM forward_migration_history
             WHERE to_version > ?1 AND to_version <= ?2",
            params![i64::from(from), i64::from(to)],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    if history_count != i64::from(to - from) {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

pub(super) fn apply_current_schema_from_v29(connection: &mut Connection) -> Result<()> {
    apply_chain(connection, schema_v29::VERSION, schema_v30::VERSION)
}

fn apply_step(connection: &mut Connection, step: &MigrationStep) -> Result<()> {
    let transaction = connection
        .transaction()
        .map_err(MetaStoreError::migration)?;
    (step.apply)(&transaction)?;
    transaction
        .execute(
            "INSERT INTO forward_migration_history (
                to_version, from_version, migration_name,
                migration_checksum, applied_at_seconds
             ) VALUES (?1, ?2, ?3, ?4, 0)",
            params![
                i64::from(step.to),
                i64::from(step.from),
                step.name,
                step.checksum(),
            ],
        )
        .map_err(MetaStoreError::migration)?;
    transaction
        .execute(
            "INSERT INTO schema_migrations (version, applied_at_seconds)
             VALUES (?1, 0)",
            params![i64::from(step.to)],
        )
        .map_err(MetaStoreError::migration)?;
    transaction.commit().map_err(MetaStoreError::migration)?;
    (step.validate)(connection)?;
    validate_history(connection, step)
}

fn validate_history(connection: &Connection, step: &MigrationStep) -> Result<()> {
    let row = connection
        .query_row(
            "SELECT from_version, migration_name, migration_checksum
             FROM forward_migration_history
             WHERE to_version = ?1",
            params![i64::from(step.to)],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(MetaStoreError::storage)?;
    if row != (i64::from(step.from), step.name.to_string(), step.checksum()) {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn apply_v29_to_v30(transaction: &Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(schema_v30::SCHEMA)
        .map_err(MetaStoreError::migration)
}

fn validate_v30(connection: &Connection) -> Result<()> {
    let table_count = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name = 'forward_migration_history'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    if table_count != 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn registry() -> [MigrationStep; 1] {
    [MigrationStep {
        from: schema_v29::VERSION,
        to: schema_v30::VERSION,
        name: V29_TO_V30_NAME,
        schema: schema_v30::SCHEMA,
        apply: apply_v29_to_v30,
        validate: validate_v30,
    }]
}
