use std::path::Path;

use rusqlite::Connection;

use super::allowlist::{validate_allowlist_inventory, AllowlistInventory};
use crate::{
    active_store_manifest::owner_regular_file_exists,
    migration_v27::{
        open_encrypted_connection, open_encrypted_read_connection, source_schema_version,
        store_identity,
    },
    schema_v28, MetaStoreError, Result,
};

pub(super) fn validate_active_v28_store(
    path: &Path,
    key: &[u8],
    store_id_digest: &str,
) -> Result<()> {
    if !owner_regular_file_exists(path)? {
        return Err(MetaStoreError::storage_invariant());
    }
    let connection = open_encrypted_read_connection(path, key)?;
    validate_active_v28_connection(&connection, store_id_digest)
}

pub(super) fn validate_active_v28_connection(
    connection: &Connection,
    store_id_digest: &str,
) -> Result<()> {
    let journal_mode = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
        .map_err(MetaStoreError::storage)?;
    if !journal_mode.eq_ignore_ascii_case("delete") {
        return Err(MetaStoreError::storage_invariant());
    }
    validate_database_integrity(connection)?;
    validate_identity(connection, store_id_digest)
}

pub(super) fn validate_staging_v28_store(
    path: &Path,
    key: &[u8],
    store_id_digest: &str,
    inventory: &AllowlistInventory,
) -> Result<()> {
    let connection = open_encrypted_connection(path, key)?;
    validate_database_integrity(&connection)?;
    validate_identity(&connection, store_id_digest)?;
    validate_allowlist_inventory(&connection, inventory)?;

    let invalid_documents = connection
        .query_row(
            "SELECT COUNT(*) FROM document
             WHERE content_hash IS NOT NULL OR text_hash IS NOT NULL
                OR (is_deleted = 0 AND status <> 'discovered')
                OR (is_deleted = 1 AND status <> 'deleted')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    let derived_rows = connection
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM source_revision)
              + (SELECT COUNT(*) FROM source_revision_triage)
              + (SELECT COUNT(*) FROM source_revision_triage_reason)
              + (SELECT COUNT(*) FROM resume_version)
              + (SELECT COUNT(*) FROM resume_version_classification)
              + (SELECT COUNT(*) FROM resume_version_classification_reason)
              + (SELECT COUNT(*) FROM resume_version_candidate)
              + (SELECT COUNT(*) FROM resume_version_seal)
              + (SELECT COUNT(*) FROM entity_mention)
              + (SELECT COUNT(*) FROM candidate)
              + (SELECT COUNT(*) FROM candidate_contact_conflict)
              + (SELECT COUNT(*) FROM ingest_job)
              + (SELECT COUNT(*) FROM ocr_job_spec)
              + (SELECT COUNT(*) FROM ocr_job_discard)
              + (SELECT COUNT(*) FROM embedding_job_spec)
              + (SELECT COUNT(*) FROM ocr_page_cache)
              + (SELECT COUNT(*) FROM worker_task_control)
              + (SELECT COUNT(*) FROM import_task)
              + (SELECT COUNT(*) FROM import_scan_scope)
              + (SELECT COUNT(*) FROM import_scan_error)
              + (SELECT COUNT(*) FROM import_task_cancellation)
              + (SELECT COUNT(*) FROM import_processing_contract)
              + (SELECT COUNT(*) FROM import_task_contract_binding)
              + (SELECT COUNT(*) FROM migration_rebuild_full_corpus_task)
              + (SELECT COUNT(*) FROM import_task_source_disposition)
              + (SELECT COUNT(*) FROM import_task_completion)
              + (SELECT COUNT(*) FROM migration_rebuild_publication_attempt)
              + (SELECT COUNT(*) FROM active_search_projection)
              + (SELECT COUNT(*) FROM search_publication_journal)
              + (SELECT COUNT(*) FROM search_publication_commit_guard)
              + (SELECT COUNT(*) FROM query_observation)",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    let staging_authority_rows = connection
        .query_row(
            "SELECT COUNT(*) FROM metadata_cow_staging_authority",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    let projection_state = connection
        .query_row(
            "SELECT service_state, generation, visible_epoch, repair_reason,
                    updated_at_seconds
             FROM search_projection_state WHERE state_key = 'default'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .map_err(MetaStoreError::storage)?;
    let rebuild_contract_state = connection
        .query_row(
            "SELECT active_contract_id, updated_at_seconds
             FROM migration_rebuild_contract_state WHERE state_key = 'default'",
            [],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?)),
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
    if invalid_documents != 0
        || derived_rows != 0
        || staging_authority_rows != 0
        || privacy_compaction_pending != 0
        || projection_state
            != (
                "repairing".to_string(),
                None,
                i64::try_from(inventory.inherited_visible_epoch)
                    .map_err(|_| MetaStoreError::storage_invariant())?,
                Some("migration_rebuild".to_string()),
                0,
            )
        || rebuild_contract_state != (None, 0)
    {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn validate_identity(connection: &Connection, store_id_digest: &str) -> Result<()> {
    let migration_count = connection
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(MetaStoreError::storage)?;
    if source_schema_version(connection)? != schema_v28::VERSION
        || migration_count != i64::from(schema_v28::VERSION)
        || store_identity(connection)? != store_id_digest
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
