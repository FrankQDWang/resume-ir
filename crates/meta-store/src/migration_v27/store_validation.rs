use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use super::owner_regular_file_exists;
use crate::{
    apply_sqlcipher_key, schema_v27, verify_sqlcipher_key, MetaStore, MetaStoreError, Result,
};

pub(super) fn validate_staging_store(store: &MetaStore, store_id_digest: &str) -> Result<()> {
    let connection = store.connection.borrow();
    validate_database_integrity(&connection)?;
    if source_schema_version(&connection)? != schema_v27::VERSION
        || store_identity(&connection)? != store_id_digest
    {
        return Err(MetaStoreError::storage_invariant());
    }
    let foreign_key_failures = connection
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(MetaStoreError::storage)?;
    let derived_rows = connection
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM source_revision)
                + (SELECT COUNT(*) FROM resume_version)
                + (SELECT COUNT(*) FROM entity_mention)
                + (SELECT COUNT(*) FROM source_revision_triage)
                + (SELECT COUNT(*) FROM resume_version_classification)
                + (SELECT COUNT(*) FROM ocr_job_spec)
                + (SELECT COUNT(*) FROM source_revision_triage_reason)
                + (SELECT COUNT(*) FROM ocr_job_discard)
                + (SELECT COUNT(*) FROM active_search_projection)
                + (SELECT COUNT(*) FROM search_publication_journal)
                + (SELECT COUNT(*) FROM search_publication_commit_guard)
                + (SELECT COUNT(*) FROM resume_version_classification_reason)
                + (SELECT COUNT(*) FROM resume_version_candidate)
                + (SELECT COUNT(*) FROM resume_version_seal)
                + (SELECT COUNT(*) FROM candidate)
                + (SELECT COUNT(*) FROM candidate_contact_conflict)
                + (SELECT COUNT(*) FROM embedding_job_spec)
                + (SELECT COUNT(*) FROM ingest_job)
                + (SELECT COUNT(*) FROM import_task)
                + (SELECT COUNT(*) FROM import_scan_scope)
                + (SELECT COUNT(*) FROM import_scan_error)
                + (SELECT COUNT(*) FROM import_task_cancellation)
                + (SELECT COUNT(*) FROM ocr_page_cache)
                + (SELECT COUNT(*) FROM query_observation)
                + (SELECT COUNT(*) FROM worker_task_control)",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    let projection_state = connection
        .query_row(
            "SELECT service_state, generation, repair_reason FROM search_projection_state
             WHERE state_key = 'default'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .map_err(MetaStoreError::storage)?;
    let privacy_compaction_pending = connection
        .query_row(
            "SELECT compaction_pending FROM privacy_maintenance_state
             WHERE state_key = 'default'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    if foreign_key_failures != 0
        || derived_rows != 0
        || privacy_compaction_pending != 0
        || projection_state
            != (
                "repairing".to_string(),
                None,
                Some("migration_rebuild".to_string()),
            )
    {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

pub(super) fn validate_active_store(path: &Path, key: &[u8], store_id_digest: &str) -> Result<()> {
    if !owner_regular_file_exists(path)? {
        return Err(MetaStoreError::storage_invariant());
    }
    let connection = open_encrypted_connection(path, key)?;
    validate_database_integrity(&connection)?;
    if source_schema_version(&connection)? != schema_v27::VERSION
        || store_identity(&connection)? != store_id_digest
    {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn validate_database_integrity(connection: &Connection) -> Result<()> {
    let integrity = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .map_err(MetaStoreError::storage)?;
    let foreign_key_failures = connection
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(MetaStoreError::storage)?;
    if integrity != "ok" || foreign_key_failures != 0 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

pub(super) fn open_encrypted_connection(path: &Path, key: &[u8]) -> Result<Connection> {
    let connection = Connection::open(path).map_err(MetaStoreError::storage)?;
    apply_sqlcipher_key(&connection, key)?;
    verify_sqlcipher_key(&connection)?;
    Ok(connection)
}

pub(super) fn source_schema_version(connection: &Connection) -> Result<u32> {
    let table_exists = connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'
             )",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?
        == 1;
    if !table_exists {
        return Ok(0);
    }
    let version = connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    u32::try_from(version).map_err(|_| MetaStoreError::invalid_value("schema_migrations.version"))
}

pub(super) fn store_identity(connection: &Connection) -> Result<String> {
    connection
        .query_row(
            "SELECT store_id_digest FROM metadata_store_identity WHERE state_key = 'default'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .ok_or_else(|| MetaStoreError::invalid_value("metadata_store_identity"))
}
